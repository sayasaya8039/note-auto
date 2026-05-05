//! 過去に書いた記事の履歴。重複投稿回避用。
//!
//! `drafts/history.json` に { slug, title, date, source_url } を追記保存。
//! 新規トレンド選定時にタイトル類似度で重複を弾く。

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const DEFAULT_PATH: &str = "drafts/history.json";
const SIMILARITY_THRESHOLD: f64 = 0.55; // bigram Jaccard、selectより厳しめ

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub slug: String,
    pub title: String,
    pub date: String,            // YYYY-MM-DD
    pub source_url: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Default)]
pub struct History {
    pub path: PathBuf,
    pub entries: Vec<HistoryEntry>,
}

impl History {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let path = path.map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from(DEFAULT_PATH));
        if !path.exists() {
            return Ok(Self { path, entries: vec![] });
        }
        let txt = std::fs::read_to_string(&path)?;
        let entries: Vec<HistoryEntry> = match serde_json::from_str(&txt) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "history.json corrupted, backing up");
                let bak = path.with_extension("json.bak");
                let _ = std::fs::copy(&path, &bak);
                vec![]
            }
        };
        Ok(Self { path, entries })
    }

    pub fn append(&mut self, entry: HistoryEntry) -> Result<()> {
        self.entries.push(entry);
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, serde_json::to_string_pretty(&self.entries)?)?;
        Ok(())
    }

    /// タイトルが履歴のいずれかと類似していれば true
    pub fn has_similar(&self, title: &str) -> bool {
        self.entries.iter().any(|e| crate::util::title_similarity(&e.title, title) >= SIMILARITY_THRESHOLD)
    }

    /// URL が履歴にあれば true (完全一致)
    pub fn has_url(&self, url: &str) -> bool {
        let u = url.trim();
        !u.is_empty() && self.entries.iter().any(|e| e.source_url.as_deref() == Some(u))
    }
}
