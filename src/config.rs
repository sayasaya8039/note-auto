use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub trends: TrendsConfig,
    #[serde(default)]
    pub scoring: ScoringConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrendsConfig {
    /// xAI Grok API key (env: XAI_API_KEY)
    #[serde(default = "env_xai_key")]
    pub xai_api_key: Option<String>,
    /// 直近何時間のトレンドを見るか
    #[serde(default = "default_hours")]
    pub window_hours: u32,
    /// 1ソースあたり最大取得件数
    #[serde(default = "default_per_source")]
    pub max_per_source: usize,
    /// 有効化するソース (x, google, note, hn, reddit)
    #[serde(default = "default_sources")]
    pub sources: Vec<String>,
    /// Reddit 対象 subreddit 群
    #[serde(default = "default_subreddits")]
    pub subreddits: Vec<String>,
}

impl Default for TrendsConfig {
    fn default() -> Self {
        Self {
            xai_api_key: env_xai_key(),
            window_hours: default_hours(),
            max_per_source: default_per_source(),
            sources: default_sources(),
            subreddits: default_subreddits(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScoringConfig {
    /// ソース別ウェイト
    #[serde(default = "default_source_weights")]
    pub source_weights: std::collections::HashMap<String, f64>,
    /// 重複判定の類似度閾値 (0-1)
    #[serde(default = "default_dedup")]
    pub dedup_threshold: f64,
}

impl Default for ScoringConfig {
    fn default() -> Self {
        Self {
            source_weights: default_source_weights(),
            dedup_threshold: default_dedup(),
        }
    }
}

fn env_xai_key() -> Option<String> {
    std::env::var("XAI_API_KEY").ok()
}
fn default_hours() -> u32 { 24 }
fn default_per_source() -> usize { 20 }
fn default_sources() -> Vec<String> {
    // Reddit は Cloudflare TLS フィンガープリンティングで rustls ベースの reqwest が
    // 弾かれるためデフォルト無効。Phase 2 で OAuth 対応後に有効化する。
    vec!["x".into(), "google".into(), "note".into(), "hn".into()]
}
fn default_subreddits() -> Vec<String> {
    vec!["programming".into(), "technology".into(), "MachineLearning".into()]
}
fn default_source_weights() -> std::collections::HashMap<String, f64> {
    let mut m = std::collections::HashMap::new();
    m.insert("x".into(), 1.2);
    m.insert("google".into(), 1.0);
    m.insert("note".into(), 1.1);
    m.insert("hn".into(), 0.9);
    m.insert("reddit".into(), 0.8);
    m
}
fn default_dedup() -> f64 { 0.65 }

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            tracing::warn!(path = %path.display(), "config not found, using defaults");
            return Ok(Self::default());
        }
        let txt = std::fs::read_to_string(path)
            .with_context(|| format!("read {}", path.display()))?;
        let mut cfg: Config = toml::from_str(&txt)
            .with_context(|| format!("parse {}", path.display()))?;
        if cfg.trends.xai_api_key.is_none() {
            cfg.trends.xai_api_key = env_xai_key();
        }
        Ok(cfg)
    }
}
