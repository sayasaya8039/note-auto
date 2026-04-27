//! Google News RSS 検索ベースの汎用フェッチャ。
//! `https://news.google.com/rss/search?q=...&hl=ja&gl=JP&ceid=JP:ja` を叩き、
//! 任意のクエリで関連ニュースを TrendItem として返す。

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};

use crate::trends::TrendItem;

const ENDPOINT: &str = "https://news.google.com/rss/search";

/// 与えたクエリで Google News RSS を叩き、結果を TrendItem として返す。
pub async fn fetch_query(
    client: &reqwest::Client,
    source_label: &str,
    query: &str,
    max_items: usize,
) -> Result<Vec<TrendItem>> {
    let url = format!(
        "{}?q={}&hl=ja&gl=JP&ceid=JP:ja",
        ENDPOINT,
        urlencoding::encode(query)
    );

    let resp = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0 note-auto/0.2")
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(anyhow!("gnews search {} -> {}", query, resp.status()));
    }

    let body = resp.text().await?;
    let channel = rss::Channel::read_from(body.as_bytes())
        .map_err(|e| anyhow!("rss parse failed for {query}: {e}"))?;

    let items: Vec<TrendItem> = channel
        .items()
        .iter()
        .take(max_items)
        .map(|it| {
            let title = it.title().unwrap_or("(no title)").to_string();
            let link = it.link().map(|s| s.to_string());
            let summary = it.description().map(strip_html);
            let pub_date = it
                .pub_date()
                .and_then(|d| DateTime::parse_from_rfc2822(d).ok())
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or_else(Utc::now);
            let mut t = TrendItem::new(source_label, title);
            t.url = link;
            t.summary = summary;
            t.fetched_at = pub_date;
            // RSS は順序が新着順なので、index ベースで簡易スコア付け
            t.raw_score = 50.0;
            t
        })
        .collect();

    Ok(items)
}

fn strip_html(s: &str) -> String {
    let re = regex::Regex::new(r"<[^>]+>").unwrap();
    let stripped = re.replace_all(s, "");
    html_escape::decode_html_entities(&stripped).trim().to_string()
}
