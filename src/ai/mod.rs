//! AI クライアント統合モジュール
//!
//! - anthropic: Claude Opus 4.7 (本文執筆) / Haiku 4.5 (タイトル・分類・スラッグ)
//! - xai: Grok (トピック深掘りリサーチ)
//! - openai: gpt-image-1 (アイキャッチ画像生成)

use serde::{Deserialize, Serialize};

pub mod anthropic;
pub mod gemini;
pub mod nvidia;
pub mod openai;
pub mod pollo;
pub mod xai;

/// Haiku が出力する記事ブリーフ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArticleBrief {
    pub title: String,
    pub slug: String,
    pub category: String,
    pub outline: Vec<String>,
    /// SEO 1位を狙う 20個のハッシュタグ (# なし、文字列のみ)
    pub tags: Vec<String>,
    pub hook: String,
    /// 4枚の画像生成プロンプト (英語、日本人・フォトリアル指定済み)
    /// [0]=見出し (hero), [1..3]=本文挿入用
    #[serde(default)]
    pub image_prompts: Vec<String>,
}

/// Grok のリサーチ結果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchResult {
    pub summary: String,
    pub key_facts: Vec<String>,
    pub citations: Vec<String>,
    pub counterpoints: Vec<String>,
}

/// Opus が出力する記事本文
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArticleDraft {
    pub title: String,
    pub body_markdown: String,
    pub char_count: usize,
}

/// 画像生成結果 (PNG bytes)
#[derive(Debug, Clone)]
pub struct ImageAsset {
    pub prompt: String,
    pub png_bytes: Vec<u8>,
}

/// 共通 HTTP クライアント (AI 用 — タイムアウト長め)
///
/// H1 (panic=abort safety): expect() を `unwrap_or_else` + `tracing::warn` で代替。
/// `util::http_client_long()` は `LazyLock<reqwest::Client>` の `clone()` ラッパなので
/// 現実的にはエラーは発生しないが、panic=abort 環境で予防的に fallback を用意:
/// builder error 時は `reqwest::Client::new()` でデフォルト client を返し、`tracing::warn!`
/// でエラーを記録する（静かな失敗を避ける）。API シグネチャは無変更。
pub fn http_client() -> reqwest::Client {
    crate::util::http_client_long().unwrap_or_else(|e| {
        tracing::warn!(error = %e, "http_client_long() failed, falling back to default Client");
        reqwest::Client::new()
    })
}

/// 今日の日付を JST (Asia/Tokyo) で "YYYY-MM-DD (曜日)" 形式で返す。
/// 記事生成時のハルシネーション防止用に、全 LLM プロンプトへアンカーとして注入する。
pub fn today_jst_label() -> String {
    use chrono::Datelike;
    let jst = chrono_tz::Asia::Tokyo;
    let now = chrono::Utc::now().with_timezone(&jst);
    let wd = ["月", "火", "水", "木", "金", "土", "日"]
        [now.weekday().num_days_from_monday() as usize];
    format!("{}-{:02}-{:02} ({})", now.year(), now.month(), now.day(), wd)
}

/// LLM プロンプト共通の「日付ハルシネーション防止」節。
/// brief / research / write すべてに貼り付ける。
pub fn date_policy_block(today: &str) -> String {
    format!(
        "【日付の厳守ルール — ハルシネーション防止】\n\
- 今日の日付は {today} (JST)。これを唯一の確定事実として扱うこと。\n\
- 本文中に書いてよい具体的な年月日は、次のいずれかに限る:\n\
  (a) 今日の日付そのもの ({today})\n\
  (b) 【リサーチ要約】【主要事実】【参考URL】または元記事タイトルに明示されている日付\n\
  (c) 一般常識として固定されている歴史的日付 (例: 東京五輪 2021 など、疑う余地のないもの)\n\
- 上記に該当しない日付 (発表日・発売日・イベント日・最終更新日など) を推測で書くのは禁止。\n\
  不明な場合は「近日」「最近」「本記事執筆時点」「{today} 現在」等の相対表現を使うこと。\n\
- 「今年」「今月」「先週」「数日前」は今日 ({today}) を基準に解釈する。未来の出来事を過去形で書かない。\n\
- 年だけなら書いてよいが、特定月日 (例: 『4月15日』『2026年3月3日』) は必ず裏取りされたものだけ。\n"
    )
}

/// JSON レスポンスからコードフェンスを剥がす
pub(crate) fn strip_code_fence(s: &str) -> String {
    crate::util::strip_code_fence(s)
}
