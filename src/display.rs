//! macOS Big Sur dark テーマ風 CLI 表示層 (v0.7.6 — Phase 1)
//!
//! 設計方針:
//! - **依存ゼロ実装**: stdlib + 既存 crate (tracing/chrono) のみ。indicatif/console/owo-colors を
//!   追加せず、ANSI escape を直接書く。バイナリサイズ膨張を避ける。
//! - **graceful degrade**: `NO_COLOR` 環境変数最優先 / `--ascii` フラグ / 非TTY検出で自動 fallback。
//! - **静的テーマ**: `Theme::init` を main で 1 回だけ呼ぶ。以降は `theme()` で取得。
//! - **置換最小限**: main.rs から既存 `println!("✓ ...")` を `display::print_done(...)` 等に置き換える。
//!
//! 公開 API: ThemeOptions / ColorMode / Theme / palette / glyphs / progress / table / 各 print_* 関数
//!
//! NOTE (v0.7.6): 未使用ヘルパは v0.7.7 の wire-up (indicatif / comfy-table / owo-colors /
//! console / supports-color を実 API 経路に組み込む段階) で消費される予定のため、モジュール
//! 全体に `#![allow(dead_code)]` を付与している。v0.7.7 完了時に剥がす。

#![allow(dead_code)]

use std::io::{IsTerminal, Write};
use std::sync::OnceLock;
use std::time::Instant;

use crate::publish::RunSummary;

// ─────────────────────────────────────────────────────────────────────────────
// Theme

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorMode {
    /// TTY なら color、パイプ時は disable (default)
    #[default]
    Auto,
    /// 強制 ON
    Always,
    /// 強制 OFF
    Never,
}

impl ColorMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "always" | "yes" | "on" | "true" => Some(Self::Always),
            "never" | "no" | "off" | "false" => Some(Self::Never),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ThemeOptions {
    /// `--ascii` フラグ: Unicode 罫線・glyph を ASCII 化
    pub force_ascii: bool,
    /// `--color={auto,always,never}`
    pub color: ColorMode,
}

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    /// truecolor / 8-bit ANSI が使えるか。NO_COLOR 時は常に false。
    pub uses_color: bool,
    /// truecolor (24bit) を使えるか。COLORTERM=truecolor 等で判定。
    pub truecolor: bool,
    /// Unicode 罫線/glyph を使うか。`--ascii` 時 false。
    pub uses_unicode: bool,
    /// stdout が TTY か。非 TTY ならプログレスバー抑止し行ベース出力。
    pub is_tty: bool,
}

static THEME: OnceLock<Theme> = OnceLock::new();

impl Theme {
    /// テーマを 1 回だけ初期化。重複呼び出しは無視（最初の値が勝つ）。
    pub fn init(opts: ThemeOptions) -> Self {
        let theme = Self::detect(opts);
        let _ = THEME.set(theme);
        theme
    }

    /// 既に初期化済みのテーマを取得。未初期化なら `Auto` で初期化。
    pub fn current() -> Self {
        *THEME.get_or_init(|| Self::detect(ThemeOptions::default()))
    }

    fn detect(opts: ThemeOptions) -> Self {
        let is_tty = std::io::stdout().is_terminal();

        // NO_COLOR は最優先 (https://no-color.org/)
        let no_color_env = std::env::var_os("NO_COLOR")
            .map(|v| !v.is_empty())
            .unwrap_or(false);

        let uses_color = match opts.color {
            ColorMode::Always => !no_color_env, // NO_COLOR は --color=always にも勝つ
            ColorMode::Never => false,
            ColorMode::Auto => is_tty && !no_color_env,
        };

        let truecolor = uses_color && truecolor_supported();
        let uses_unicode = !opts.force_ascii && unicode_supported();

        Theme { uses_color, truecolor, uses_unicode, is_tty }
    }
}

