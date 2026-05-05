//! CLI サブモジュール群。
//!
//! v0.8.0 W7 で `tui` サブコマンド (ratatui ベース)を `--features tui` で追加。

#[cfg(feature = "tui")]
pub mod tui;
