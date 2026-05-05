//! Anthropic Messages API クライアント (Opus 4.7 / Haiku 4.5)
//!
//! - `brief()`: Haiku 4.5 で JSON 骨子 (title/slug/outline/20タグ/4画像プロンプト)
//! - `write()`: Opus 4.7 で SEO重視 note 記事 (導入150-250字 / 本論1000-1500字 / 結論200-400字)

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::json;

use super::client::AiClient;
use super::{strip_code_fence, ArticleBrief, ArticleDraft, ResearchResult};
use crate::scoring::SelectedTrend;

const ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";

pub struct AnthropicClient<'a> {
    http: &'a reqwest::Client,
    api_key: String,
    opus_model: String,
    haiku_model: String,
    max_tokens: u32,
}

impl AiClient for AnthropicClient<'_> {
    fn label(&self) -> &'static str {
        "anthropic"
    }

    fn build_request(&self, body: &serde_json::Value) -> reqwest::RequestBuilder {
        self.http
            .post(ENDPOINT)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .json(body)
    }
}

impl<'a> AnthropicClient<'a> {
    pub fn new(
        http: &'a reqwest::Client,
        api_key: &str,
        opus_model: &str,
        haiku_model: &str,
        max_tokens: u32,
    ) -> Self {
        Self {
            http,
            api_key: api_key.to_string(),
            opus_model: opus_model.to_string(),
            haiku_model: haiku_model.to_string(),
            max_tokens,
        }
    }

    /// Haiku 4.5: トレンド + リサーチ → 記事ブリーフ(JSON)
    /// - 20個のSEOハッシュタグ
    /// - 4枚の画像生成プロンプト (英語、日本人・フォトリアル)
    pub async fn brief(&self, trend: &SelectedTrend, research: &ResearchResult) -> Result<ArticleBrief> {
        let system = "あなたは note 記事のプランナー兼 SEO アナリストです。\
読者は日本の実務家・生活者・投資家。Google 検索1位と note スキ最大化の両方を狙います。\
出力は JSON のみ。前後に説明文は一切入れないこと。";

        let user = format!(
            "【トピック】\n{}\n\n\
【リサーチ要約】\n{}\n\n\
【主要事実】\n- {}\n\n\
【反対意見】\n- {}\n\n\
以下のスキーマで JSON を返してください (```で囲まない):\n\
{{\n  \
\"title\": \"35-45字、検索上位を狙う日本語タイトル。数字や感情語を入れる\",\n  \
\"slug\": \"半角英数ハイフンのスラッグ、30字以内\",\n  \
\"category\": \"AI/テクノロジー/ビジネス/ライフスタイル/日本情勢/エンタメ/ツール\",\n  \
\"outline\": [\"章1の小見出し (感情語+具体名詞)\", \"章2\", \"章3\", \"章4\", \"章5\"],\n  \
\"tags\": [\"ハッシュタグ1\", ...20個。検索1位を狙う note ハッシュタグ。スペース不可。#は付けない],\n  \
\"hook\": \"冒頭1-2文で読者の悩みを代弁するフック\",\n  \
\"image_prompts\": [\n    \
\"[HERO] English prompt for cover image. Japanese setting and Japanese person. Photorealistic, subsurface scattering, lens flare, ray-traced lighting, portrait, 16:9, cinematic. Include visible but elegant small watermark text 'cityriver.sayasaya.workers.dev' as tiny bottom-right badge. Visually convey the article theme: ... (書き込んで)\",\n    \
\"[INLINE 1] English prompt for image illustrating 導入 scene. Japanese person, photorealistic, no text, 16:9, ...\",\n    \
\"[INLINE 2] English prompt for image illustrating 本論 middle. ...\",\n    \
\"[INLINE 3] English prompt for image illustrating 結論 hope/future. ...\"\n  ]\n\
}}\n\n\
注意: image_prompts は英語で、'Japanese setting', 'Japanese person/people', 'photorealistic', \
'subsurface scattering', 'lens flare', 'ray-traced lighting', 'portrait', '16:9' を必ず含めてください。\
記事内容と連動した具体的な場面描写 (場所、表情、しぐさ、服装、髪型、色彩) を入れます。",
            trend.item.title,
            research.summary,
            research.key_facts.join("\n- "),
            research.counterpoints.join("\n- "),
        );

        let text = self.call(&self.haiku_model, system, &user, 3000).await?;
        let cleaned = strip_code_fence(&text);
        let brief: ArticleBrief = serde_json::from_str(&cleaned).with_context(|| {
            format!("brief JSON parse failed. head: {}", cleaned.chars().take(300).collect::<String>())
        })?;
        Ok(brief)
    }

