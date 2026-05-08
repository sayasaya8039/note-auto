//! Phase 3 公開パイプライン
//!
//! writer の WrittenArticle を受けて:
//! 1. note.com へ自動投稿 (Playwright サイドカー)
//! 2. X へ告知投稿 (X API v2 + OAuth1.0a)
//! 3. Slack へ実行結果通知 (Incoming Webhook)

use anyhow::Result;
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

/// 記事リストを公開する (シリアル実行 + 記事間 cooldown)。
///
/// v0.9.5: `buffered(2)` → `buffered(1)` に変更しシリアル化。
///   2 並列だと `.cookies/browser-profile/` への同時アクセスで Chromium SingletonLock
///   衝突 → 片方が managed chromium fallback (cookie なし) で起動 → needs_login の
///   連鎖を引き起こしていた。シリアル化で衝突を完全排除。
///
/// v0.9.7: 各 publish の間に **8 秒 cooldown** を挿入。シリアル実行でも
///   前 publish のゾンビ msedge.exe が user-data-dir を握ったままになり、
///   次 spawn で SingletonLock 衝突 → managed chromium fallback (cookie なし)
///   → needs_login が後半 (TOP=7 で 5 記事目以降) で連発していた。
///   8 秒待つことで Edge プロセスの自然終了とロック解放を確実にする。
///
/// articles.to_vec() で HRTB lifetime 問題を回避。
///
/// L10: `#[tracing::instrument]` で publish stage 経過時間を自動計測。
/// W7-E (v0.9.1): `progress` を渡すと publish 完了時に sub_bar 経由で進捗を可視化。
///                None の場合は既存挙動と完全互換。
#[tracing::instrument(name = "publish", skip_all, fields(article_count = articles.len()))]
pub async fn publish_all(
    cfg: &Config,
    articles: &[WrittenArticle],
    progress: Option<&crate::display::PipelineProgress>,
) -> Result<Vec<PublishResult>> {
    if articles.is_empty() {
        return Ok(vec![]);
    }

    // HIGH #4 fix (codex review 2026-05-09):
    //   旧実装は daemon::execute_cycle のみ `.note-auto.lock` を取得していたため、
    //   別ターミナルから `note-auto publish --from drafts/.../articles.json` を直接叩くと
    //   daemon と同時に Chromium profile (cookie_dir/browser-profile/) を触り、
    //   SingletonLock 衝突 → managed chromium fallback (cookie なし) → needs_login の
    //   連鎖を引き起こしていた。
    //
    //   publish_all の入口で cookie_dir 単位の `FileLock` を取り、daemon と Command::Publish
    //   の両方を同じロックで排他化する。dry_run / note_skip 時は note サイドカーが
    //   そもそも起動しないのでロック不要。
    let _publish_lock: Option<crate::util::FileLock> =
        if cfg.publish.dry_run || cfg.publish.note_skip {
            None
        } else {
            let lock_path = std::path::Path::new(&cfg.publish.cookie_dir).join(".publish.lock");
            match crate::util::FileLock::acquire(&lock_path, true) {
                Ok(g) => {
                    tracing::debug!(path = %lock_path.display(), "publish lock acquired");
                    Some(g)
                }
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "failed to acquire publish lock at {}: {}\n\
                         別プロセス (daemon / 別 terminal の publish コマンド) が同じ\n\
                         cookie_dir で稼働中の可能性があります。完了を待つか手動で\n\
                         {} を削除してください。",
                        lock_path.display(),
                        e,
                        lock_path.display(),
                    ));
                }
            }
        };

    let mut results: Vec<PublishResult> = Vec::with_capacity(articles.len());
    for (idx, a) in articles.iter().enumerate() {
        // v0.9.7: 2 記事目以降は 8 秒 cooldown を挿入（msedge ロック解放待ち）
        if idx > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(8)).await;
        }
        results.push(publish_one(cfg, a).await);
    }

    // W7-E: 各記事の publish 結果を sub_bar に反映 (note status / x status 別に done/fail)
    if let Some(p) = progress {
        for r in &results {
            let bar = p.sub_bar(crate::display::Stage::Publish, &r.slug);
            let success_count = (r.note_status == "published" || r.note_status == "draft") as u8
                + (r.x_status == "posted") as u8;
            if r.errors.is_empty() && success_count > 0 {
                bar.done(&format!("note={}, x={}", r.note_status, r.x_status));
            } else if !r.errors.is_empty() {
                bar.fail(&r.errors.join(" / "));
            } else {
                bar.done(&format!("skipped (note={}, x={})", r.note_status, r.x_status));
            }
        }
    }
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