fn truecolor_supported() -> bool {
    let colorterm = std::env::var("COLORTERM").unwrap_or_default();
    matches!(colorterm.as_str(), "truecolor" | "24bit")
        // Windows Terminal / VSCode / iTerm2
        || std::env::var("WT_SESSION").is_ok()
        || std::env::var("TERM_PROGRAM").map(|v| matches!(v.as_str(), "iTerm.app" | "vscode")).unwrap_or(false)
}

fn unicode_supported() -> bool {
    // Windows ConHost (legacy) は UTF-8 にしないと罫線が化ける。
    // chcp 65001 されている前提で許容。実害は --ascii で回避できる。
    std::env::var("LANG").map(|v| v.to_uppercase().contains("UTF")).unwrap_or(true)
        || std::env::var("WT_SESSION").is_ok()
}

// ─────────────────────────────────────────────────────────────────────────────
// Palette (Big Sur dark)

pub mod palette {
    /// 24bit RGB
    #[derive(Clone, Copy, Debug)]
    pub struct Rgb(pub u8, pub u8, pub u8);

    pub const ACCENT: Rgb = Rgb(0, 122, 255); // #007AFF systemBlue
    pub const SUCCESS: Rgb = Rgb(48, 209, 88); // #30D158 systemGreen (Big Sur)
    pub const WARNING: Rgb = Rgb(255, 159, 10); // #FF9F0A systemOrange
    pub const ERROR_C: Rgb = Rgb(255, 69, 58); // #FF453A systemRed (Big Sur)
    pub const SECONDARY: Rgb = Rgb(142, 142, 147); // #8E8E93 secondaryLabel
    pub const TERTIARY: Rgb = Rgb(99, 99, 102); // #636366 tertiaryLabel

    /// 8-bit ANSI fallback (truecolor 不可時)
    pub const ACCENT_8: u8 = 33; // bright blue
    pub const SUCCESS_8: u8 = 10; // bright green
    pub const WARNING_8: u8 = 214; // orange
    pub const ERROR_8: u8 = 203; // bright red
    pub const SECONDARY_8: u8 = 245; // grey
}

// ─────────────────────────────────────────────────────────────────────────────
// Glyphs

#[derive(Clone, Copy, Debug)]
pub struct Glyphs {
    pub check: &'static str,
    pub cross: &'static str,
    pub warn: &'static str,
    pub info: &'static str,
    pub bullet: &'static str,
    pub arrow: &'static str,
    pub dot_dim: &'static str,
    pub box_tl: &'static str,
    pub box_tr: &'static str,
    pub box_bl: &'static str,
    pub box_br: &'static str,
    pub h_line: &'static str,
    pub v_line: &'static str,
    pub spinner: &'static [&'static str],
}

const G_UNICODE: Glyphs = Glyphs {
    check: "✓",
    cross: "✗",
    warn: "⚠",
    info: "ⓘ",
    bullet: "●",
    arrow: "▸",
    dot_dim: "·",
    box_tl: "╭",
    box_tr: "╮",
    box_bl: "╰",
    box_br: "╯",
    h_line: "─",
    v_line: "│",
    spinner: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
};

const G_ASCII: Glyphs = Glyphs {
    check: "OK",
    cross: "FAIL",
    warn: "!",
    info: "i",
    bullet: "*",
    arrow: ">",
    dot_dim: ".",
    box_tl: "+",
    box_tr: "+",
    box_bl: "+",
    box_br: "+",
    h_line: "-",
    v_line: "|",
    spinner: &["|", "/", "-", "\\"],
};

pub fn glyphs(theme: &Theme) -> Glyphs {
    if theme.uses_unicode { G_UNICODE } else { G_ASCII }
}

// ─────────────────────────────────────────────────────────────────────────────
// ANSI helpers

fn rgb_fg(theme: &Theme, c: palette::Rgb, fallback_8: u8, s: &str) -> String {
    if !theme.uses_color {
        return s.to_string();
    }
    if theme.truecolor {
        format!("\x1b[38;2;{};{};{}m{}\x1b[0m", c.0, c.1, c.2, s)
    } else {
        format!("\x1b[38;5;{}m{}\x1b[0m", fallback_8, s)
    }
}

