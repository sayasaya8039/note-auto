//! Phase 3 公開パイプライン
//!
//! writer の WrittenArticle を受けて:
//! 1. note.com へ自動投稿 (Playwright サイドカー)
//! 2. X へ告知投稿 (X API v2 + OAuth1.0a)
//! 3. Slack へ実行結果通知 (Incoming Webhook)

use anyhow::Result;
use futures::stream::{self, StreamExt};
use serde::Serialize;

pub mod note;
pub mod slack;
pub mod x_post;

pub use slack::post_progress;

use crate::config::Config;
use crate::writer::WrittenArticle;

/// 記事1件あたりの公開結果
#[derive(Debug, Clone, Serialize)]
pub struct PublishResult {
    pub slug: String,
    pub title: String,
    pub note_url: Option<String>,
    pub note_status: String,
    pub x_tweet_url: Option<String>,
    pub x_status: String,
    pub errors: Vec<String>,
}

/// 全体の実行サマリ (Slack通知用)
#[derive(Debug, Clone, Serialize)]
pub struct RunSummary {
    pub date: String,
    pub articles: Vec<PublishResult>,
    pub total_chars: usize,
    pub duration_secs: u64,
}

/// 記事リストを並列公開する (並列度 2、Playwright セッション競合を抑制)。
///
/// articles.to_vec() で HRTB lifetime 問題を回避し、buffered(2) で並列処理。
pub async fn publish_all(cfg: &Config, articles: &[WrittenArticle]) -> Result<Vec<PublishResult>> {
    if articles.is_empty() {
        return Ok(vec![]);
    }

    let owned: Vec<WrittenArticle> = articles.to_vec();
    let results: Vec<PublishResult> = stream::iter(owned)
        .map(|a| {
            let cfg = cfg.clone();
            async move { publish_one(&cfg, &a).await }
        })
        .buffered(2)
        .collect()
        .await;
    Ok(results)
}

async fn publish_one(cfg: &Config, article: &WrittenArticle) -> PublishResult {
    let mut result = PublishResult {
        slug: article.slug.clone(),
        title: article.title.clone(),
        note_url: None,
        note_status: "skipped".into(),
        x_tweet_url: None,
        x_status: "skipped".into(),
        errors: vec![],
    };

    // 1. note.com 投稿
    match note::publish(cfg, article).await {
        Ok(r) => {
            result.note_url = r.url;
            result.note_status = r.status;
        }
        Err(e) => {
            result.note_status = "error".into();
            result.errors.push(format!("note: {e}"));
            tracing::error!(slug = %article.slug, error = %e, "note publish failed");
        }
    }

    // 2. X 告知 — note が「published」(完全公開) のときのみ発火。
    //    note_publish=false (安全モード、下書き保存のみ) のときは X も自動でスキップ。
    if cfg.publish.x_announce {
        if result.note_status != "published" {
            tracing::info!(
                slug = %article.slug,
                note_status = %result.note_status,
                "note が published ではないため X 告知を自動スキップ (安全モード)"
            );
            result.x_status = "skipped".into();
        } else {
            match x_post::announce(cfg, article, result.note_url.as_deref()).await {
                Ok(Some(url)) => {
                    result.x_tweet_url = Some(url);
                    result.x_status = "posted".into();
                }
                Ok(None) => result.x_status = "skipped".into(),
                Err(e) => {
                    result.x_status = "error".into();
                    result.errors.push(format!("x: {e}"));
                    tracing::warn!(slug = %article.slug, error = %e, "x announce failed");
                }
            }
        }
    }

    result
}

pub async fn notify_summary(cfg: &Config, summary: &RunSummary) -> Result<()> {
    slack::post_summary(cfg, summary).await
}
