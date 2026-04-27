//! 100均新商品トレンド (ダイソー / セリア / キャンドゥ / ワッツ)
//!
//! Phase 2 ハイブリッドフェッチチェーン:
//! 1. Playwright サイドカー (`scripts/scrape-hyakkin.mjs`) — 公式ページ直スクレイプ
//! 2. Google News RSS (各チェーン × 新商品 クエリ) — 静的に確実
//! 3. Grok 知識ベース — 最終フォールバック

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;

use crate::config::Config;
use crate::trends::{gnews_rss, sidecar, TrendItem};

const ENDPOINT: &str = "https://api.x.ai/v1/chat/completions";
const SIDECAR_SCRIPT: &str = "scripts/scrape-hyakkin.mjs";
const SIDECAR_TIMEOUT_SECS: u64 = 90;

const GNEWS_QUERIES: &[&str] = &[
    "ダイソー 新商品",
    "セリア 新商品",
    "キャンドゥ 新商品",
    "100均 新商品",
];

pub async fn fetch(client: &reqwest::Client, cfg: &Config) -> Result<Vec<TrendItem>> {
    let top_n = cfg.trends.max_per_source.max(5);

    // 1. Playwright サイドカー
    let script_path = PathBuf::from(SIDECAR_SCRIPT);
    match sidecar::run(&script_path, "hyakkin", top_n, SIDECAR_TIMEOUT_SECS).await {
        Ok(items) if !items.is_empty() => {
            tracing::info!(count = items.len(), "hyakkin: playwright sidecar OK");
            return Ok(items);
        }
        Ok(_) => tracing::debug!("hyakkin: sidecar empty, fallback to gnews"),
        Err(e) => tracing::warn!(error = %e, "hyakkin: sidecar failed, fallback to gnews"),
    }

    // 2. Google News RSS (複数クエリを並行)
    let rss_futs = GNEWS_QUERIES.iter().map(|q| {
        gnews_rss::fetch_query(client, "hyakkin", q, top_n.div_ceil(GNEWS_QUERIES.len()))
    });
    let rss_results = futures::future::join_all(rss_futs).await;
    let mut rss_items = Vec::new();
    for r in rss_results {
        if let Ok(mut items) = r {
            rss_items.append(&mut items);
        }
    }
    if !rss_items.is_empty() {
        tracing::info!(count = rss_items.len(), "hyakkin: gnews RSS OK");
        rss_items.truncate(top_n);
        return Ok(rss_items);
    }

    // 3. Grok 知識ベースフォールバック
    tracing::info!("hyakkin: falling back to Grok knowledge");
    grok_fallback(client, cfg, top_n).await
}

async fn grok_fallback(
    client: &reqwest::Client,
    cfg: &Config,
    top_n: usize,
) -> Result<Vec<TrendItem>> {
    let Some(api_key) = cfg.trends.xai_api_key.as_deref() else {
        tracing::debug!("XAI_API_KEY 未設定 — hyakkin Grok フォールバックもスキップ");
        return Ok(vec![]);
    };
    let model = &cfg.writer.grok_model;

    let prompt = format!(
        "あなたは日本の100円ショップ(ダイソー/セリア/キャンドゥ/ワッツ)新商品アナリストです。\
直近で X / Instagram / TikTok でバズっている、または公式から発表された新商品のうち、\
note 読者の生活トピックとして価値が出そうなものを {top_n} 件厳選してください。\
収納・キッチン・推し活グッズ・季節商品・コラボ商品・便利グッズを優先。\
以下のJSON配列のみを返してください（コードブロック禁止）:\n\
[{{\"title\": \"〇〇(チェーン名)\", \"summary\": \"特徴/価格/評判を1-2文\", \"url\": \"公式商品ページURLまたは空文字\", \"score\": 1-100の話題性数値}}, ...]"
    );

    let body = json!({
        "model": model,
        "stream": false,
        "temperature": 0.3,
        "messages": [
            {"role": "system", "content": "出力は厳密にJSON配列のみ。前後に説明文を入れない。あなたは日本の100円ショップ事情とSNSトレンドに詳しいアナリスト。"},
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
        return Err(anyhow!("Grok API (hyakkin) {}: {}", status, txt));
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
        .ok_or_else(|| anyhow!("empty choices (hyakkin)"))?
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
            "hyakkin Grok JSON parse failed: {e}. content head: {:?}",
            cleaned.chars().take(200).collect::<String>()
        )
    })?;

    let items: Vec<TrendItem> = topics
        .into_iter()
        .map(|t| {
            let mut it = TrendItem::new("hyakkin", t.title);
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
