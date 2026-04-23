//! AI クライアント統合モジュール
//!
//! - anthropic: Claude Opus 4.7 (本文執筆) / Haiku 4.5 (タイトル・分類・スラッグ)
//! - xai: Grok (トピック深掘りリサーチ)
//! - openai: gpt-image-1 (アイキャッチ画像生成)

use serde::{Deserialize, Serialize};

pub mod anthropic;
pub mod openai;
pub mod xai;

/// Haiku が出力する記事ブリーフ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArticleBrief {
    pub title: String,
    pub slug: String,
    pub category: String,
    pub outline: Vec<String>,
    pub tags: Vec<String>,
    pub hook: String,
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
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("note-auto/0.2")
        .timeout(std::time::Duration::from_secs(180))
        .gzip(true)
        .build()
        .expect("reqwest client")
}

/// JSON レスポンスからコードフェンスを剥がす
pub(crate) fn strip_code_fence(s: &str) -> String {
    let t = s.trim();
    for prefix in ["```json", "```JSON", "```"] {
        if let Some(rest) = t.strip_prefix(prefix) {
            return rest.trim_end_matches("```").trim().to_string();
        }
    }
    t.to_string()
}
