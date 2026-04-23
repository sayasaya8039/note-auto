//! xAI Grok Live Search API で X の直近トレンドを取得。
//! Grok-3 モデルに対して search_parameters 付きで投稿傾向を問い合わせ、
//! 返ってきたトピックを TrendItem として正規化する。

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::config::Config;
use crate::trends::TrendItem;

const ENDPOINT: &str = "https://api.x.ai/v1/chat/completions";
const MODEL: &str = "grok-3-latest";

pub async fn fetch(client: &reqwest::Client, cfg: &Config) -> Result<Vec<TrendItem>> {
    let Some(api_key) = cfg.trends.xai_api_key.as_deref() else {
        tracing::debug!("XAI_API_KEY 未設定 — X/Grok スキップ");
        return Ok(vec![]);
    };
    let hours = cfg.trends.window_hours.max(1);
    let top_n = cfg.trends.max_per_source.max(5);

    let prompt = format!(
        "あなたはXトレンド分析者です。直近{hours}時間で日本語/英語のXで伸びている話題のうち、\
note記事化して日本の読者に価値が出そうなトピックを {top_n} 件厳選してください。\
以下のJSON配列のみを返してください（コードブロック禁止）:\n\
[{{\"title\": \"...\", \"summary\": \"1-2文\", \"url\": \"代表投稿URL\", \"score\": 1-100の数値}}, ...]"
    );

    // NOTE: xAI の search_parameters は 2026-04 に deprecated (410 Gone) になり
    // Agent Tools API (https://docs.x.ai/docs/guides/tools/overview) へ移行する必要がある。
    // 当面は Grok モデルの学習データベースの知識でトピック案を出す。
    // 実時間 X トレンドは Phase 1b で Agent Tools API 対応時に復活させる。
    let body = json!({
        "model": MODEL,
        "stream": false,
        "temperature": 0.3,
        "messages": [
            {"role": "system", "content": "出力は厳密にJSON配列のみ。前後に説明文を入れない。あなたは X と web に関する最新知識を持つアナリスト。"},
            {"role": "user", "content": prompt}
        ]
    });

    let resp = client
        .post(ENDPOINT)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let txt = resp.text().await.unwrap_or_default();
        return Err(anyhow!("Grok API {}: {}", status, txt));
    }

    #[derive(Deserialize)]
    struct ChatResponse {
        choices: Vec<Choice>,
    }
    #[derive(Deserialize)]
    struct Choice {
        message: Msg,
    }
    #[derive(Deserialize)]
    struct Msg {
        content: String,
    }

    let parsed: ChatResponse = resp.json().await?;
    let content = parsed
        .choices
        .first()
        .ok_or_else(|| anyhow!("empty choices"))?
        .message
        .content
        .clone();

    let cleaned = strip_code_fence(&content);

    #[derive(Deserialize, Serialize)]
    struct Topic {
        title: String,
        #[serde(default)]
        summary: Option<String>,
        #[serde(default)]
        url: Option<String>,
        #[serde(default)]
        score: Option<f64>,
    }

    let topics: Vec<Topic> = serde_json::from_str(&cleaned).map_err(|e| {
        anyhow!("Grok content JSON parse failed: {e}. content head: {:?}", cleaned.chars().take(200).collect::<String>())
    })?;

    let items: Vec<TrendItem> = topics
        .into_iter()
        .map(|t| {
            let mut it = TrendItem::new("x", t.title);
            it.summary = t.summary;
            it.url = t.url;
            it.raw_score = t.score.unwrap_or(50.0);
            it
        })
        .collect();

    Ok(items)
}

fn strip_code_fence(s: &str) -> String {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix("```json") {
        return rest.trim_end_matches("```").trim().to_string();
    }
    if let Some(rest) = t.strip_prefix("```") {
        return rest.trim_end_matches("```").trim().to_string();
    }
    t.to_string()
}
