use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub trends: TrendsConfig,
    #[serde(default)]
    pub scoring: ScoringConfig,
    #[serde(default)]
    pub writer: WriterConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrendsConfig {
    #[serde(default = "env_xai_key")]
    pub xai_api_key: Option<String>,
    #[serde(default = "default_hours")]
    pub window_hours: u32,
    #[serde(default = "default_per_source")]
    pub max_per_source: usize,
    #[serde(default = "default_sources")]
    pub sources: Vec<String>,
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
    #[serde(default = "default_source_weights")]
    pub source_weights: std::collections::HashMap<String, f64>,
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

/// Phase 2: AI執筆パイプライン設定
#[derive(Debug, Clone, Deserialize)]
pub struct WriterConfig {
    /// Anthropic API キー (env: ANTHROPIC_API_KEY)
    #[serde(default = "env_anthropic_key")]
    pub anthropic_api_key: Option<String>,
    /// OpenAI API キー (env: OPENAI_API_KEY)
    #[serde(default = "env_openai_key")]
    pub openai_api_key: Option<String>,
    /// 本文執筆モデル
    #[serde(default = "default_opus_model")]
    pub opus_model: String,
    /// 分類・タイトル生成モデル
    #[serde(default = "default_haiku_model")]
    pub haiku_model: String,
    /// Grok リサーチモデル
    #[serde(default = "default_grok_model")]
    pub grok_model: String,
    /// 画像生成モデル
    #[serde(default = "default_image_model")]
    pub image_model: String,
    /// 画像サイズ (OpenAI Images API)
    #[serde(default = "default_image_size")]
    pub image_size: String,
    /// 本文の目標文字数
    #[serde(default = "default_article_chars")]
    pub target_chars: usize,
    /// 執筆時の max_tokens
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// dry-run: AI呼び出しをスキップしスタブで代替
    #[serde(default)]
    pub dry_run: bool,
}

impl Default for WriterConfig {
    fn default() -> Self {
        Self {
            anthropic_api_key: env_anthropic_key(),
            openai_api_key: env_openai_key(),
            opus_model: default_opus_model(),
            haiku_model: default_haiku_model(),
            grok_model: default_grok_model(),
            image_model: default_image_model(),
            image_size: default_image_size(),
            target_chars: default_article_chars(),
            max_tokens: default_max_tokens(),
            dry_run: false,
        }
    }
}

fn env_xai_key() -> Option<String> { std::env::var("XAI_API_KEY").ok() }
fn env_anthropic_key() -> Option<String> { std::env::var("ANTHROPIC_API_KEY").ok() }
fn env_openai_key() -> Option<String> { std::env::var("OPENAI_API_KEY").ok() }

fn default_hours() -> u32 { 24 }
fn default_per_source() -> usize { 20 }
fn default_sources() -> Vec<String> {
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
fn default_opus_model() -> String { "claude-opus-4-7".into() }
fn default_haiku_model() -> String { "claude-haiku-4-5-20251001".into() }
fn default_grok_model() -> String { "grok-3-latest".into() }
fn default_image_model() -> String { "gpt-image-1".into() }
fn default_image_size() -> String { "1536x1024".into() }
fn default_article_chars() -> usize { 3000 }
fn default_max_tokens() -> u32 { 8000 }

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        // .env を最初に読む (存在しなくてもエラーにしない)
        let _ = dotenvy::from_filename(".env");

        if !path.exists() {
            tracing::warn!(path = %path.display(), "config not found, using defaults");
            return Ok(Self::default());
        }
        let txt = std::fs::read_to_string(path)
            .with_context(|| format!("read {}", path.display()))?;
        let mut cfg: Config = toml::from_str(&txt)
            .with_context(|| format!("parse {}", path.display()))?;
        // 環境変数があれば補完
        if cfg.trends.xai_api_key.is_none() { cfg.trends.xai_api_key = env_xai_key(); }
        if cfg.writer.anthropic_api_key.is_none() { cfg.writer.anthropic_api_key = env_anthropic_key(); }
        if cfg.writer.openai_api_key.is_none() { cfg.writer.openai_api_key = env_openai_key(); }
        Ok(cfg)
    }
}
