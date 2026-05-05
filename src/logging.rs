//! note-auto ロギング初期化
//!
//! 設計 (v0.7.6 — Phase 1):
//! - **stdout**: `display::print_*` 系のユーザ向け出力専用。tracing は流さない。
//! - **stderr**: `tracing` イベント (info+) を compact 形式で出力。
//!   既存 `run-*.bat` は `>> %LOGFILE% 2>&1` で stderr もログファイルに取り込むため、
//!   結果として "人間向け stdout" と "機械向け stderr→log" の二系統が成立する。
//! - **ANSI**: `Theme.uses_color` を尊重して `with_ansi()` を切り替える。
//! - **互換**: 引数なしの `init()` も残す (既存呼び出し位置の後方互換)。
//!
//! v0.9.0 W7-D: `init_with_progress(&Theme, MultiProgress)` 追加。
//! `MultiProgress::suspend` 経由の MakeWriter で tracing 出力時に progress bar を
//! 一時退避 → log と bar の描画競合を解消する。

use tracing_subscriber::{fmt, fmt::format::FmtSpan, prelude::*, EnvFilter};

use crate::display::Theme;

/// 後方互換用。テーマ未指定時はデフォルト検出 (`Theme::current()`) を使う。
/// 現状 `main.rs` は `init_with(&theme)` を直接呼ぶため未使用、外部呼び出し向けに保持。
#[allow(dead_code)]
pub fn init() {
    init_with(&Theme::current());
}

/// テーマを反映した初期化。`main` で `Theme::init()` 後に呼ぶ。
/// 出力先は stderr 直書き (MultiProgress 連携なし、CLI/TUI スタートアップ時のフォールバック)。
pub fn init_with(theme: &Theme) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,note_auto=debug"));

    let stderr_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_ansi(theme.uses_color)
        .with_span_events(FmtSpan::CLOSE)
        .compact();

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(stderr_layer)
        .try_init();
}

/// W7-D: `MultiProgress` を渡して、tracing 出力時に progress bar を suspend する初期化。
///
/// `IndicatifBackend::new` から呼ばれ、tracing イベント発火時に
/// `MultiProgress::suspend(|| eprint!(...))` を経由することで bar の cursor 制御と
/// 競合せず log を出せる。tracing と log エコシステムは結合しないため、
/// indicatif-log-bridge は不要 (自前 MakeWriter で完結)。
///
/// L12 互換: `with_span_events(FmtSpan::CLOSE)` で span CLOSE 時の elapsed も継続出力。
pub fn init_with_progress(theme: &Theme, multi: indicatif::MultiProgress) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,note_auto=debug"));

    let suspend_writer = MultiProgressWriter { multi };

    let layer = fmt::layer()
        .with_writer(suspend_writer)
        .with_target(false)
        .with_ansi(theme.uses_color)
        .with_span_events(FmtSpan::CLOSE)
        .compact();

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(layer)
        .try_init();
}

/// `MultiProgress::suspend` 経由で stderr に書き込む `MakeWriter` 実装。
///
/// tracing-subscriber は event 発火ごとに `make_writer()` を呼んで新しい
/// `Writer` を取得 → `Write::write_all` でバイト列を渡す。
/// 一旦 buffer に溜めて、`Drop` 時に `MultiProgress::suspend` で進捗を退避し、
/// stderr へまとめて書き出す。
#[derive(Clone)]
struct MultiProgressWriter {
    multi: indicatif::MultiProgress,
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for MultiProgressWriter {
    type Writer = SuspendingWriter;
    fn make_writer(&'a self) -> Self::Writer {
        SuspendingWriter {
            multi: self.multi.clone(),
            buf: Vec::with_capacity(256),
        }
    }
}

struct SuspendingWriter {
    multi: indicatif::MultiProgress,
    buf: Vec<u8>,
}

impl std::io::Write for SuspendingWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let chunk = std::mem::take(&mut self.buf);
        // suspend 中に各 ProgressBar の描画を消去 → クロージャ内で stderr 書込 → 復帰
        // UFCS で Write::write_all を呼ぶ (use 不要)
        self.multi.suspend(|| {
            let mut stderr = std::io::stderr();
            let _ = std::io::Write::write_all(&mut stderr, &chunk);
        });
        Ok(())
    }
}

impl Drop for SuspendingWriter {
    fn drop(&mut self) {
        let _ = std::io::Write::flush(self);
    }
}
