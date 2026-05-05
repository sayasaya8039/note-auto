//! 過去に書いた記事の履歴。重複投稿回避用。
//!
//! `drafts/history.json` に { slug, title, date, source_url } を追記保存。
//! 新規トレンド選定時にタイトル類似度で重複を弾く。
//!
//! v0.7.7 改善 (Q2/Q3):
//! - `url_set: HashSet<String>` で has_url を O(n) → O(1) に高速化
//! - append は temp+rename アトミック書き込みで耐クラッシュ性を確保

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
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

/// 投稿履歴。`url_set` で URL 重複チェックを O(1) に高速化する。
#[derive(Debug, Default)]
pub struct History {
    pub path: PathBuf,
    pub entries: Vec<HistoryEntry>,
    /// source_url の正規化済みセット (has_url の O(1) ルックアップ用)
    url_set: HashSet<String>,
}

impl History {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let path = path.map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from(DEFAULT_PATH));
        if !path.exists() {
            return Ok(Self { path, entries: vec![], url_set: HashSet::new() });
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
        // URL セットをロード時に一括構築 (O(n)、以降は O(1) ルックアップ)
        let url_set: HashSet<String> = entries.iter()
            .filter_map(|e| e.source_url.as_ref())
            .map(|u| u.trim().to_string())
            .filter(|u| !u.is_empty())
            .collect();
        Ok(Self { path, entries, url_set })
    }

    /// エントリを追記してファイルに永続化する。
    ///
    /// temp+rename によるアトミック書き込みで、書き込み途中のクラッシュ時に
    /// ファイルが破損しないことを保証する。
    pub fn append(&mut self, entry: HistoryEntry) -> Result<()> {
        if let Some(u) = &entry.source_url {
            let key = u.trim().to_string();
            if !key.is_empty() {
                self.url_set.insert(key);
            }
        }
        self.entries.push(entry);
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // アトミック書き込み: .tmp に書いてからリネームで耐クラッシュ
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&self.entries)?)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    /// タイトルが履歴のいずれかと類似していれば true (O(n) — Jaccard は近似)
    pub fn has_similar(&self, title: &str) -> bool {
        self.entries.iter().any(|e| crate::util::title_similarity(&e.title, title) >= SIMILARITY_THRESHOLD)
    }

    /// URL が履歴にあれば true — O(1) HashSet ルックアップ
    pub fn has_url(&self, url: &str) -> bool {
        let u = url.trim();
        !u.is_empty() && self.url_set.contains(u)
    }
}
