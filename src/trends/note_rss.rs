//! note.com 新着/人気 RSS を集約。
//! - https://note.com/topic/<topic>/rss (カテゴリ新着)
//! - https://note.com/trend/rss (トレンド)

use anyhow::Result;

use crate::config::Config;
use crate::trends::TrendItem;

const FEEDS: &[(&str, &str)] = &[
    ("note-trending", "https://note.com/trending/rss"),
    ("note-ai", "https://note.com/hashtag/AI/rss"),
    ("note-tech", "https://note.com/hashtag/テクノロジー/rss"),
    ("note-biz", "https://note.com/hashtag/ビジネス/rss"),
];

pub async fn fetch(client: &reqwest::Client, cfg: &Config) -> Result<Vec<TrendItem>> {
    let mut all = Vec::new();
    let limit = cfg.trends.max_per_source;

    for (tag, url) in FEEDS {
        match fetch_feed(client, url).await {
            Ok(items) => {
                let boost: f64 = if *tag == "note-trending" { 1.5 } else { 1.0 };
                for (rank, mut it) in items.into_iter().enumerate() {
                    // 順位ベースのスコア (上位ほど高い)
                    it.raw_score = (limit as f64 - rank as f64).max(1.0) * boost;
                    all.push(it);
                    if all.len() >= limit * 2 {
                        break;
                    }
                }
            }
            Err(e) => tracing::warn!(url = %url, error = %e, "note feed failed"),
        }
    }
    Ok(all)
}

async fn fetch_feed(client: &reqwest::Client, url: &str) -> Result<Vec<TrendItem>> {
    let text = client.get(url).send().await?.error_for_status()?.text().await?;
    let channel = rss::Channel::read_from(text.as_bytes())?;
    let mut out = Vec::new();
    for item in channel.items() {
        let Some(title) = item.title() else { continue };
        let mut it = TrendItem::new("note", title);
        it.url = item.link().map(|s| s.to_string());
        it.summary = item.description().map(|d| strip_html(d));
        out.push(it);
    }
    Ok(out)
}

fn strip_html(s: &str) -> String {
    let re = regex::Regex::new(r"<[^>]+>").unwrap();
    let stripped = re.replace_all(s, " ");
    html_escape::decode_html_entities(&stripped)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
