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
    #[serde(default)]
    pub publish: PublishConfig,
    #[serde(default)]
    pub schedule: ScheduleConfig,
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
    /// Pollo AI API キー (env: POLLO_API_KEY) — 代替画像プロバイダ
    #[serde(default = "env_pollo_key")]
    pub pollo_api_key: Option<String>,
    /// 画像プロバイダ: "pollo" | "openai" (default: pollo if key set, else openai)
    #[serde(default = "default_image_provider")]
    pub image_provider: String,
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
            pollo_api_key: env_pollo_key(),
            image_provider: default_image_provider(),
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

/// Phase 3: 公開 + 告知
#[derive(Debug, Clone, Deserialize)]
pub struct PublishConfig {
    /// note.com 投稿用 Playwright スクリプトパス
    #[serde(default = "default_playwright_script")]
    pub playwright_script: String,
    /// Playwright 実行ランタイム (bun / npx tsx など)
    #[serde(default = "default_playwright_runtime")]
    pub playwright_runtime: String,
    /// Cookie 永続化ディレクトリ
    #[serde(default = "default_cookie_dir")]
    pub cookie_dir: String,
    /// note 自動投稿 (true=公開ボタン押下、false=下書き保存のみ)
    #[serde(default)]
    pub note_publish: bool,
    /// X 告知投稿を有効化するか
    #[serde(default = "default_bool_true")]
    pub x_announce: bool,
    /// X OAuth1.0a (env: X_API_KEY / X_API_SECRET / X_ACCESS_TOKEN / X_ACCESS_SECRET)
    #[serde(default = "env_x_api_key")]
    pub x_api_key: Option<String>,
    #[serde(default = "env_x_api_secret")]
    pub x_api_secret: Option<String>,
    #[serde(default = "env_x_access_token")]
    pub x_access_token: Option<String>,
    #[serde(default = "env_x_access_secret")]
    pub x_access_secret: Option<String>,
    /// Slack Incoming Webhook URL (env: SLACK_WEBHOOK_URL)
    #[serde(default = "env_slack_webhook")]
    pub slack_webhook_url: Option<String>,
    /// 進捗通知 (各ステージで Slack に短いメッセージ送信) を有効化
    #[serde(default = "default_bool_true")]
    pub progress_notifications: bool,
    /// dry-run: 外部呼び出しをスキップ
    #[serde(default)]
    pub dry_run: bool,
}

impl Default for PublishConfig {
    fn default() -> Self {
        Self {
            playwright_script: default_playwright_script(),
            playwright_runtime: default_playwright_runtime(),
            cookie_dir: default_cookie_dir(),
            note_publish: false,
            x_announce: true,
            x_api_key: env_x_api_key(),
            x_api_secret: env_x_api_secret(),
            x_access_token: env_x_access_token(),
            x_access_secret: env_x_access_secret(),
            slack_webhook_url: env_slack_webhook(),
            progress_notifications: true,
            dry_run: false,
        }
    }
}

/// Phase 3: スケジューラ
#[derive(Debug, Clone, Deserialize)]
pub struct ScheduleConfig {
    /// cron 式 (6フィールド: sec min hour day month dow)
    #[serde(default = "default_cron")]
    pub cron: String,
    /// タイムゾーン (IANA 形式)
    #[serde(default = "default_tz")]
    pub timezone: String,
    /// 1発火で処理する記事数
    #[serde(default = "default_daily_top")]
    pub daily_top: usize,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            cron: default_cron(),
            timezone: default_tz(),
            daily_top: default_daily_top(),
        }
    }
}

fn env_xai_key() -> Option<String> { std::env::var("XAI_API_KEY").ok() }
fn env_anthropic_key() -> Option<String> { std::env::var("ANTHROPIC_API_KEY").ok() }
fn env_openai_key() -> Option<String> { std::env::var("OPENAI_API_KEY").ok() }
fn env_pollo_key() -> Option<String> { std::env::var("POLLO_API_KEY").ok() }
fn default_image_provider() -> String { "pollo".into() }
fn env_x_api_key() -> Option<String> { std::env::var("X_API_KEY").ok() }
fn env_x_api_secret() -> Option<String> { std::env::var("X_API_SECRET").ok() }
fn env_x_access_token() -> Option<String> { std::env::var("X_ACCESS_TOKEN").ok() }
fn env_x_access_secret() -> Option<String> { std::env::var("X_ACCESS_SECRET").ok() }
fn env_slack_webhook() -> Option<String> { std::env::var("SLACK_WEBHOOK_URL").ok() }
fn default_playwright_script() -> String { "scripts/note-publish.ts".into() }
fn default_playwright_runtime() -> String { "bun".into() }
fn default_cookie_dir() -> String { ".cookies".into() }
fn default_bool_true() -> bool { true }
fn default_cron() -> String { "0 0 7 * * *".into() } // 毎日 07:00
fn default_tz() -> String { "Asia/Tokyo".into() }
fn default_daily_top() -> usize { 3 }

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
// pollo モデル名 (例: "openai-gpt-image-2-0" / "openai-gpt-image-1-5" / "gpt-4o" / "dall-e-3")
// OpenAI 直呼びの場合は "gpt-image-1" / "dall-e-3"
fn default_image_model() -> String { "openai-gpt-image-2-0".into() }
// pollo 使用時は "16:9", openai 直呼びは "1536x1024"
fn default_image_size() -> String { "16:9".into() }
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
        if cfg.writer.pollo_api_key.is_none() { cfg.writer.pollo_api_key = env_pollo_key(); }
        if cfg.publish.x_api_key.is_none() { cfg.publish.x_api_key = env_x_api_key(); }
        if cfg.publish.x_api_secret.is_none() { cfg.publish.x_api_secret = env_x_api_secret(); }
        if cfg.publish.x_access_token.is_none() { cfg.publish.x_access_token = env_x_access_token(); }
        if cfg.publish.x_access_secret.is_none() { cfg.publish.x_access_secret = env_x_access_secret(); }
        if cfg.publish.slack_webhook_url.is_none() { cfg.publish.slack_webhook_url = env_slack_webhook(); }
        Ok(cfg)
    }
}
