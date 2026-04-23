use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod ai;
mod config;
mod logging;
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
        /// 入力 trends.json (fetch-trends の出力)
        #[arg(long)]
        from: PathBuf,
        /// 出力ディレクトリ (デフォルト: trends.json と同じ場所)
        #[arg(long)]
        out: Option<PathBuf>,
        /// 処理する上位件数 (デフォルト: 全件)
        #[arg(long)]
        limit: Option<usize>,
        /// ドライラン (AI 呼び出しせずスタブ生成)
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
    }

    Ok(())
}

fn resolve_out(opt: Option<PathBuf>) -> PathBuf {
    opt.unwrap_or_else(|| {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        PathBuf::from("drafts").join(today)
    })
}
