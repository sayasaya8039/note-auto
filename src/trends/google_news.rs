//! Google News 日本版トップニュース RSS。
//! https://news.google.com/rss?hl=ja&gl=JP&ceid=JP:ja
//!
//! foryou は認証必須のため代替として公開トップニュースを使用。
//! 国内ニュース (政治/経済/社会/エンタメ/スポーツ) を広く拾える。

use anyhow::Result;

use crate::config::Config;
use crate::trends::TrendItem;

const FEEDS: &[(&str, &str)] = &[
    ("top", "https://news.google.com/rss?hl=ja&gl=JP&ceid=JP:ja"),
    ("world", "https://news.google.com/rss/headlines/section/topic/WORLD?hl=ja&gl=JP&ceid=JP:ja"),
    ("nation", "https://news.google.com/rss/headlines/section/topic/NATION?hl=ja&gl=JP&ceid=JP:ja"),
    ("business", "https://news.google.com/rss/headlines/section/topic/BUSINESS?hl=ja&gl=JP&ceid=JP:ja"),
    ("entertainment", "https://news.google.com/rss/headlines/section/topic/ENTERTAINMENT?hl=ja&gl=JP&ceid=JP:ja"),
    ("science", "https://news.google.com/rss/headlines/section/topic/SCIENCE?hl=ja&gl=JP&ceid=JP:ja"),
    ("sports", "https://news.google.com/rss/headlines/section/topic/SPORTS?hl=ja&gl=JP&ceid=JP:ja"),
    ("health", "https://news.google.com/rss/headlines/section/topic/HEALTH?hl=ja&gl=JP&ceid=JP:ja"),
];

pub async fn fetch(client: &reqwest::Client, cfg: &Config) -> Result<Vec<TrendItem>> {
    let mut all = Vec::new();
    let limit = cfg.trends.max_per_source;
    for (tag, url) in FEEDS {
        match fetch_feed(client, url, tag).await {
            Ok(items) => {
                // top セクションは boost、カテゴリは通常重み
                let boost: f64 = if *tag == "top" { 1.3 } else { 1.0 };
                for (rank, mut it) in items.into_iter().enumerate() {
                    it.raw_score = ((limit as f64 - rank as f64).max(1.0)) * boost;
                    all.push(it);
                    if all.len() >= limit * 3 { break; }
                }
            }
            Err(e) => tracing::warn!(url = %url, error = %e, "google news feed failed"),
        }
    }
    Ok(all)
}

async fn fetch_feed(client: &reqwest::Client, url: &str, tag: &str) -> Result<Vec<TrendItem>> {
    let text = client.get(url).send().await?.error_for_status()?.text().await?;
    let channel = rss::Channel::read_from(text.as_bytes())?;
    let mut out = Vec::new();
    for item in channel.items() {
        let Some(title) = item.title() else { continue };
        let mut it = TrendItem::new("gnews", title);
        it.url = item.link().map(|s| s.to_string());
        it.summary = item.description().map(|d| strip_html(d));
        it.metrics = serde_json::json!({ "section": tag });
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
