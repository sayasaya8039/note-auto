use anyhow::Result;
use chrono::{DateTime, Utc};
use futures::future::join_all;
use serde::{Deserialize, Serialize};

use crate::config::Config;

pub mod gnews_rss;
pub mod google;
pub mod google_news;
pub mod hn;
pub mod hyakkin;
pub mod konbini;
pub mod note_rss;
pub mod reddit;
pub mod sidecar;
pub mod x_grok;

/// 1つのトレンド候補 (全ソース共通)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendItem {
    /// ソース名: "x", "google", "gnews", "note", "hn", "reddit", "konbini", "hyakkin"
    pub source: String,
    /// トピックタイトル / キーワード
    pub title: String,
    /// 本文 / 概要 (省略可)
    pub summary: Option<String>,
    /// 参照URL
    pub url: Option<String>,
    /// 一次スコア (ソース内で正規化されていない生値: impressions, points, etc)
    pub raw_score: f64,
    /// エンゲージ系メトリクス（表示用）
    #[serde(default)]
    pub metrics: serde_json::Value,
    /// ソースから取得した画像URL（konbini/hyakkin の Playwright スクレイプで埋まる）
    /// 記事生成時、本文挿入画像はこの URL からダウンロードして優先利用する。
    #[serde(default)]
    pub image_urls: Vec<String>,
    /// 取得時刻
    pub fetched_at: DateTime<Utc>,
}

impl TrendItem {
    pub fn new(source: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            title: title.into(),
            summary: None,
            url: None,
            raw_score: 0.0,
            metrics: serde_json::Value::Null,
            image_urls: Vec::new(),
            fetched_at: Utc::now(),
        }
    }
}

/// HTTP クライアントを共有
pub fn http_client() -> reqwest::Client {
    crate::util::http_client().expect("reqwest client")
}

/// 全ソースを並行で取得。個別失敗はログして継続。
///
/// L10: `#[tracing::instrument]` で stage 経過時間を自動計測（RUST_LOG=info,note_auto=debug）。
#[tracing::instrument(name = "fetch", skip_all, fields(source_count = cfg.trends.sources.len()))]
pub async fn fetch_all(cfg: &Config) -> Result<Vec<TrendItem>> {
    let client = http_client();
    let enabled: std::collections::HashSet<&str> =
        cfg.trends.sources.iter().map(|s| s.as_str()).collect();

    let mut futs: Vec<futures::future::BoxFuture<'_, (&'static str, Result<Vec<TrendItem>>)>> = vec![];

    if enabled.contains("x") {
        futs.push(Box::pin(async {
            ("x", x_grok::fetch(&client, cfg).await)
        }));
    }
    if enabled.contains("google") {
        futs.push(Box::pin(async {
            ("google", google::fetch(&client, cfg).await)
        }));
    }
    if enabled.contains("gnews") {
        futs.push(Box::pin(async {
            ("gnews", google_news::fetch(&client, cfg).await)
        }));
    }
    if enabled.contains("note") {
        futs.push(Box::pin(async {
            ("note", note_rss::fetch(&client, cfg).await)
        }));
    }
    if enabled.contains("hn") {
        futs.push(Box::pin(async {
            ("hn", hn::fetch(&client, cfg).await)
        }));
    }
    if enabled.contains("reddit") {
        futs.push(Box::pin(async {
            ("reddit", reddit::fetch(&client, cfg).await)
        }));
    }
    if enabled.contains("konbini") {
        futs.push(Box::pin(async {
            ("konbini", konbini::fetch(&client, cfg).await)
        }));
    }
    if enabled.contains("hyakkin") {
        futs.push(Box::pin(async {
            ("hyakkin", hyakkin::fetch(&client, cfg).await)
        }));
    }

    let results = join_all(futs).await;
    let mut all = Vec::new();
    for (src, res) in results {
        match res {
            Ok(items) => {
                tracing::info!(source = src, count = items.len(), "fetched");
                all.extend(items);
            }
            Err(e) => {
                tracing::warn!(source = src, error = %e, "fetch failed");
            }
        }
    }
    Ok(all)
}
