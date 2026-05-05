//! note.com 新着/人気 RSS を集約。
//! - https://note.com/topic/<topic>/rss (カテゴリ新着)
//! - https://note.com/trend/rss (トレンド)

use anyhow::Result;

use crate::config::Config;
use crate::trends::TrendItem;

// 多様なカテゴリから拾うため hashtag を広く張る
// tech/AI は HN + Grok でカバーできるので note 側は非 tech 寄せ
const FEEDS: &[(&str, &str)] = &[
    ("note-trending", "https://note.com/trending/rss"),
    // ライフ & 日常
    ("note-lifestyle", "https://note.com/hashtag/ライフスタイル/rss"),
    ("note-daily", "https://note.com/hashtag/日常/rss"),
    ("note-essay", "https://note.com/hashtag/エッセイ/rss"),
    // エンタメ
    ("note-entame", "https://note.com/hashtag/エンタメ/rss"),
    ("note-movie", "https://note.com/hashtag/映画/rss"),
    ("note-music", "https://note.com/hashtag/音楽/rss"),
    // グルメ・健康・子育て
    ("note-food", "https://note.com/hashtag/グルメ/rss"),
    ("note-health", "https://note.com/hashtag/健康/rss"),
    ("note-parenting", "https://note.com/hashtag/子育て/rss"),
    // 国内ニュース・社会
    ("note-news", "https://note.com/hashtag/ニュース/rss"),
    ("note-japan", "https://note.com/hashtag/日本/rss"),
    // ガジェット・デザイン (軽く tech)
    ("note-gadget", "https://note.com/hashtag/ガジェット/rss"),
    ("note-design", "https://note.com/hashtag/デザイン/rss"),
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
        it.summary = item.description().map(crate::util::strip_html);
        out.push(it);
    }
    Ok(out)
}

