//! Slack Incoming Webhook で実行結果サマリを投稿

use std::sync::LazyLock;

use anyhow::{anyhow, Result};
use serde_json::json;

use super::RunSummary;
use crate::config::Config;

static SLACK_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("slack client")
});

static SLACK_PROGRESS_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("slack progress client")
});

pub async fn post_summary(cfg: &Config, summary: &RunSummary) -> Result<()> {
    if cfg.publish.dry_run {
        tracing::info!("[dry-run] Slack通知スキップ");
        return Ok(());
    }
    let Some(url) = cfg.publish.slack_webhook_url.as_deref() else {
        tracing::info!("SLACK_WEBHOOK_URL 未設定 — Slack通知スキップ");
        return Ok(());
    };

    let blocks = build_blocks(summary);
    let body = json!({ "blocks": blocks });

    let resp = SLACK_CLIENT.post(url).json(&body).send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let txt = resp.text().await.unwrap_or_default();
        return Err(anyhow!("Slack webhook {}: {}", status, txt));
    }
    Ok(())
}

fn build_blocks(s: &RunSummary) -> Vec<serde_json::Value> {
    let total = s.articles.len();
    let note_ok = s.articles.iter().filter(|a| a.note_status == "published" || a.note_status == "draft").count();
    let x_ok = s.articles.iter().filter(|a| a.x_status == "posted").count();
    let errors: Vec<String> = s.articles.iter().flat_map(|a| a.errors.clone()).collect();

    let header = format!("📝 note-auto 実行レポート ({})", s.date);
    let summary_line = format!(
        "記事: {} / note投稿: {} / X告知: {} / 総文字数: {} / 所要: {}s",
        total, note_ok, x_ok, s.total_chars, s.duration_secs
    );

    let mut articles_text = String::new();
    for a in &s.articles {
        let note_mark = match a.note_status.as_str() {
            "published" => "🟢",
            "draft" => "🟡",
            "skipped" => "⚪",
            _ => "🔴",
        };
        articles_text.push_str(&format!("{} *{}*\n", note_mark, escape(&a.title)));
        if let Some(u) = &a.note_url {
            articles_text.push_str(&format!("   ↳ <{u}|note>  "));
        }
        if let Some(u) = &a.x_tweet_url {
            articles_text.push_str(&format!("<{u}|X>"));
        }
        articles_text.push('\n');
    }

    let mut blocks = vec![
        json!({"type": "header", "text": {"type": "plain_text", "text": header}}),
        json!({"type": "section", "text": {"type": "mrkdwn", "text": summary_line}}),
        json!({"type": "divider"}),
        json!({"type": "section", "text": {"type": "mrkdwn", "text": articles_text}}),
    ];

    if !errors.is_empty() {
        let err_text = format!(":warning: エラー:\n```\n{}\n```", errors.join("\n"));
        blocks.push(json!({"type": "section", "text": {"type": "mrkdwn", "text": err_text}}));
    }

    blocks
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// 進捗メッセージを Slack に送信 (ベストエフォート、失敗は warn のみ)
pub async fn post_progress(cfg: &Config, text: &str) {
    if cfg.publish.dry_run {
        tracing::info!(msg = text, "[dry-run] Slack進捗スキップ");
        return;
    }
    if !cfg.publish.progress_notifications {
        return;
    }
    let Some(url) = cfg.publish.slack_webhook_url.as_deref() else {
        return;
    };
    let body = json!({ "text": text });
    match SLACK_PROGRESS_CLIENT.post(url).json(&body).send().await {
        Ok(r) if !r.status().is_success() => {
            let st = r.status();
            let t = r.text().await.unwrap_or_default();
            tracing::warn!(status = %st, body = %t.chars().take(120).collect::<String>(), "slack progress: non-200");
        }
        Err(e) => tracing::warn!(error = %e, "slack progress: request failed"),
        _ => {}
    }
}
