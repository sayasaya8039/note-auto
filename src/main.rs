use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

// Windows: HeapAlloc の MT スループット劣化を回避するため mimalloc を採用。
// fetch_all の 8 ソース並列パース + scoring の HashMap/HashSet で
// String allocation が集中するため 5〜15% 全体スループット改善見込み。
#[cfg(windows)]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod ai;
mod config;
mod daemon;
mod display;
mod history;
mod logging;
mod publish;
mod scoring;
mod trends;
mod util;
mod writer;

#[derive(Parser)]
#[command(name = "note-auto", version, about = "note記事トレンドドリブン自動生成")]
struct Cli {
    #[arg(long, default_value = "config.toml", global = true)]
    config: PathBuf,

    /// `note|x|google|hn|konbini|hyakkin|gnews|all` のいずれかを指定すると
    /// `--config configs/<name>.toml` を上書き設定する (`all` のみルート `config.toml`)。
    /// bat shim から呼ばれるためのショートカット。明示 `--config` 指定があれば本フラグが優先。
    #[arg(long, global = true)]
    category: Option<String>,

    /// Unicode 罫線 / glyph を ASCII にフォールバック。旧 cmd.exe や非 UTF-8 環境向け。
    #[arg(long, global = true)]
    ascii: bool,

    /// 色付け制御: `auto` (TTY 検出 + NO_COLOR 尊重) | `always` | `never`
    #[arg(long, global = true, default_value = "auto", value_parser = display::parse_color_mode)]
    color: display::ColorMode,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 4ソース並行でトレンド取得 → スコアリング → drafts/YYYY-MM-DD/trends.json
    FetchTrends {
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value_t = 3)]
        top: usize,
    },
    /// trends.json を入力に AI 執筆 → <slug>.md + <slug>.png を保存
    Write {
        #[arg(long)]
        from: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long)]
        dry_run: bool,
    },
    /// fetch-trends → write を一括実行
    Run {
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value_t = 3)]
        top: usize,
        #[arg(long)]
        dry_run: bool,
    },
    /// articles.json の記事を note 投稿 + X 告知 + Slack 通知
    Publish {
        /// articles.json (writer の出力 manifest)
        #[arg(long)]
        from: PathBuf,
        /// 外部呼び出しをスキップ (配線検証)
        #[arg(long)]
        dry_run: bool,
    },
    /// Slack Webhook に任意メッセージを送信 (配線確認用)
    Notify {
        /// 本文
        #[arg(long, default_value = "note-auto 配線確認 📡")]
        message: String,
    },
    /// X に任意テキストを投稿 (OAuth 疎通確認用)
    XTest {
        #[arg(long, default_value = "note-auto OAuth test ✅")]
        message: String,
    },
    /// 常駐モード — cron (default 07:00 JST) で fetch→write→publish→notify を発火
    Daemon,
    /// fetch→write→publish→notify を即座に1回だけ実行 (cron 待たずに)
    Once {
        #[arg(long)]
        top: Option<usize>,
        #[arg(long)]
        dry_run: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // テーマ初期化 (NO_COLOR / --ascii / --color の解決)
    let theme = display::Theme::init(display::ThemeOptions {
        force_ascii: cli.ascii,
        color: cli.color,
    });
    logging::init_with(&theme);

    // --category 指定時は --config を上書き
    let config_path = match cli.category.as_deref() {
        Some("all") => PathBuf::from("config.toml"),
        Some(cat) => PathBuf::from("configs").join(format!("{cat}.toml")),
        None => cli.config.clone(),
    };

    let mut cfg = config::Config::load(&config_path)?;
    display::print_banner(&theme, env!("CARGO_PKG_VERSION"));
    tracing::info!("note-auto v{} 起動", env!("CARGO_PKG_VERSION"));

    match cli.command {
        Command::FetchTrends { out, top } => {
            let out_dir = resolve_out(out);
            std::fs::create_dir_all(&out_dir)?;
            let items = trends::fetch_all(&cfg).await?;
            let selected = scoring::select_top(items, top, &cfg.scoring);
            let out_path = out_dir.join("trends.json");
            std::fs::write(&out_path, serde_json::to_string_pretty(&selected)?)?;
            tracing::info!(path = %out_path.display(), count = selected.len(), "trends.json を出力");
            display::print_check(&theme, &format!("{} ({}件)", out_path.display(), selected.len()));
        }
        Command::Write { from, out, limit, dry_run } => {
            if dry_run { cfg.writer.dry_run = true; }
            let txt = std::fs::read_to_string(&from)
                .with_context(|| format!("read {}", from.display()))?;
            let mut trends: Vec<scoring::SelectedTrend> = serde_json::from_str(&txt)
                .with_context(|| "parse trends.json")?;
            if let Some(n) = limit { trends.truncate(n); }
            let out_dir = out.unwrap_or_else(|| {
                from.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
            });
            let written = writer::run(&cfg, &trends, &out_dir).await?;
            display::print_check(&theme, &format!("{} 記事を出力", written.len()));
            for a in &written {
                println!("  - {} ({}文字) → {}", a.title, a.char_count, a.md_path.display());
            }
        }
        Command::Run { out, top, dry_run } => {
            if dry_run { cfg.writer.dry_run = true; }
            let out_dir = resolve_out(out);
            std::fs::create_dir_all(&out_dir)?;
            let items = trends::fetch_all(&cfg).await?;
            let selected = scoring::select_top(items, top, &cfg.scoring);
            let trends_path = out_dir.join("trends.json");
            std::fs::write(&trends_path, serde_json::to_string_pretty(&selected)?)?;
            tracing::info!(count = selected.len(), "trends selected");

            let written = writer::run(&cfg, &selected, &out_dir).await?;
            display::print_check(&theme, &format!("{} 記事を出力 → {}", written.len(), out_dir.display()));
            for a in &written {
                println!("  - {} ({}文字) → {}", a.title, a.char_count, a.md_path.display());
            }
        }
        Command::Publish { from, dry_run } => {
            if dry_run { cfg.publish.dry_run = true; }
            let txt = std::fs::read_to_string(&from)
                .with_context(|| format!("read {}", from.display()))?;
            let articles: Vec<writer::WrittenArticle> = serde_json::from_str(&txt)
                .with_context(|| "parse articles.json")?;
            let start = std::time::Instant::now();
            let results = publish::publish_all(&cfg, &articles).await?;
            let total_chars: usize = articles.iter().map(|a| a.char_count).sum();
            let summary = publish::RunSummary {
                date: chrono::Local::now().format("%Y-%m-%d").to_string(),
                articles: results,
                total_chars,
                duration_secs: start.elapsed().as_secs(),
            };
            publish::notify_summary(&cfg, &summary).await.ok();
            display::print_done(&theme, &summary);
            display::print_check(&theme, &format!("publish完了: note={}/X={}/Slack=送信",
                summary.articles.iter().filter(|a| a.note_status == "published" || a.note_status == "draft").count(),
                summary.articles.iter().filter(|a| a.x_status == "posted").count(),
            ));
        }
        Command::Notify { message } => {
            let summary = publish::RunSummary {
                date: chrono::Local::now().format("%Y-%m-%d").to_string(),
                articles: vec![publish::PublishResult {
                    slug: "test".into(),
                    title: message.clone(),
                    note_url: None,
                    note_status: "skipped".into(),
                    x_tweet_url: None,
                    x_status: "skipped".into(),
                    errors: vec![],
                }],
                total_chars: 0,
                duration_secs: 0,
            };
            publish::notify_summary(&cfg, &summary).await?;
            display::print_check(&theme, "Slack Webhook に送信");
        }
        Command::XTest { message } => {
            let url = publish::x_post::post_text(&cfg, &message).await?;
            display::print_check(&theme, &format!("X 投稿完了: {}", url));
        }
        Command::Daemon => {
            daemon::run_daemon(cfg).await?;
        }
        Command::Once { top, dry_run } => {
            if dry_run {
                cfg.writer.dry_run = true;
                cfg.publish.dry_run = true;
            }
            if let Some(n) = top { cfg.schedule.daily_top = n; }
            let summary = daemon::execute_cycle(&cfg).await?;
            publish::notify_summary(&cfg, &summary).await.ok();
            display::print_done(&theme, &summary);
        }
    }

    Ok(())
}

fn resolve_out(opt: Option<PathBuf>) -> PathBuf {
    opt.unwrap_or_else(|| {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        PathBuf::from("drafts").join(today)
    })
}
