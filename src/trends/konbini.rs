//! コンビニ来週新商品トレンド (セブン-イレブン / ローソン / ファミマ)
//!
//! Phase 1 (案2): xAI Grok の知識ベースから「来週発売予定の新商品」を抽出する。
//! Phase 2 (案1): セブン `sej.co.jp/products/a/week_new/`、ローソン
//! `lawson.co.jp/recommend/new/`、ファミマ `family.co.jp/goods.html` を
//! HTML スクレイプして TrendItem 化する予定。

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::config::Config;
use crate::trends::TrendItem;

const ENDPOINT: &str = "https://api.x.ai/v1/chat/completions";

pub async fn fetch(client: &reqwest::Client, cfg: &Config) -> Result<Vec<TrendItem>> {
    let Some(api_key) = cfg.trends.xai_api_key.as_deref() else {
        tracing::debug!("XAI_API_KEY 未設定 — konbini スキップ");
        return Ok(vec![]);
    };
    let top_n = cfg.trends.max_per_source.max(5);
    let model = &cfg.writer.grok_model;

    let prompt = format!(
        "あなたは日本のコンビニ新商品アナリストです。\
直近〜来週にセブン-イレブン / ローソン / ファミリーマートで発売される(された)新商品のうち、\
SNSや note 読者の興味を引きそうなものを {top_n} 件厳選してください。\
スイーツ・パン・麺・冷食・ドリンク・コラボ商品・限定品を優先。\
以下のJSON配列のみを返してください（コードブロック禁止）:\n\
[{{\"title\": \"〇〇(チェーン名)\", \"summary\": \"発売日/特徴/価格を1-2文\", \"url\": \"公式商品ページURLまたは空文字\", \"score\": 1-100の話題性数値}}, ...]"
    );

    let body = json!({
        "model": model,
        "stream": false,
        "temperature": 0.3,
        "messages": [
            {"role": "system", "content": "出力は厳密にJSON配列のみ。前後に説明文を入れない。あなたは日本のコンビニ事情に詳しいトレンドアナリスト。"},
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
        return Err(anyhow!("Grok API (konbini) {}: {}", status, txt));
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
        .ok_or_else(|| anyhow!("empty choices (konbini)"))?
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
        anyhow!(
            "konbini Grok JSON parse failed: {e}. content head: {:?}",
            cleaned.chars().take(200).collect::<String>()
        )
    })?;

    let items: Vec<TrendItem> = topics
        .into_iter()
        .map(|t| {
            let mut it = TrendItem::new("konbini", t.title);
            it.summary = t.summary;
            it.url = t.url.filter(|u| !u.is_empty());
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