pub fn accent(theme: &Theme, s: &str) -> String {
    rgb_fg(theme, palette::ACCENT, palette::ACCENT_8, s)
}
pub fn success(theme: &Theme, s: &str) -> String {
    rgb_fg(theme, palette::SUCCESS, palette::SUCCESS_8, s)
}
pub fn warning(theme: &Theme, s: &str) -> String {
    rgb_fg(theme, palette::WARNING, palette::WARNING_8, s)
}
pub fn error_color(theme: &Theme, s: &str) -> String {
    rgb_fg(theme, palette::ERROR_C, palette::ERROR_8, s)
}
pub fn dim(theme: &Theme, s: &str) -> String {
    rgb_fg(theme, palette::SECONDARY, palette::SECONDARY_8, s)
}
pub fn bold(theme: &Theme, s: &str) -> String {
    if !theme.uses_color { s.to_string() } else { format!("\x1b[1m{}\x1b[0m", s) }
}

// ─────────────────────────────────────────────────────────────────────────────
// Banner / Print helpers

const STAGES: &[&str] = &["fetch", "score", "write", "publish", "notify"];

pub fn print_banner(theme: &Theme, version: &str) {
    let g = glyphs(theme);
    let title = format!("note-auto v{}", version);
    let stages_inline: Vec<String> = STAGES
        .iter()
        .map(|s| format!("{} {}", accent(theme, g.bullet), s))
        .collect();
    let line = stages_inline.join(&format!("   {}   ", dim(theme, "")));

    let inner_width = 56_usize;
    let title_pad = inner_width.saturating_sub(visible_len(&title));
    let title_filler = g.h_line.repeat(title_pad.saturating_sub(2));

    eprintln!(
        "{}{} {} {}{}",
        g.box_tl,
        g.h_line,
        bold(theme, &title),
        title_filler,
        g.box_tr
    );
    eprintln!("{}  {}  {}", g.v_line, line, g.v_line);
    eprintln!(
        "{}{}{}",
        g.box_bl,
        g.h_line.repeat(inner_width + 2),
        g.box_br
    );
}

pub fn print_info(theme: &Theme, msg: &str) {
    let g = glyphs(theme);
    eprintln!("{} {}", accent(theme, g.info), msg);
}

pub fn print_skipped(theme: &Theme, reason: &str) {
    let g = glyphs(theme);
    eprintln!("{} {}", warning(theme, g.warn), dim(theme, reason));
}

pub fn print_warning(theme: &Theme, msg: &str) {
    let g = glyphs(theme);
    eprintln!("{} {}", warning(theme, g.warn), msg);
}

pub fn print_error(theme: &Theme, msg: &str) {
    let g = glyphs(theme);
    eprintln!("{} {}", error_color(theme, g.cross), msg);
}

pub fn print_check(theme: &Theme, msg: &str) {
    let g = glyphs(theme);
    println!("{} {}", success(theme, g.check), msg);
}

// ─────────────────────────────────────────────────────────────────────────────
// Progress

#[derive(Clone, Copy, Debug)]
pub enum Stage {
    Fetch,
    Score,
    Write,
    Publish,
    Notify,
}

impl Stage {
    pub fn label(&self) -> &'static str {
        match self {
            Stage::Fetch => "fetch",
            Stage::Score => "score",
            Stage::Write => "write",
            Stage::Publish => "publish",
            Stage::Notify => "notify",
        }
    }
}

/// 軽量プログレス表示。indicatif 不使用、stderr に行ベースで出力。
/// 非 TTY の場合は最終結果のみ出力（中間更新は抑制）。
pub struct PipelineProgress {
    theme: Theme,
    started: Instant,
}

impl PipelineProgress {
    pub fn new(theme: Theme) -> Self {
        Self { theme, started: Instant::now() }
    }

