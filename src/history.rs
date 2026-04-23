//! 過去に書いた記事の履歴。重複投稿回避用。
//!
//! `drafts/history.json` に { slug, title, date, source_url } を追記保存。
//! 新規トレンド選定時にタイトル類似度で重複を弾く。

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use unicode_segmentation::UnicodeSegmentation;

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
        let entries: Vec<HistoryEntry> = serde_json::from_str(&txt).unwrap_or_default();
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
        self.entries.iter().any(|e| title_similarity(&e.title, title) >= SIMILARITY_THRESHOLD)
    }

    /// URL が履歴にあれば true (完全一致)
    pub fn has_url(&self, url: &str) -> bool {
        let u = url.trim();
        !u.is_empty() && self.entries.iter().any(|e| e.source_url.as_deref() == Some(u))
    }
}

fn title_similarity(a: &str, b: &str) -> f64 {
    let ga = bigrams(a);
    let gb = bigrams(b);
    if ga.is_empty() || gb.is_empty() { return 0.0; }
    let inter = ga.intersection(&gb).count() as f64;
    let union = ga.union(&gb).count() as f64;
    inter / union
}

fn bigrams(s: &str) -> std::collections::HashSet<String> {
    let lower = s.to_lowercase();
    let gs: Vec<&str> = lower.graphemes(true).collect();
    if gs.len() < 2 { return std::iter::once(lower).collect(); }
    gs.windows(2).map(|w| w.concat()).collect()
}
