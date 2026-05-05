//! Google Trends Daily RSS (geo=JP) をパース。
//! 公式エンドポイント: https://trends.google.co.jp/trending/rss?geo=JP

use anyhow::Result;

use crate::config::Config;
use crate::trends::TrendItem;

const URL: &str = "https://trends.google.co.jp/trending/rss?geo=JP";

pub async fn fetch(client: &reqwest::Client, cfg: &Config) -> Result<Vec<TrendItem>> {
    let text = client.get(URL).send().await?.error_for_status()?.text().await?;
    let channel = rss::Channel::read_from(text.as_bytes())?;

    let mut items: Vec<TrendItem> = Vec::new();
    for item in channel.items().iter().take(cfg.trends.max_per_source) {
        let Some(title) = item.title() else { continue };

        // ht:approx_traffic extension (e.g. "20,000+") をスコアに使う
        let traffic = item
            .extensions()
            .get("ht")
            .and_then(|ns| ns.get("approx_traffic"))
            .and_then(|exts| exts.first())
            .and_then(|e| e.value.clone());

        let score = traffic
            .as_deref()
            .map(parse_traffic)
            .unwrap_or(100.0);

        let summary = item
            .description()
            .map(html_to_text)
            .or_else(|| item.extensions()
                .get("ht")
                .and_then(|ns| ns.get("news_item"))
                .and_then(|v| v.first())
                .and_then(|e| e.children.get("news_item_title"))
                .and_then(|v| v.first())
                .and_then(|e| e.value.clone()));

        let mut it = TrendItem::new("google", title);
        it.summary = summary;
        it.url = item.link().map(|s| s.to_string());
        it.raw_score = score;
        it.metrics = serde_json::json!({ "approx_traffic": traffic });
        items.push(it);
    }
    Ok(items)
}

fn parse_traffic(s: &str) -> f64 {
    let cleaned: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    cleaned.parse().unwrap_or(100.0)
}

fn html_to_text(s: &str) -> String {
    crate::util::strip_html(s)
}
