//! Reddit 公開 Atom フィードから複数 subreddit のトップを取得。
//! JSON エンドポイントは Cloudflare の TLS フィンガープリントで reqwest を弾くため、
//! フィード版 (https://old.reddit.com/r/<sub>/top.rss) を使う。
//! スコアは順位ベース（RSSには得点が含まれないため）。

use anyhow::Result;
use atom_syndication::Feed;
use futures::future::join_all;
use std::sync::LazyLock;

use crate::config::Config;
use crate::trends::TrendItem;

static REDDIT_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent("windows:note-auto:0.1 (by /u/note_auto_bot)")
        .timeout(std::time::Duration::from_secs(30))
        .gzip(true)
        .build()
        .expect("reddit client")
});

pub async fn fetch(_client: &reqwest::Client, cfg: &Config) -> Result<Vec<TrendItem>> {
    let subs = &cfg.trends.subreddits;
    if subs.is_empty() {
        return Ok(vec![]);
    }
    let t = match cfg.trends.window_hours {
        0..=24 => "day",
        25..=168 => "week",
        _ => "month",
    };
    let per_sub = (cfg.trends.max_per_source / subs.len().max(1)).max(5);

    let futs = subs.iter().map(|sub| {
        let url = format!("https://old.reddit.com/r/{sub}/top.rss?t={t}&limit={per_sub}");
        let c = REDDIT_CLIENT.clone();
        let sub = sub.clone();
        async move { fetch_sub(&c, &url, &sub, per_sub).await }
    });

    let results = join_all(futs).await;
    let mut all = Vec::new();
    for r in results {
        match r {
            Ok(items) => all.extend(items),
            Err(e) => tracing::warn!(error = %e, "reddit sub failed"),
        }
    }
    Ok(all)
}

async fn fetch_sub(client: &reqwest::Client, url: &str, sub: &str, limit: usize) -> Result<Vec<TrendItem>> {
    let text = client.get(url).send().await?.error_for_status()?.text().await?;
    let feed: Feed = text.parse()?;
    let items = feed.entries.into_iter().enumerate().map(|(rank, e)| {
        let title = e.title.value;
        let link = e.links.first().map(|l| l.href.clone());
        let mut it = TrendItem::new("reddit", title);
        it.url = link;
        // RSSにscoreがないので順位ベース
        it.raw_score = (limit as f64 - rank as f64).max(1.0);
        it.metrics = serde_json::json!({ "subreddit": sub, "rank": rank + 1 });
        it
    }).collect();
    Ok(items)
}
