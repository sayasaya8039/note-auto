//! コンビニ来週新商品トレンド (セブン-イレブン / ローソン / ファミマ)
//!
//! Phase 2 ハイブリッドフェッチチェーン:
//! 1. Playwright サイドカー (`scripts/scrape-konbini.mjs`) — 公式ページ直スクレイプ
//! 2. Google News RSS (各チェーン × 新商品 クエリ) — 静的に確実
//! 3. Grok 知識ベース — 最終フォールバック

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;

use crate::config::Config;
use crate::trends::{gnews_rss, sidecar, TrendItem};

const ENDPOINT: &str = "https://api.x.ai/v1/chat/completions";
const SIDECAR_SCRIPT: &str = "scripts/scrape-konbini.mjs";
const SIDECAR_TIMEOUT_SECS: u64 = 90;

const GNEWS_QUERIES: &[&str] = &[
    "セブンイレブン 新商品",
    "ローソン 新商品",
    "ファミリーマート 新商品",
    "コンビニ 来週 新商品",
];

pub async fn fetch(client: &reqwest::Client, cfg: &Config) -> Result<Vec<TrendItem>> {
    let top_n = cfg.trends.max_per_source.max(5);

    // 1. Playwright サイドカー
    let script_path = PathBuf::from(SIDECAR_SCRIPT);
    match sidecar::run(&script_path, "konbini", top_n, SIDECAR_TIMEOUT_SECS).await {
        Ok(items) if !items.is_empty() => {
            tracing::info!(count = items.len(), "konbini: playwright sidecar OK");
            return Ok(items);
        }
        Ok(_) => tracing::debug!("konbini: sidecar empty, fallback to gnews"),
        Err(e) => tracing::warn!(error = %e, "konbini: sidecar failed, fallback to gnews"),
    }

    // 2. Google News RSS (複数クエリを並行)
    let rss_futs = GNEWS_QUERIES.iter().map(|q| {
        gnews_rss::fetch_query(client, "konbini", q, top_n.div_ceil(GNEWS_QUERIES.len()))
    });
    let rss_results = futures::future::join_all(rss_futs).await;
    let mut rss_items = Vec::new();
    for mut items in rss_results.into_iter().flatten() {
        rss_items.append(&mut items);
    }
    if !rss_items.is_empty() {
        tracing::info!(count = rss_items.len(), "konbini: gnews RSS OK");
        rss_items.truncate(top_n);
        return Ok(rss_items);
    }

    // 3. Grok 知識ベースフォールバック
    tracing::info!("konbini: falling back to Grok knowledge");
    grok_fallback(client, cfg, top_n).await
}

async fn grok_fallback(
    client: &reqwest::Client,
    cfg: &Config,
    top_n: usize,
) -> Result<Vec<TrendItem>> {
    let Some(api_key) = cfg.trends.xai_api_key.as_deref() else {
        tracing::debug!("XAI_API_KEY 未設定 — konbini Grok フォールバックもスキップ");
        return Ok(vec![]);
    };
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

    // HIGH #5 fix (codex review 2026-05-09): retry/backoff 抜けを util::send_with_retry で補修。
    // 旧 `.send().await?` は 429/503/timeout で即失敗 → konbini source 全滅。
    let resp = crate::util::send_with_retry(
        || {
            client
                .post(ENDPOINT)
                .bearer_auth(api_key)
                .json(&body)
                .send()
        },
        3,
        "konbini-grok",
    )
    .await
    .map_err(|e| anyhow!("Grok API (konbini) send: {}", e))?;

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

    let cleaned = crate::util::strip_code_fence(&content);

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
