use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod config;
mod logging;
mod scoring;
mod trends;

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
        /// 出力ディレクトリ (デフォルト: drafts/<today>)
        #[arg(long)]
        out: Option<PathBuf>,
        /// 選定数 (デフォルト: 3)
        #[arg(long, default_value_t = 3)]
        top: usize,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    logging::init();

    let cfg = config::Config::load(&cli.config)?;
    tracing::info!("note-auto v{} 起動", env!("CARGO_PKG_VERSION"));

    match cli.command {
        Command::FetchTrends { out, top } => {
            let out_dir = match out {
                Some(p) => p,
                None => {
                    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                    PathBuf::from("drafts").join(today)
                }
            };
            std::fs::create_dir_all(&out_dir)?;
            let items = trends::fetch_all(&cfg).await?;
            let selected = scoring::select_top(items, top);
            let out_path = out_dir.join("trends.json");
            std::fs::write(&out_path, serde_json::to_string_pretty(&selected)?)?;
            tracing::info!(path = %out_path.display(), count = selected.len(), "trends.json を出力");
            println!("✓ {} ({}件)", out_path.display(), selected.len());
        }
    }

    Ok(())
}
