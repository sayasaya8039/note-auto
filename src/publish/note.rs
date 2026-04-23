//! note.com 自動投稿 — Playwright サイドカー (Bun/tsx) 経由
//!
//! Rust は JSON でタスクを stdin に流し、サイドカー側が stdout に JSON で結果を返す。
//! Cookie は Playwright の storageState (.cookies/note.json) に永続化。

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::config::Config;
use crate::writer::WrittenArticle;

#[derive(Debug, Clone, Serialize)]
pub struct PublishOutput {
    pub status: String,
    pub url: Option<String>,
}

#[derive(Debug, Serialize)]
struct PublishInput<'a> {
    md_path: String,
    image_path: Option<String>,
    title: &'a str,
    tags: &'a [String],
    publish: bool,
    cookie_dir: &'a str,
}

#[derive(Debug, Deserialize)]
struct SidecarResponse {
    status: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

pub async fn publish(cfg: &Config, article: &WrittenArticle) -> Result<PublishOutput> {
    if cfg.publish.dry_run {
        tracing::info!(slug = %article.slug, "[dry-run] note投稿スキップ");
        return Ok(PublishOutput { status: "skipped".into(), url: None });
    }

    let script = &cfg.publish.playwright_script;
    if !std::path::Path::new(script).exists() {
        return Err(anyhow!("Playwright script not found: {}", script));
    }

    let input = PublishInput {
        md_path: article.md_path.to_string_lossy().to_string(),
        image_path: article.image_path.as_ref().map(|p| p.to_string_lossy().to_string()),
        title: &article.title,
        tags: &article.tags,
        publish: cfg.publish.note_publish,
        cookie_dir: &cfg.publish.cookie_dir,
    };
    let input_json = serde_json::to_string(&input)?;

    let runtime_parts: Vec<&str> = cfg.publish.playwright_runtime.split_whitespace().collect();
    let (program, base_args) = runtime_parts.split_first()
        .ok_or_else(|| anyhow!("publish.playwright_runtime is empty"))?;

    let mut cmd = Command::new(program);
    cmd.args(base_args).arg(script);
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn()
        .with_context(|| format!("spawn {} (runtime={})", script, cfg.publish.playwright_runtime))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input_json.as_bytes()).await?;
        stdin.shutdown().await?;
    }

    let out = child.wait_with_output().await?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    if !stderr.is_empty() {
        tracing::debug!(stderr = %stderr, "playwright stderr");
    }
    if !out.status.success() {
        return Err(anyhow!(
            "playwright exited with {}: stdout={} stderr={}",
            out.status, stdout, stderr
        ));
    }

    // stdout の最後の行に JSON が入っている前提
    let last_line = stdout.lines().rev().find(|l| l.trim_start().starts_with('{'))
        .ok_or_else(|| anyhow!("no JSON in playwright stdout. full: {}", stdout))?;
    let resp: SidecarResponse = serde_json::from_str(last_line)
        .with_context(|| format!("parse playwright response: {last_line}"))?;

    if let Some(err) = resp.error {
        return Err(anyhow!("playwright reported: {}", err));
    }
    Ok(PublishOutput {
        status: resp.status,
        url: resp.url,
    })
}