    /// Opus 4.7: ブリーフ + リサーチ → note 記事本文 (markdown)
    /// スタイル指示書準拠、画像プレースホルダ {{IMAGE_HEADER}} / {{IMAGE_1}} / {{IMAGE_2}} / {{IMAGE_3}} 挿入
    pub async fn write(
        &self,
        brief: &ArticleBrief,
        research: &ResearchResult,
        trend: &SelectedTrend,
        _target_chars: usize,
    ) -> Result<ArticleDraft> {
        let system = r#"あなたは note 人気クリエイター兼 SEO ライターです。以下のスタイル指示に厳密に従い、
日本語で検索 1位を取れる note 記事を markdown で執筆してください。出力は markdown 本文のみ。

【文章構造 (字数厳守)】
- 導入 (150-250字): 読者の悩みを具体的に代弁し、一人称の体験 1文を入れる。情景描写・擬態語を用いる
- 本論 (1,000-1,500字, 複数章): 各章の小見出しは「感情語＋具体名詞」。
  章ごとに「一次体験 / 事実 / 一般見解 / データ / 反論→再説明」の順序をランダムに入れ替える (全て入れなくて可)
  数字を出す時は「取得方法→計算式→結果」をセットで
  未来志向の提案と感情的呼びかけを織り交ぜる
- 結論 (200-400字): 未来志向で締める

【文体 — 柔らかく優しい、女性的な語り口】
- 全体のトーン: ふんわり優しい、共感的、読者に寄り添う女友達のような話し方。
  ただし甘すぎず、きちんと情報を伝える。中立的で偏りのない視点を保つ。
- 一文 5-120字、意図的にばらつかせる
- 語尾は柔らかい日本語を循環: 「〜ですね」「〜かもしれません」「〜のかな、と思います」
  「〜でしょうか」「〜みたいです」「〜だそうです」「〜らしいですよ」
  断定より推量・共感寄り。同じ語尾の連続は 2 回まで
- 接続詞・副詞: 「ちなみに」「そういえば」「個人的には」「やっぱり」「なんとなく」
  「ふとした瞬間」「ちょっと」「もしかしたら」「実は」「じつは」
  堅い接続詞 (従って・したがって・故に) は避ける
- 擬音語・比喩・対話風挿入句を各段落に 1つまで (「ほっ」「じんわり」「ふわっ」
  「すーっと」「ちょこん」「わくわく」等、やわらかい擬音優先)
- 固有名詞・日時・場所・登場人物を必ず入れる
- 絵文字を各章 2-3 個、やわらかい絵文字中心 (✨ 🌸 ☕ 🌿 💭 🫧 🕊 💫 🍀 🌷 等)
- 読者への疑問文を 2-3 箇所のみ、「〜じゃないですか？」「〜したこと、ありませんか？」
  などの共感を誘う形
- 失敗談 + 教訓を最少 2箇所、「わたしも最初は〜でした」のように自分を下げて共感を作る
- 英文の引用は必ず分かりやすい日本語に和訳する
- 執筆者の性別は絶対に明記しない (ただし語り口は自然に柔らかく)
- キーワード密度は 1.5% 以下

【中立性の徹底】
- 賛否が分かれる話題では必ず両論を併記。どちらかを断罪する表現は禁止
- 「〜すべき」「〜すべきではない」を控え、「〜という選択肢もありますね」に寄せる
- 政治・宗教・特定個人への攻撃的表現は禁止
- 投資/医療/法律アドバイスに見える断定は避ける (「参考程度に」「判断は読者自身で」)

【n-gram・AI 検出対策】
- PREP / SDS をあえて固定せず、章ごとに順序を変える
- 同義語を積極的に活用
- 文末・接続詞・語尾の連続を避ける

【表】
- note は表が崩れるので、どうしても必要なら箇条書きまたは擬似ブロック (---) で代替

【画像挿入位置 (必ずこの通りのプレースホルダを本文に配置)】
- タイトル行 (# ...) の直後: {{IMAGE_HEADER}}
- 導入セクションの末尾: {{IMAGE_1}}
- 本論の中間の章間: {{IMAGE_2}}
- 結論セクションの直前: {{IMAGE_3}}
※ プレースホルダは単独行で、前後に空行を入れる

【出典】
本文末尾に「## 参考リンク」セクションを作り、参考URLをリスト化

【固定フッター (本文末尾に編集せずそのまま追加)】
下の「---」含め、以下をそのまま貼り付ける:

---

皆様の意見はどうでしょうか？
良かったらコメントで教えて下さい。
フォロー＆スキもお願いします♪

この記事への感想やご質問、お仕事のご依頼など、
お気軽にメッセージをお送りください♪
📩メッセージはこちらから
https://note.com/alvis8039/message

(↑ このブロックの後、最後の行にハッシュタグを 20個スペース区切り。先頭は # を付ける)
"#;

        let user = format!(
            "【記事メタ】\n\
タイトル: {}\n\
カテゴリ: {}\n\
フック: {}\n\
20ハッシュタグ: {}\n\n\
【構成 (この順で見出しを立てる)】\n{}\n\n\
【リサーチ要約】\n{}\n\n\
【主要事実 (引用可)】\n- {}\n\n\
【反対意見・留意点】\n- {}\n\n\
【参考URL (本文末の「## 参考リンク」に載せる)】\n- {}\n{}\n\n\
上記を元に、スタイル指示に厳密従って markdown 記事を執筆してください。\
冒頭は `# {{title}}` から始め、直後に `{{{{IMAGE_HEADER}}}}` プレースホルダを単独行で置き、\
導入末尾に `{{{{IMAGE_1}}}}`、本論中間に `{{{{IMAGE_2}}}}`、結論直前に `{{{{IMAGE_3}}}}` を置いてください。\
最後に固定フッター + 20ハッシュタグ (`#タグ1 #タグ2 ... #タグ20`) を 1行で配置。\
参考動画や商品URLが研究結果にあれば本文中に自然に埋め込んでください。",
            brief.title,
            brief.category,
            brief.hook,
            brief.tags.iter().map(|t| format!("#{}", t)).collect::<Vec<_>>().join(" "),
            brief.outline.iter().enumerate()
                .map(|(i, o)| format!("{}. {}", i + 1, o))
                .collect::<Vec<_>>()
                .join("\n"),
            research.summary,
            research.key_facts.join("\n- "),
            research.counterpoints.join("\n- "),
            trend.item.url.clone().unwrap_or_else(|| "(出典URLなし)".into()),
            research.citations.iter().map(|c| format!("- {}", c)).collect::<Vec<_>>().join("\n"),
        );

        let body_md = self.call(&self.opus_model, system, &user, self.max_tokens).await?;
        let body_md = body_md
            .trim()
            .trim_start_matches("```markdown")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
            .to_string();
        let char_count = body_md.chars().count();
        Ok(ArticleDraft {
            title: brief.title.clone(),
            body_markdown: body_md,
            char_count,
        })
    }

    async fn call(&self, model: &str, system: &str, user: &str, max_tokens: u32) -> Result<String> {
        let body = json!({
            "model": model,
            "max_tokens": max_tokens,
            "system": system,
            "messages": [
                {"role": "user", "content": user}
            ]
        });

        #[derive(Deserialize)]
        struct Resp { content: Vec<Block> }
        #[derive(Deserialize)]
        struct Block { #[serde(rename = "type")] kind: String, text: Option<String> }

        // AF2: AiClient::build_request + client::send_json 経由で
        //      M2 retry/backoff + status check + JSON parse + error 整形を統合実行
        let parsed: Resp = super::client::send_json(self, &body).await?;
        let text = parsed.content.into_iter()
            .filter(|b| b.kind == "text")
            .filter_map(|b| b.text)
            .collect::<Vec<_>>()
            .join("");
        if text.is_empty() {
            return Err(anyhow!("empty response from {}", model));
        }
        Ok(text)
    }
}
