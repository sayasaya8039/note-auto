//! 常駐モード — tokio-cron-scheduler で 07:00 JST 発火
//!
//! 発火時: fetch-trends → write → publish → notify を連結実行。

use anyhow::{Context, Result};
use chrono::Utc;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_cron_scheduler::{Job, JobScheduler};

use crate::config::Config;
use crate::publish::{publish_all, notify_summary, PublishResult, RunSummary};
use crate::scoring::select_top;
use crate::trends::fetch_all;
use crate::writer;

pub async fn run_daemon(cfg: Config) -> Result<()> {
    let mut sched = JobScheduler::new().await?;
    let tz: chrono_tz::Tz = cfg.schedule.timezone.parse()
        .with_context(|| format!("invalid timezone: {}", cfg.schedule.timezone))?;

    let cron = cfg.schedule.cron.clone();
    let shared = Arc::new(cfg);
    let shared_job = Arc::clone(&shared);

    let job = Job::new_cron_job_async_tz(cron.as_str(), tz, move |_uuid, _l| {
        let cfg = Arc::clone(&shared_job);
        Box::pin(async move {
            tracing::info!("cron tick — note-auto cycle 開始");
            match execute_cycle(&cfg).await {
                Ok(summary) => {
                    tracing::info!(articles = summary.articles.len(), "cycle 完了");
                    if let Err(e) = notify_summary(&cfg, &summary).await {
                        tracing::error!(error = %e, "slack notify failed");
                    }
                }
                Err(e) => tracing::error!(error = %e, "cycle failed"),
            }
        })
    })?;

    sched.add(job).await?;
    sched.start().await?;
    tracing::info!(cron = %shared.schedule.cron, tz = %shared.schedule.timezone, "daemon 起動");
    println!("✓ daemon 常駐中 (cron={}, tz={})", shared.schedule.cron, shared.schedule.timezone);
    println!("  Ctrl+C で停止");

    // Ctrl+C まで待機
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutdown signal received");
    sched.shutdown().await?;
    Ok(())
}

pub async fn execute_cycle(cfg: &Config) -> Result<RunSummary> {
    let start = std::time::Instant::now();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let out_dir = PathBuf::from("drafts").join(&today);
    std::fs::create_dir_all(&out_dir)?;

    // 1. fetch
    let items = fetch_all(cfg).await?;
    let selected = select_top(items, cfg.schedule.daily_top);
    let trends_path = out_dir.join("trends.json");
    std::fs::write(&trends_path, serde_json::to_string_pretty(&selected)?)?;
    tracing::info!(count = selected.len(), "trends selected");

    // 2. write
    let articles = writer::run(cfg, &selected, &out_dir).await?;

    // 3. publish
    let publish_results: Vec<PublishResult> = publish_all(cfg, &articles).await?;

    let total_chars: usize = articles.iter().map(|a| a.char_count).sum();
    let duration_secs = start.elapsed().as_secs();

    let summary = RunSummary {
        date: today,
        articles: publish_results,
        total_chars,
        duration_secs,
    };

    // 実行サマリを JSON 保存
    let summary_path = out_dir.join("summary.json");
    std::fs::write(&summary_path, serde_json::to_string_pretty(&summary)?)?;

    let _ = Utc::now(); // keep chrono import
    Ok(summary)
}