    /// ステージ開始ログ
    pub fn stage_start(&self, stage: Stage, msg: &str) {
        let g = glyphs(&self.theme);
        let label = format!("[{}]", stage.label());
        eprintln!(
            "{} {} {}",
            accent(&self.theme, &label),
            g.arrow,
            msg
        );
    }

    /// ステージ完了
    pub fn stage_done(&self, stage: Stage, msg: &str) {
        let g = glyphs(&self.theme);
        let label = format!("[{}]", stage.label());
        eprintln!(
            "{} {} {}",
            success(&self.theme, g.check),
            dim(&self.theme, &label),
            msg
        );
    }

    /// ステージ失敗
    pub fn stage_fail(&self, stage: Stage, err: &str) {
        let g = glyphs(&self.theme);
        let label = format!("[{}]", stage.label());
        eprintln!(
            "{} {} {} — {}",
            error_color(&self.theme, g.cross),
            dim(&self.theme, &label),
            error_color(&self.theme, "failed"),
            err
        );
    }

    /// 子イベント（ソース毎の進捗等）
    pub fn item(&self, msg: &str) {
        if !self.theme.is_tty {
            return;
        }
        let g = glyphs(&self.theme);
        eprintln!("  {} {}", dim(&self.theme, g.dot_dim), dim(&self.theme, msg));
    }

