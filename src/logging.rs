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
//! 将来 (Phase 2): `tracing-appender` 追加で hourly rolling JSON file を別レイヤとして並走させる。
//! 現状は依存ゼロを優先し、bat の `2>&1` リダイレクトで代替する。

use tracing_subscriber::{fmt, fmt::format::FmtSpan, prelude::*, EnvFilter};

use crate::display::Theme;

/// 後方互換用。テーマ未指定時はデフォルト検出 (`Theme::current()`) を使う。
/// 現状 `main.rs` は `init_with(&theme)` を直接呼ぶため未使用、外部呼び出し向けに保持。
#[allow(dead_code)]
pub fn init() {
    init_with(&Theme::current());
}

/// テーマを反映した初期化。`main` で `Theme::init()` 後に呼ぶ。
pub fn init_with(theme: &Theme) {
    // 二重初期化はサイレントに無視 (テスト時など)
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,note_auto=debug"));

    // L12 (W7-C): span CLOSE 時に elapsed (time.busy / time.idle) を log に出力。
    // lowlevel PR-I で導入された 4 stage tracing::info_span! の経過時間を観測可能にする。
    // 例: `INFO close fetch{source_count=11}: time.busy=12.3s`
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
