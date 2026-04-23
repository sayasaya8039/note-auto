//! Anthropic Messages API クライアント (Opus 4.7 / Haiku 4.5)
//!
//! - `brief()`: Haiku 4.5 でトレンド→タイトル・スラッグ・アウトラインを JSON 生成
//! - `write()`: Opus 4.7 でブリーフ+リサーチ→ note向けmarkdown本文を執筆

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::json;

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
    pub async fn brief(&self, trend: &SelectedTrend, research: &ResearchResult) -> Result<ArticleBrief> {
        let system = "あなたはnote記事のプランナーです。読者は日本の実務家・エンジニア・投資家。\
与えられたトピックとリサーチ結果から、クリックされる記事の骨子を JSON のみで返してください。\
前後に説明文を入れてはいけません。";

        let user = format!(
            "【トピック】\n{}\n\n\
【リサーチ要約】\n{}\n\n\
【主要事実】\n{}\n\n\
【反対意見】\n{}\n\n\
以下のスキーマで JSON を返してください (```で囲まない):\n\
{{\n  \"title\": \"note記事のタイトル (35文字前後・検索流入を意識)\",\n  \
\"slug\": \"半角英数ハイフンのスラッグ\",\n  \
\"category\": \"AI/テクノロジー/ビジネス/ライフスタイル/日本情勢/トレンド/ツール のいずれか\",\n  \
\"outline\": [\"見出し1\", \"見出し2\", \"見出し3\", \"見出し4\", \"見出し5\"],\n  \
\"tags\": [\"タグ1\", \"タグ2\", \"タグ3\"],\n  \
\"hook\": \"冒頭1-2文で読者を引き込むフック文\"\n}}",
            trend.item.title,
            research.summary,
            research.key_facts.join("\n- "),
            research.counterpoints.join("\n- "),
        );

        let text = self.call(&self.haiku_model, system, &user, 2000).await?;
        let cleaned = strip_code_fence(&text);
        let brief: ArticleBrief = serde_json::from_str(&cleaned).with_context(|| {
            format!("brief JSON parse failed. head: {}", cleaned.chars().take(200).collect::<String>())
        })?;
        Ok(brief)
    }

    /// Opus 4.7: ブリーフ + リサーチ → note 記事本文(markdown)
    pub async fn write(
        &self,
        brief: &ArticleBrief,
        research: &ResearchResult,
        trend: &SelectedTrend,
        target_chars: usize,
    ) -> Result<ArticleDraft> {
        let system = format!(
            "あなたはnote人気クリエイターとして、日本語で {target_chars} 文字前後の note 記事を執筆します。\
文体: 一人称「私」、ですます調、見出し構造(#/##)を活用、具体例・数字・引用を織り込む、\
投資助言に見える断定は避ける、ハッシュタグは末尾に付ける。\
出力は markdown 本文のみ。前後の説明やコードフェンスは不要。",
        );

        let user = format!(
            "【記事メタ】\nタイトル: {}\nカテゴリ: {}\nフック: {}\nタグ: {}\n\n\
【構成 (この順で見出しを立てる)】\n{}\n\n\
【リサーチ要約】\n{}\n\n\
【主要事実】\n- {}\n\n\
【反対意見・留意点】\n- {}\n\n\
【出典URL】\n- {}\n{}\n\n\
上記をもとに note 記事本文を markdown で書いてください。タイトルは # で始め、\
最後に「## 出典」セクションを設け、参考URLをリスト形式で記載してください。\
末尾に `#タグ1 #タグ2 #タグ3` 形式のハッシュタグ行も追加してください。",
            brief.title,
            brief.category,
            brief.hook,
            brief.tags.join(", "),
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

        let body_md = self.call(&self.opus_model, &system, &user, self.max_tokens).await?;
        let body_md = body_md.trim().trim_start_matches("```markdown").trim_start_matches("```").trim_end_matches("```").trim().to_string();
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

        let resp = self.http
            .post(ENDPOINT)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Anthropic API {}: {}", status, txt));
        }

        #[derive(Deserialize)]
        struct Resp { content: Vec<Block> }
        #[derive(Deserialize)]
        struct Block { #[serde(rename = "type")] kind: String, text: Option<String> }

        let parsed: Resp = resp.json().await?;
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

