//! Hacker News Algolia API から直近24hのトップストーリー取得。
//! https://hn.algolia.com/api/v1/search?tags=story&numericFilters=created_at_i>SINCE

use anyhow::Result;
use chrono::{Duration, Utc};
use serde::Deserialize;

use crate::config::Config;
use crate::trends::TrendItem;

pub async fn fetch(client: &reqwest::Client, cfg: &Config) -> Result<Vec<TrendItem>> {
    let since = (Utc::now() - Duration::hours(cfg.trends.window_hours as i64)).timestamp();
    let url = format!(
        "https://hn.algolia.com/api/v1/search?tags=story&numericFilters=created_at_i>{}&hitsPerPage={}",
        since,
        cfg.trends.max_per_source
    );

    #[derive(Deserialize)]
    struct Resp { hits: Vec<Hit> }
    #[derive(Deserialize)]
    struct Hit {
        title: Option<String>,
        url: Option<String>,
        #[serde(rename = "objectID")]
        object_id: String,
        points: Option<f64>,
        num_comments: Option<f64>,
    }

    let resp: Resp = client.get(&url).send().await?.error_for_status()?.json().await?;
    let items = resp.hits.into_iter().filter_map(|h| {
        let title = h.title?;
        let mut it = TrendItem::new("hn", &title);
        it.url = h.url.or_else(|| Some(format!("https://news.ycombinator.com/item?id={}", h.object_id)));
        let pts = h.points.unwrap_or(0.0);
        let cmt = h.num_comments.unwrap_or(0.0);
        it.raw_score = pts + cmt * 0.5;
        it.metrics = serde_json::json!({ "points": pts, "comments": cmt });
        Some(it)
    }).collect();

    Ok(items)
}
