use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod ai;
mod config;
mod daemon;
mod logging;
mod publish;
mod scoring;
mod trends;
mod writer;

#[derive(Parser)]
#[command(name = "note-auto", version, about = "note記事トレンドドリブン自動生成")]
struct Cli {
    #[arg(long, default_value = "config.toml", global = true)]
    config: PathBuf,

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
    logging::init();

    let mut cfg = config::Config::load(&cli.config)?;
    tracing::info!("note-auto v{} 起動", env!("CARGO_PKG_VERSION"));

    match cli.command {
        Command::FetchTrends { out, top } => {
            let out_dir = resolve_out(out);
            std::fs::create_dir_all(&out_dir)?;
            let items = trends::fetch_all(&cfg).await?;
            let selected = scoring::select_top(items, top);
            let out_path = out_dir.join("trends.json");
            std::fs::write(&out_path, serde_json::to_string_pretty(&selected)?)?;
            tracing::info!(path = %out_path.display(), count = selected.len(), "trends.json を出力");
            println!("✓ {} ({}件)", out_path.display(), selected.len());
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
            println!("✓ {} 記事を出力", written.len());
            for a in &written {
                println!("  - {} ({}文字) → {}", a.title, a.char_count, a.md_path.display());
            }
        }
        Command::Run { out, top, dry_run } => {
            if dry_run { cfg.writer.dry_run = true; }
            let out_dir = resolve_out(out);
            std::fs::create_dir_all(&out_dir)?;
            let items = trends::fetch_all(&cfg).await?;
            let selected = scoring::select_top(items, top);
            let trends_path = out_dir.join("trends.json");
            std::fs::write(&trends_path, serde_json::to_string_pretty(&selected)?)?;
            tracing::info!(count = selected.len(), "trends selected");

            let written = writer::run(&cfg, &selected, &out_dir).await?;
            println!("✓ {} 記事を出力 → {}", written.len(), out_dir.display());
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
            println!("✓ publish完了: note={}/X={}/Slack=送信",
                summary.articles.iter().filter(|a| a.note_status == "published" || a.note_status == "draft").count(),
                summary.articles.iter().filter(|a| a.x_status == "posted").count(),
            );
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
            println!("✓ Slack Webhook に送信");
        }
        Command::XTest { message } => {
            let url = publish::x_post::post_text(&cfg, &message).await?;
            println!("✓ X 投稿完了: {}", url);
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
            println!("✓ once完了 ({}記事 / {}s / {}文字)",
                summary.articles.len(), summary.duration_secs, summary.total_chars);
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
