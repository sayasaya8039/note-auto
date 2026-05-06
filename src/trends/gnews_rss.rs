//! Google News RSS 検索ベースの汎用フェッチャ。
//! `https://news.google.com/rss/search?q=...&hl=ja&gl=JP&ceid=JP:ja` を叩き、
//! 任意のクエリで関連ニュースを TrendItem として返す。
//!
//! v0.5: 各 item の Google News redirect URL を follow して
//!       実記事ページの og:image を抽出し image_urls に格納する。

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use std::sync::LazyLock;

use crate::trends::TrendItem;

const ENDPOINT: &str = "https://news.google.com/rss/search";
const OG_IMAGE_TIMEOUT_SECS: u64 = 8;
const OG_IMAGE_MAX_PAR: usize = 5; // 同時 og:image 取得数

/// PERF-4 (v0.9.3): OG image 抽出用 regex を LazyLock で 1 度だけコンパイル。
/// 旧実装は extract_og_image 呼び出し毎 (= URL 毎) に Regex::new していたため、
/// OG enrich N 件で N 回コンパイル。固定 pattern なので static 化で µs オーダー削減。
static OG_IMAGE_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r#"(?is)<meta[^>]+(?:property|name)\s*=\s*["'](?:og:image(?::secure_url)?|twitter:image)["'][^>]*content\s*=\s*["']([^"']+)["']"#,
    )
    .expect("OG_IMAGE_RE compile (固定 pattern, ビルド時に必ず通る)")
});

/// 与えたクエリで Google News RSS を叩き、結果を TrendItem として返す。
pub async fn fetch_query(
    client: &reqwest::Client,
    source_label: &str,
    query: &str,
    max_items: usize,
) -> Result<Vec<TrendItem>> {
    // DEP-1 (v0.9.3): urlencoding crate を撤去し、既存依存の `url` で代用
    let encoded_query: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
    let url = format!("{ENDPOINT}?q={encoded_query}&hl=ja&gl=JP&ceid=JP:ja");

    let resp = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0 note-auto/0.5")
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(anyhow!("gnews search {} -> {}", query, resp.status()));
    }

    let body = resp.text().await?;
    let channel = rss::Channel::read_from(body.as_bytes())
        .map_err(|e| anyhow!("rss parse failed for {query}: {e}"))?;

    let mut items: Vec<TrendItem> = channel
        .items()
        .iter()
        .take(max_items)
        .map(|it| {
            let title = it.title().unwrap_or("(no title)").to_string();
            let link = it.link().map(|s| s.to_string());
            let summary = it.description().map(crate::util::strip_html);
            let pub_date = it
                .pub_date()
                .and_then(|d| DateTime::parse_from_rfc2822(d).ok())
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or_else(Utc::now);
            let mut t = TrendItem::new(source_label, title);
            t.url = link;
            t.summary = summary;
            t.fetched_at = pub_date;
            t.raw_score = 50.0;
            t
        })
        .collect();

    // og:image を上位 OG_IMAGE_MAX_PAR 件だけ並行取得 (失敗時は無視)
    enrich_og_images(client, &mut items, OG_IMAGE_MAX_PAR).await;

    Ok(items)
}

/// 各 TrendItem の URL を訪問し og:image を取得して image_urls に追記。
/// 失敗は無視 (image_urls は空のまま)。
///
/// L11: `take(parallelism)` の意味論を「対象数上限 = 並列度」から
///      「全件 / 並列度 N」に正規化。`buffer_unordered` で本来の並列度制御を実現。
async fn enrich_og_images(
    client: &reqwest::Client,
    items: &mut [TrendItem],
    parallelism: usize,
) {
    use futures::stream::{self, StreamExt};
    let parallelism = parallelism.max(1);
    let targets: Vec<(usize, String)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, t)| t.url.clone().map(|u| (i, u)))
        .collect();
    if targets.is_empty() {
        return;
    }

    let results: Vec<(usize, Option<String>)> = stream::iter(targets.into_iter().map(|(idx, url)| {
        let client = client.clone();
        async move {
            let og = fetch_og_image(&client, &url).await;
            (idx, og)
        }
    }))
    .buffer_unordered(parallelism)
    .collect()
    .await;

    for (idx, og) in results {
        if let Some(og_url) = og {
            if let Some(item) = items.get_mut(idx) {
                item.image_urls.push(og_url);
            }
        }
    }
}

/// 記事ページから og:image / twitter:image を抽出。リダイレクトは reqwest が follow。
async fn fetch_og_image(client: &reqwest::Client, url: &str) -> Option<String> {
    let req = client
        .get(url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
        )
        .header("Accept", "text/html,application/xhtml+xml")
        .header("Accept-Language", "ja-JP,ja;q=0.9");
    let resp = match tokio::time::timeout(
        std::time::Duration::from_secs(OG_IMAGE_TIMEOUT_SECS),
        req.send(),
    )
    .await
    {
        Ok(Ok(r)) => r,
        _ => return None,
    };
    if !resp.status().is_success() {
        return None;
    }
    let final_url = resp.url().clone();
    let body = match tokio::time::timeout(
        std::time::Duration::from_secs(OG_IMAGE_TIMEOUT_SECS),
        resp.text(),
    )
    .await
    {
        Ok(Ok(t)) => t,
        _ => return None,
    };
    extract_og_image(&body, &final_url)
}

fn extract_og_image(html: &str, base: &url::Url) -> Option<String> {
    // PERF-4 (v0.9.3): OG_IMAGE_RE を LazyLock 化済、毎回 Regex::new コストなし
    let captures = OG_IMAGE_RE.captures(html)?;
    let raw = captures.get(1)?.as_str();
    let abs = base.join(raw).ok()?;
    let s = abs.to_string();
    if s.starts_with("http") {
        Some(s)
    } else {
        None
    }
}
