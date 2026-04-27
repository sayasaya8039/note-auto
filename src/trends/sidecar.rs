//! Playwright サイドカー実行ヘルパー。
//! Node.js プロセスを spawn し、stdout に出力された JSON 配列を TrendItem に変換する。

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::path::Path;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::trends::TrendItem;

#[derive(Debug, Deserialize)]
pub struct SidecarItem {
    pub title: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default)]
    pub chain: Option<String>,
}

/// `node <script>` を spawn し、stdin に top_n を投げて stdout の JSON 配列を取得する。
/// timeout 秒で強制 kill。
pub async fn run(
    script: &Path,
    source_label: &str,
    top_n: usize,
    timeout_secs: u64,
) -> Result<Vec<TrendItem>> {
    if !script.exists() {
        return Err(anyhow!("sidecar script not found: {}", script.display()));
    }

    let mut child = Command::new("node")
        .arg(script)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn node {}", script.display()))?;

    if let Some(mut stdin) = child.stdin.take() {
        let payload = serde_json::json!({ "top_n": top_n });
        stdin
            .write_all(payload.to_string().as_bytes())
            .await
            .ok();
        drop(stdin);
    }

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        child.wait_with_output(),
    )
    .await;

    let output = match result {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(anyhow!("sidecar wait failed: {e}")),
        Err(_) => {
            return Err(anyhow!(
                "sidecar timeout {}s: {}",
                timeout_secs,
                script.display()
            ));
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "sidecar exit {}: {}",
            output.status,
            stderr.chars().take(500).collect::<String>()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Ok(vec![]);
    }
    let items: Vec<SidecarItem> = serde_json::from_str(trimmed).map_err(|e| {
        anyhow!(
            "sidecar JSON parse: {e}. head: {:?}",
            trimmed.chars().take(200).collect::<String>()
        )
    })?;

    let trend_items = items
        .into_iter()
        .map(|i| {
            let mut t = TrendItem::new(source_label, i.title);
            t.summary = i.summary;
            t.url = i.url.filter(|u| !u.is_empty());
            t.raw_score = i.score.unwrap_or(50.0);
            if let Some(chain) = i.chain {
                t.metrics = serde_json::json!({ "chain": chain });
            }
            t
        })
        .collect();

    Ok(trend_items)
}