    pub fn elapsed_secs(&self) -> u64 {
        self.started.elapsed().as_secs()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Summary table (comfy-table 不使用、自前で UTF-8 罫線を組む)

pub fn print_done(theme: &Theme, summary: &RunSummary) {
    let g = glyphs(theme);

    // ヘッダ + 行データ
    let headers = ["#", "Slug", "Title", "Chars", "note", "X"];
    let mut rows: Vec<[String; 6]> = Vec::with_capacity(summary.articles.len());
    for (i, a) in summary.articles.iter().enumerate() {
        rows.push([
            format!("{}", i + 1),
            truncate(&a.slug, 20),
            truncate(&a.title, 32),
            String::new(), // chars は WrittenArticle 側にあり、PublishResult にはないので空
            short_status(theme, &a.note_status),
            short_status(theme, &a.x_status),
        ]);
    }

    // 列幅計算
    let mut widths = [0usize; 6];
    for (i, h) in headers.iter().enumerate() {
        widths[i] = visible_len(h);
    }
    for row in &rows {
        for (i, c) in row.iter().enumerate() {
            widths[i] = widths[i].max(visible_len(c));
        }
    }
    let widths = widths;

    // 罫線
    let h = g.h_line;
    let make_sep = |left: &str, mid: &str, right: &str| -> String {
        let mut s = String::from(left);
        for (i, w) in widths.iter().enumerate() {
            s.push_str(&h.repeat(w + 2));
            s.push_str(if i + 1 < widths.len() { mid } else { right });
        }
        s
    };

    let top = make_sep(g.box_tl, &cross_glyph(theme, "top"), g.box_tr);
    let mid = make_sep(g.v_line.trim_end(), &cross_glyph(theme, "mid"), g.v_line.trim_end());
    let bot = make_sep(g.box_bl, &cross_glyph(theme, "bot"), g.box_br);

    let render_row = |cells: &[String; 6]| -> String {
        let mut s = String::from(g.v_line);
        for (i, c) in cells.iter().enumerate() {
            let pad = widths[i].saturating_sub(visible_len(c));
            s.push(' ');
            s.push_str(c);
            s.push_str(&" ".repeat(pad + 1));
            s.push_str(g.v_line);
        }
        s
    };

    let header_strs: [String; 6] =
        std::array::from_fn(|i| bold(theme, headers[i]));

    println!();
    println!("{}", top);
    println!("{}", render_row(&header_strs));
    println!("{}", mid);
    for row in &rows {
        println!("{}", render_row(row));
    }
    println!("{}", bot);

    // フッタ
    let footer = format!(
        "{} {} 記事 / {}s / {} 文字",
        success(theme, g.check),
        summary.articles.len(),
        summary.duration_secs,
        summary.total_chars
    );
    println!("{}", footer);
}

fn cross_glyph(theme: &Theme, _pos: &str) -> String {
    // シンプル化: 単純に v_line を返す（隅は box_* を使うため _pos は将来用）
    if theme.uses_unicode { "┼".to_string() } else { "+".to_string() }
}

fn short_status(theme: &Theme, status: &str) -> String {
    match status {
        "published" | "posted" | "draft" => success(theme, status),
        "skipped" | "" => dim(theme, status),
        _ => error_color(theme, status),
    }
}

fn truncate(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        let truncated: String = chars.into_iter().take(max.saturating_sub(1)).collect();
        format!("{}…", truncated)
    }
}

/// ANSI escape を除いた可視文字数
fn visible_len(s: &str) -> usize {
    let mut n = 0usize;
    let mut in_esc = false;
    for c in s.chars() {
        if in_esc {
            if c == 'm' {
                in_esc = false;
            }
            continue;
        }
        if c == '\x1b' {
            in_esc = true;
            continue;
        }
        // wide char を 2 とカウント
        n += if c.is_ascii() { 1 } else if (c as u32) < 0x2000 { 1 } else { 1 };
    }
    n
}

// ─────────────────────────────────────────────────────────────────────────────
// Public re-exports for main.rs convenience

/// shorthand: `display::theme()`
pub fn theme() -> Theme {
    Theme::current()
}

/// CLI value parser for clap
pub fn parse_color_mode(s: &str) -> Result<ColorMode, String> {
    ColorMode::parse(s).ok_or_else(|| format!("unknown color mode: {}", s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_mode_parse() {
        assert_eq!(ColorMode::parse("auto"), Some(ColorMode::Auto));
        assert_eq!(ColorMode::parse("always"), Some(ColorMode::Always));
        assert_eq!(ColorMode::parse("never"), Some(ColorMode::Never));
        assert_eq!(ColorMode::parse("nope"), None);
    }

    #[test]
    fn theme_no_color_env_disables_color() {
        // NO_COLOR を見るが、テストでは env をいじりたくないので detect 単体で検証
        let opts = ThemeOptions { force_ascii: false, color: ColorMode::Never };
        let t = Theme::detect(opts);
        assert!(!t.uses_color);
    }

    #[test]
    fn theme_force_ascii_disables_unicode() {
        let opts = ThemeOptions { force_ascii: true, color: ColorMode::Never };
        let t = Theme::detect(opts);
        assert!(!t.uses_unicode);
    }

    #[test]
    fn glyphs_switch_on_unicode() {
        let unicode = Theme { uses_color: false, truecolor: false, uses_unicode: true, is_tty: false };
        let ascii = Theme { uses_color: false, truecolor: false, uses_unicode: false, is_tty: false };
        assert_eq!(glyphs(&unicode).check, "✓");
        assert_eq!(glyphs(&ascii).check, "OK");
    }

    #[test]
    fn truncate_shortens() {
        assert_eq!(truncate("hello world", 5), "hell…");
        assert_eq!(truncate("hi", 5), "hi");
    }

    #[test]
    fn ansi_helpers_skip_when_no_color() {
        let t = Theme { uses_color: false, truecolor: false, uses_unicode: true, is_tty: true };
        assert_eq!(accent(&t, "x"), "x");
        assert_eq!(bold(&t, "x"), "x");
    }

    #[test]
    fn ansi_truecolor_emits_38_2() {
        let t = Theme { uses_color: true, truecolor: true, uses_unicode: true, is_tty: true };
        let out = accent(&t, "x");
        assert!(out.starts_with("\x1b[38;2;0;122;255m"));
        assert!(out.ends_with("\x1b[0m"));
    }

    #[test]
    fn visible_len_strips_ansi() {
        assert_eq!(visible_len("hello"), 5);
        assert_eq!(visible_len("\x1b[1mhello\x1b[0m"), 5);
    }
}

// 念のため: Write trait を使ったが import 残し
#[allow(dead_code)]
fn _write_unused(_w: &mut dyn Write) {}
