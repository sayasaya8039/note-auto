//! macOS Big Sur dark テーマ風 CLI 表示層 (v0.7.7 — Phase 1.5-A)
//!
//! 設計方針 (v0.7.7-A 改訂):
//! - **`console` + `supports-color` で TTY/COLOR 検出統一**: 自前環境変数判定を排除し、
//!   `console::Term` (Windows ConHost / Windows Terminal / Unix tty 等) と
//!   `supports-color` (NO_COLOR / FORCE_COLOR / COLORTERM / WT_SESSION 一括判定) を採用。
//! - **`owo-colors` で truecolor 着色**: ANSI escape の直書きを `OwoColorize::truecolor` に置換。
//!   8-bit (256-color) fallback は ID 動的指定する API が owo-colors に無いため ANSI 直書きを残す。
//! - **API 互換 100%**: `Theme::init` / `Theme::current` / `glyphs` / `accent` 等の公開シグネチャは
//!   v0.7.6 から完全維持。内部実装のみ差替。main.rs の呼び出しは無変更で動く。
//! - **NO_COLOR 最優先 / --ascii フラグ / 非TTY検出 で自動 fallback** は維持。
//!
//! 公開 API: ThemeOptions / ColorMode / Theme / palette / glyphs / progress / table / 各 print_* 関数
//!
//! NOTE (v0.7.7-C): Phase C 完了時に `#![allow(dead_code)]` を剥がした。
//! 残存する未使用ヘルパ (`print_info` / `print_warning` / `print_error` 等) は
//! 個別に `#[allow(dead_code)]` を付ける方針。将来 main から呼ぶ可能性があるため
//! 残置。

use std::sync::OnceLock;

use crate::publish::RunSummary;

// W4 console + W5 supports-color
use console::Term;
use supports_color::Stream as ColorStream;

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
        // W4: console::Term で TTY 検出統一 (Windows ConHost / Windows Terminal / Unix tty)
        let term = Term::stdout();
        let is_tty = term.is_term();

        // NO_COLOR 最優先 (https://no-color.org/)。supports-color も内部処理するが念のため明示。
        let no_color_env = std::env::var_os("NO_COLOR")
            .map(|v| !v.is_empty())
            .unwrap_or(false);

        // W5: supports-color で COLORTERM / FORCE_COLOR / TTY / CI 環境を一括判定
        let color_support = supports_color::on(ColorStream::Stdout);

        let uses_color = match opts.color {
            // --color=always: NO_COLOR が立っていれば常に false (NO_COLOR 最優先原則)
            ColorMode::Always => !no_color_env,
            ColorMode::Never => false,
            // Auto: supports-color が判定 (NO_COLOR / FORCE_COLOR / TTY / CI 自動考慮)
            ColorMode::Auto => color_support.is_some() && !no_color_env,
        };

        let truecolor = uses_color
            && color_support.map(|s| s.has_16m).unwrap_or(false);

        // W4: Unicode サポート判定は permissive default (v0.7.6 互換)。
        // - 第一: console::Term::features().wants_emoji() が true なら確実にサポート
        //   (macOS Terminal / iTerm / Windows Terminal / VS Code 等)
        // - 第二: cfg!(unix) は通常 UTF-8 環境
        // - 第三 (fallback): LANG が UTF-8 系、または unset (default UTF-8 想定)、
        //   または WT_SESSION 存在で楽観的に許可。
        // ConHost 等の非 UTF-8 環境では --ascii を明示する運用 (v0.7.6 と同方針)。
        let uses_unicode = !opts.force_ascii
            && (term.features().wants_emoji()
                || cfg!(unix)
                || std::env::var("LANG")
                    .map(|v| v.to_uppercase().contains("UTF"))
                    .unwrap_or(true)
                || std::env::var_os("WT_SESSION").is_some());

        Theme { uses_color, truecolor, uses_unicode, is_tty }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Palette (Big Sur dark) — W3: (u8,u8,u8) タプルで宣言、owo_colors::OwoColorize::truecolor へ渡す

pub mod palette {
    /// 24bit RGB tuple. owo-colors の `OwoColorize::truecolor(r, g, b)` および
    /// ANSI 256-color (8-bit) 各 fallback に渡す。
    pub const ACCENT:    (u8, u8, u8) = (0, 122, 255);    // #007AFF systemBlue
    pub const SUCCESS:   (u8, u8, u8) = (48, 209, 88);    // #30D158 systemGreen (Big Sur)
    #[allow(dead_code)] // Phase 2 で warning helper に wire 予定
    pub const WARNING:   (u8, u8, u8) = (255, 159, 10);   // #FF9F0A systemOrange
    pub const ERROR_C:   (u8, u8, u8) = (255, 69, 58);    // #FF453A systemRed (Big Sur)
    pub const SECONDARY: (u8, u8, u8) = (142, 142, 147);  // #8E8E93 secondaryLabel
    #[allow(dead_code)] // Phase 2 で 3 階層 dim 表示に wire 予定
    pub const TERTIARY:  (u8, u8, u8) = (99, 99, 102);    // #636366 tertiaryLabel

    /// 8-bit ANSI fallback (truecolor 不可時)。
    /// owo-colors v4 は 256-color ID を動的指定する公開 API が無いため、ID は ANSI 直書きに使う。
    pub const ACCENT_8: u8 = 33;     // bright blue
    pub const SUCCESS_8: u8 = 10;    // bright green
    #[allow(dead_code)] // Phase 2 で warning helper に wire 予定
    pub const WARNING_8: u8 = 214;   // orange
    pub const ERROR_8: u8 = 203;     // bright red
    pub const SECONDARY_8: u8 = 245; // grey
}

// ─────────────────────────────────────────────────────────────────────────────
// Glyphs

#[derive(Clone, Copy, Debug)]
pub struct Glyphs {
    pub check: &'static str,
    pub cross: &'static str,
    /// Phase 2 で warning helper に wire 予定
    #[allow(dead_code)]
    pub warn: &'static str,
    /// Phase 2 で info helper に wire 予定
    #[allow(dead_code)]
    pub info: &'static str,
    pub bullet: &'static str,
    /// Phase 2 で進捗 prefix に wire 予定
    #[allow(dead_code)]
    pub arrow: &'static str,
    /// Phase 2 で sub-item bullet に wire 予定
    #[allow(dead_code)]
    pub dot_dim: &'static str,
    pub box_tl: &'static str,
    pub box_tr: &'static str,
    pub box_bl: &'static str,
    pub box_br: &'static str,
    pub h_line: &'static str,
    pub v_line: &'static str,
    /// Phase 2 で SourceBar/ArticleBar の独自 spinner に wire 予定
    #[allow(dead_code)]
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
// Color helpers (W3: owo-colors 経由)

/// truecolor は owo-colors の `OwoColorize::truecolor` を経由、
/// 8-bit fallback は ID 動的指定 API が owo-colors v4 に無いため ANSI 直書きで残す。
fn rgb_fg(theme: &Theme, rgb: (u8, u8, u8), fallback_8: u8, s: &str) -> String {
    if !theme.uses_color {
        return s.to_string();
    }
    if theme.truecolor {
        use owo_colors::OwoColorize;
        s.truecolor(rgb.0, rgb.1, rgb.2).to_string()
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
/// Phase 2 で `print_warning` 等の wire を増やす際に使う。現状未使用。
#[allow(dead_code)]
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
    if !theme.uses_color {
        s.to_string()
    } else {
        use owo_colors::OwoColorize;
        s.bold().to_string()
    }
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

    // CLI-5 (v0.9.3): terminal 幅に追従。
    // - 取得失敗時は 56 cols (v0.7.6 互換) を fallback。
    // - 上限 120 cols でクリップ (横長 terminal でバナーが冗長化しない)。
    // - 下限はステージ行 (visible_len 約 50) + 余白に配慮して 50 cols。
    // - 実 inner_width = max(stage 行幅, title 幅) を満たす最小値で動的決定。
    let term_width = console::Term::stdout()
        .size_checked()
        .map(|(_h, w)| w as usize)
        .unwrap_or(60);
    let stages_visible = visible_len(&line);
    let title_visible = visible_len(&title);
    let min_required = stages_visible.max(title_visible) + 4; // 内側余白
    let cap = term_width.saturating_sub(2).clamp(50, 120);
    let inner_width = cap.max(min_required).min(120);

    let title_pad = inner_width.saturating_sub(title_visible);
    let title_filler = g.h_line.repeat(title_pad.saturating_sub(2));

    eprintln!(
        "{}{} {} {}{}",
        g.box_tl,
        g.h_line,
        bold(theme, &title),
        title_filler,
        g.box_tr
    );
    let stages_pad = inner_width.saturating_sub(stages_visible).saturating_sub(2);
    eprintln!(
        "{}  {}{}  {}",
        g.v_line,
        line,
        " ".repeat(stages_pad),
        g.v_line
    );
    eprintln!(
        "{}{}{}",
        g.box_bl,
        g.h_line.repeat(inner_width + 2),
        g.box_br
    );
}

/// Phase 2 で wire 予定の info/skipped/warning/error helper。
/// 現状 main.rs / daemon.rs では PipelineProgress を経由して通知しているため未使用。
#[allow(dead_code)]
pub fn print_info(theme: &Theme, msg: &str) {
    let g = glyphs(theme);
    eprintln!("{} {}", accent(theme, g.info), msg);
}

#[allow(dead_code)]
pub fn print_skipped(theme: &Theme, reason: &str) {
    let g = glyphs(theme);
    eprintln!("{} {}", warning(theme, g.warn), dim(theme, reason));
}

#[allow(dead_code)]
pub fn print_warning(theme: &Theme, msg: &str) {
    let g = glyphs(theme);
    eprintln!("{} {}", warning(theme, g.warn), msg);
}

#[allow(dead_code)]
pub fn print_error(theme: &Theme, msg: &str) {
    let g = glyphs(theme);
    eprintln!("{} {}", error_color(theme, g.cross), msg);
}

pub fn print_check(theme: &Theme, msg: &str) {
    let g = glyphs(theme);
    println!("{} {}", success(theme, g.check), msg);
}

// ─────────────────────────────────────────────────────────────────────────────
// Progress (W1: indicatif::MultiProgress 統合)
//
// 5 stage spinner を MultiProgress で並べ、各 stage_start/stage_done/stage_fail で
// 状態遷移する。ProgressBar は Spinner 専用 (count なし)、tick_chars は theme に
// 合わせて Braille / ASCII 切替。非 TTY なら DrawTarget::hidden で完全静音。

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum Stage {
    Fetch = 0,
    Score = 1,
    Write = 2,
    Publish = 3,
    Notify = 4,
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

/// パイプライン進捗の出力先抽象化 (W7-A)。
///
/// `IndicatifBackend` (現行 v0.7.7 互換、stderr に MultiProgress) と
/// 将来追加予定の `TuiBackend` (W7-B、ratatui Frame に mpsc 経由 push) を切替可能にする。
/// `PipelineProgress` は薄い facade として trait オブジェクトを保持し、
/// 既存呼出 (main.rs / daemon.rs / writer.rs) からは backend を意識せず使える。
pub trait PipelineBackend: Send + Sync {
    /// ステージ開始: spinner / 進捗開始
    fn stage_start(&self, stage: Stage, msg: &str);
    /// ステージ完了
    fn stage_done(&self, stage: Stage, msg: &str);
    /// ステージ失敗
    fn stage_fail(&self, stage: Stage, err: &str);
    /// 全 stage 終了処理 (Drop 時にも自動)
    fn finish(&self);

    /// W7-E (v0.9.1): 子 sub-bar (source/article/publish 等) を返す。
    /// IndicatifBackend は indicatif の子 ProgressBar を作成、TuiBackend / その他は default no-op。
    /// `label` は表示用ラベル (例: "hn", "<slug>:write", "<slug>:publish-note")。
    fn sub_bar(&self, _stage: Stage, _label: &str) -> Box<dyn SubBar> {
        Box::new(NoopSubBar)
    }
}

/// W7-E: 子 sub-bar インターフェース。
/// `Drop` 時に自動 finish するが、明示的に `done` / `fail` で締めるのが推奨。
pub trait SubBar: Send + Sync {
    /// メッセージ更新 (進行中の phase 情報等)。
    /// 現状 W7-E では writer/publish/trends は完了/失敗時のみ sub_bar を駆動するため
    /// `tick` は未呼出。Phase 4 で writer 内部の細粒度 phase (LLM/画像/save) wire に使う予定。
    #[allow(dead_code)]
    fn tick(&self, msg: &str);
    /// 完了 — ✓ prefix + finish_with_message
    fn done(&self, msg: &str);
    /// 失敗 — ✗ prefix + abandon_with_message
    fn fail(&self, err: &str);
}

/// no-op SubBar (default backend / TuiBackend で利用)
pub struct NoopSubBar;

impl SubBar for NoopSubBar {
    fn tick(&self, _msg: &str) {}
    fn done(&self, _msg: &str) {}
    fn fail(&self, _err: &str) {}
}

/// indicatif::MultiProgress 経由のパイプライン進捗 backend (v0.7.7 互換)。
///
/// 5 stage (fetch / score / write / publish / notify) の状態遷移を
/// MultiProgress + ProgressBar (Spinner) で可視化。
/// 非 TTY 時は DrawTarget::hidden で完全静音、CI/redirect 出力にゴミを残さない。
pub struct IndicatifBackend {
    multi: indicatif::MultiProgress,
    bars: [indicatif::ProgressBar; 5],
    theme: Theme,
}

impl IndicatifBackend {
    pub fn new(theme: Theme) -> Self {
        use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};

        let g = glyphs(&theme);
        let multi = MultiProgress::new();

        // 非 TTY (パイプ / リダイレクト / CI) は完全静音
        if !theme.is_tty {
            multi.set_draw_target(ProgressDrawTarget::hidden());
        }

        // W7-D: tracing 出力時に MultiProgress::suspend 経由で stderr に書く
        // logging を初期化し直す。既に main.rs::init_with(&theme) で初期化済の場合
        // try_init() がサイレントに失敗するが、その場合は既存の stderr 直書きを維持。
        // → IndicatifBackend が起動経路で最初に作られる場合 (典型的な Cli once / fetch-trends 等)
        //   は MultiProgress 連動 layer が有効になり、log と progress bar の競合が解消する。
        crate::logging::init_with_progress(&theme, multi.clone());

        // tick_strings は theme で切替 (Unicode Braille / ASCII)
        let ticks_unicode: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", "✓"];
        let ticks_ascii: &[&str] = &["|", "/", "-", "\\", "OK"];
        let ticks: &[&str] = if theme.uses_unicode { ticks_unicode } else { ticks_ascii };

        let style = ProgressStyle::with_template("{prefix:<14} {spinner} {wide_msg}")
            .unwrap()
            .tick_strings(ticks);

        let make_bar = |label: &str| -> ProgressBar {
            let pb = multi.add(ProgressBar::new_spinner());
            pb.set_style(style.clone());
            pb.set_prefix(format!("{} {}", accent(&theme, g.bullet), label));
            pb
        };

        let bars = [
            make_bar("fetch"),
            make_bar("score"),
            make_bar("write"),
            make_bar("publish"),
            make_bar("notify"),
        ];

        Self { multi, bars, theme }
    }
}

impl PipelineBackend for IndicatifBackend {
    fn stage_start(&self, stage: Stage, msg: &str) {
        let bar = &self.bars[stage as usize];
        bar.enable_steady_tick(std::time::Duration::from_millis(80));
        bar.set_message(msg.to_string());
    }

    fn stage_done(&self, stage: Stage, msg: &str) {
        let g = glyphs(&self.theme);
        let bar = &self.bars[stage as usize];
        bar.set_prefix(format!("{} {}", success(&self.theme, g.check), stage.label()));
        bar.disable_steady_tick();
        bar.finish_with_message(msg.to_string());
    }

    fn stage_fail(&self, stage: Stage, err: &str) {
        let g = glyphs(&self.theme);
        let bar = &self.bars[stage as usize];
        bar.set_prefix(format!("{} {}", error_color(&self.theme, g.cross), stage.label()));
        bar.disable_steady_tick();
        bar.abandon_with_message(format!("failed: {err}"));
    }

    fn finish(&self) {
        for bar in &self.bars {
            if !bar.is_finished() {
                bar.finish();
            }
        }
        let _ = self.multi.clear();
    }

    fn sub_bar(&self, _stage: Stage, label: &str) -> Box<dyn SubBar> {
        use indicatif::{ProgressBar, ProgressStyle};

        // W7-E: 子 bar は親 stage 直下にインデント表示 ("    └─ <label>")
        let pb = self.multi.add(ProgressBar::new_spinner());
        let style = ProgressStyle::with_template("{prefix:<14} {spinner} {wide_msg}")
            .unwrap()
            .tick_strings(if self.theme.uses_unicode {
                &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", "✓"]
            } else {
                &["|", "/", "-", "\\", "OK"]
            });
        pb.set_style(style);
        pb.set_prefix(format!(
            "  {} {}",
            dim(&self.theme, if self.theme.uses_unicode { "↳" } else { ">" }),
            dim(&self.theme, label)
        ));
        pb.enable_steady_tick(std::time::Duration::from_millis(120));

        Box::new(IndicatifSubBar {
            pb,
            theme: self.theme,
        })
    }
}

/// W7-E: indicatif 子 ProgressBar の SubBar 実装
pub struct IndicatifSubBar {
    pb: indicatif::ProgressBar,
    theme: Theme,
}

impl SubBar for IndicatifSubBar {
    /// Phase 4 で writer/publish 内部の phase 進捗 (LLM 呼出 / 画像生成 / save 等) を細粒度
    /// tick する際に wire 予定。現状 W7-E では done/fail 時にしか sub_bar を駆動しないため未呼出。
    #[allow(dead_code)]
    fn tick(&self, msg: &str) {
        self.pb.set_message(msg.to_string());
    }
    fn done(&self, msg: &str) {
        let g = glyphs(&self.theme);
        // prefix を ✓ に切替てから finish
        let current = self.pb.prefix();
        self.pb.set_prefix(format!(
            "  {} {}",
            success(&self.theme, g.check),
            dim(&self.theme, current.trim_start_matches(|c: char| c.is_whitespace() || c == '↳' || c == '>').trim())
        ));
        self.pb.disable_steady_tick();
        self.pb.finish_with_message(msg.to_string());
    }
    fn fail(&self, err: &str) {
        let g = glyphs(&self.theme);
        let current = self.pb.prefix();
        self.pb.set_prefix(format!(
            "  {} {}",
            error_color(&self.theme, g.cross),
            dim(&self.theme, current.trim_start_matches(|c: char| c.is_whitespace() || c == '↳' || c == '>').trim())
        ));
        self.pb.disable_steady_tick();
        self.pb.abandon_with_message(format!("failed: {err}"));
    }
}

/// パイプライン進捗 facade。`IndicatifBackend` (default) または将来の `TuiBackend` を保持。
///
/// 既存呼出 (`main.rs` / `daemon.rs`) からは `new(theme)` / `stage_start` / `stage_done` /
/// `stage_fail` / `finish` のシグネチャ無変更で動く (v0.7.7 互換)。
pub struct PipelineProgress {
    backend: Box<dyn PipelineBackend>,
}

impl PipelineProgress {
    /// v0.7.7 互換: `IndicatifBackend` を使うコンストラクタ。
    /// (内部は `new_indicatif` への alias)
    pub fn new(theme: Theme) -> Self {
        Self::new_indicatif(theme)
    }

    /// 明示的に indicatif backend を指定 (現状 v0.7.7 と同等動作)。
    pub fn new_indicatif(theme: Theme) -> Self {
        Self { backend: Box::new(IndicatifBackend::new(theme)) }
    }

    /// W7-B: TuiBackend を使うコンストラクタ。`cli::tui` から渡される
    /// `tokio::sync::mpsc::UnboundedSender<PipelineUpdate>` 経由で進捗を ratatui App に push する。
    /// `--features tui` 時のみ利用可能。
    #[cfg(feature = "tui")]
    pub fn new_tui(tx: tokio::sync::mpsc::UnboundedSender<PipelineUpdate>) -> Self {
        Self { backend: Box::new(TuiBackend { tx }) }
    }

    pub fn stage_start(&self, stage: Stage, msg: &str) {
        self.backend.stage_start(stage, msg);
    }

    pub fn stage_done(&self, stage: Stage, msg: &str) {
        self.backend.stage_done(stage, msg);
    }

    pub fn stage_fail(&self, stage: Stage, err: &str) {
        self.backend.stage_fail(stage, err);
    }

    /// W7-E: 子 sub-bar (source/article/publish 個別) を取得。
    /// IndicatifBackend では子 ProgressBar を作成、TuiBackend / その他は no-op。
    pub fn sub_bar(&self, stage: Stage, label: &str) -> Box<dyn SubBar> {
        self.backend.sub_bar(stage, label)
    }

    pub fn finish(&self) {
        self.backend.finish();
    }
}

impl Drop for PipelineProgress {
    fn drop(&mut self) {
        self.finish();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TUI backend (W7-B, --features tui only)
//
// `cli::tui` モジュールが ratatui App を所有し、`PipelineProgress::new_tui(tx)` 経由で
// 起動した worker タスクから stage_start/done/fail を mpsc::UnboundedSender に push する。
// App 側は `UnboundedReceiver<PipelineUpdate>` を select! で受信して state を更新、
// ratatui Frame を再描画する。

/// パイプライン進捗イベント (TUI backend が mpsc 経由で送信)。
/// W7-G (v0.9.2): SubStart/SubDone/SubFail を追加。writer/publish/trends から発火される
/// sub_bar 進捗イベントを TUI App に流し、Pipeline ペインで階層表示する。
#[cfg(feature = "tui")]
#[derive(Clone, Debug)]
pub enum PipelineUpdate {
    StageStart { stage: Stage, msg: String },
    StageDone { stage: Stage, msg: String },
    StageFail { stage: Stage, err: String },
    /// W7-G: sub-bar 開始 (sub_bar() 呼出時、現状未使用だが将来 tick 系拡張時に利用)
    SubStart { stage: Stage, label: String },
    /// W7-G: sub-bar tick (`SubBar::tick(msg)` 経由)。
    /// WPW1 (v0.9.2) で writer 内部 phase wire が完成し、`writer::write_one` から
    /// 各 phase 開始時に発火される (research / brief / draft / image / embed / save)。
    SubTick { stage: Stage, label: String, msg: String },
    /// W7-G: sub-bar 完了 (`SubBar::done(msg)`)
    SubDone { stage: Stage, label: String, msg: String },
    /// W7-G: sub-bar 失敗 (`SubBar::fail(err)`)
    SubFail { stage: Stage, label: String, err: String },
    Finished,
}

#[cfg(feature = "tui")]
struct TuiBackend {
    tx: tokio::sync::mpsc::UnboundedSender<PipelineUpdate>,
}

#[cfg(feature = "tui")]
impl PipelineBackend for TuiBackend {
    fn stage_start(&self, stage: Stage, msg: &str) {
        let _ = self.tx.send(PipelineUpdate::StageStart {
            stage,
            msg: msg.to_string(),
        });
    }
    fn stage_done(&self, stage: Stage, msg: &str) {
        let _ = self.tx.send(PipelineUpdate::StageDone {
            stage,
            msg: msg.to_string(),
        });
    }
    fn stage_fail(&self, stage: Stage, err: &str) {
        let _ = self.tx.send(PipelineUpdate::StageFail {
            stage,
            err: err.to_string(),
        });
    }
    fn finish(&self) {
        let _ = self.tx.send(PipelineUpdate::Finished);
    }

    /// W7-G: TuiBackend では sub_bar も mpsc 経由で App に流す。
    /// `TuiSubBar` 構造体が tick/done/fail で SubTick/SubDone/SubFail event を送信する。
    fn sub_bar(&self, stage: Stage, label: &str) -> Box<dyn SubBar> {
        let _ = self.tx.send(PipelineUpdate::SubStart {
            stage,
            label: label.to_string(),
        });
        Box::new(TuiSubBar {
            tx: self.tx.clone(),
            stage,
            label: label.to_string(),
        })
    }
}

/// W7-G: TuiBackend 用 SubBar 実装。各 method で mpsc 経由で App に PipelineUpdate を送信。
#[cfg(feature = "tui")]
struct TuiSubBar {
    tx: tokio::sync::mpsc::UnboundedSender<PipelineUpdate>,
    stage: Stage,
    label: String,
}

#[cfg(feature = "tui")]
impl SubBar for TuiSubBar {
    fn tick(&self, msg: &str) {
        let _ = self.tx.send(PipelineUpdate::SubTick {
            stage: self.stage,
            label: self.label.clone(),
            msg: msg.to_string(),
        });
    }
    fn done(&self, msg: &str) {
        let _ = self.tx.send(PipelineUpdate::SubDone {
            stage: self.stage,
            label: self.label.clone(),
            msg: msg.to_string(),
        });
    }
    fn fail(&self, err: &str) {
        let _ = self.tx.send(PipelineUpdate::SubFail {
            stage: self.stage,
            label: self.label.clone(),
            err: err.to_string(),
        });
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Summary table (W2: comfy-table 統合)
//
// - Unicode (rounded): preset UTF8_FULL_CONDENSED + 角丸ヘッダ
// - ASCII (--ascii):   preset ASCII_BORDERS_ONLY_CONDENSED
// - 着色: ヘッダは bold、note/X status は success/dim/error_color で塗る
// - 幅制御: ContentArrangement::Dynamic + theme 検出済端末幅 (or fallback 100)

fn make_table(theme: &Theme) -> comfy_table::Table {
    use comfy_table::{
        presets::{ASCII_BORDERS_ONLY_CONDENSED, UTF8_FULL_CONDENSED},
        ContentArrangement, Table,
    };
    let mut table = Table::new();
    let preset = if theme.uses_unicode {
        UTF8_FULL_CONDENSED
    } else {
        ASCII_BORDERS_ONLY_CONDENSED
    };
    table
        .load_preset(preset)
        .set_content_arrangement(ContentArrangement::Dynamic)
        // 端末幅取得 (console::Term 経由) — 取れなければ 100 cols
        .set_width(
            console::Term::stdout()
                .size_checked()
                .map(|(_h, w)| w)
                .unwrap_or(100),
        );
    table
}

/// publish 完了サマリ表 (写経の `print_done` 互換)。`PublishResult` の slug / title /
/// note_status / x_status を rounded 表で表示し、フッタに件数 / 経過秒 / 総文字数を出す。
pub fn print_done(theme: &Theme, summary: &RunSummary) {
    use comfy_table::Cell;

    let mut table = make_table(theme);
    table.set_header(vec![
        Cell::new(bold(theme, "#")),
        Cell::new(bold(theme, "Slug")),
        Cell::new(bold(theme, "Title")),
        Cell::new(bold(theme, "note")),
        Cell::new(bold(theme, "X")),
    ]);

    for (i, a) in summary.articles.iter().enumerate() {
        table.add_row(vec![
            Cell::new(format!("{}", i + 1)),
            Cell::new(truncate(&a.slug, 20)),
            Cell::new(truncate(&a.title, 40)),
            Cell::new(short_status(theme, &a.note_status)),
            Cell::new(short_status(theme, &a.x_status)),
        ]);
    }

    println!();
    println!("{table}");

    // フッタ
    let g = glyphs(theme);
    println!(
        "{} {} 記事 / {}s / {} 文字",
        success(theme, g.check),
        summary.articles.len(),
        summary.duration_secs,
        summary.total_chars
    );
}

/// scoring 結果サマリ表。`SelectedTrend` の source / title / raw_score / composite_score を表示。
/// main.rs の `Run` / `Once` コマンドで scoring 完了直後に呼ぶ想定。
pub fn print_scoring_table(theme: &Theme, selected: &[crate::scoring::SelectedTrend]) {
    use comfy_table::Cell;

    if selected.is_empty() {
        return;
    }

    let mut table = make_table(theme);
    table.set_header(vec![
        Cell::new(bold(theme, "#")),
        Cell::new(bold(theme, "Source")),
        Cell::new(bold(theme, "Title")),
        Cell::new(bold(theme, "Raw")),
        Cell::new(bold(theme, "Composite")),
    ]);

    for (i, s) in selected.iter().enumerate() {
        table.add_row(vec![
            Cell::new(format!("{}", i + 1)),
            Cell::new(accent(theme, &s.item.source)),
            Cell::new(truncate(&s.item.title, 48)),
            Cell::new(format!("{:.2}", s.item.raw_score)),
            Cell::new(success(theme, &format!("{:.3}", s.composite_score))),
        ]);
    }

    println!();
    println!("{table}");
    let g = glyphs(theme);
    println!(
        "{} {} 件選定",
        success(theme, g.check),
        selected.len()
    );
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
        // CL1: 旧実装は wide char を 2 とカウントする計画だったが全 branch で 1 を返す
        //      identical if blocks になっていた (clippy::if_same_then_else)。
        //      現状は CJK 等を含めて 1 char = 1 column として扱う。
        //      正確な width 判定は将来 unicode-width crate 導入時に再検討。
        n += 1;
    }
    n
}

// ─────────────────────────────────────────────────────────────────────────────
// Public re-exports for main.rs convenience

/// shorthand: `display::theme()` — `Theme::current()` を直接呼ぶ簡略呼び出し。
/// main.rs は `Theme::init()` の戻り値を直接保持するため未使用、Phase 2 で他モジュールから
/// グローバル取得するときに wire 予定。
#[allow(dead_code)]
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
        // W3: owo-colors の truecolor 経由でも先頭は `\x1b[38;2;R;G;Bm` で同じ。
        // 末尾の reset は owo-colors v4 では `\x1b[39m` (foreground reset) を使う。
        let t = Theme { uses_color: true, truecolor: true, uses_unicode: true, is_tty: true };
        let out = accent(&t, "x");
        assert!(out.starts_with("\x1b[38;2;0;122;255m"), "got: {:?}", out);
        // owo-colors v4 → `\x1b[39m`、自前 ANSI → `\x1b[0m` のどちらでも可
        assert!(out.ends_with("\x1b[0m") || out.ends_with("\x1b[39m"), "got: {:?}", out);
        assert_eq!(visible_len(&out), 1, "visible char must be just 'x'");
    }

    #[test]
    fn ansi_8bit_fallback_when_no_truecolor() {
        // truecolor=false 時は ANSI 256-color 直書き
        let t = Theme { uses_color: true, truecolor: false, uses_unicode: true, is_tty: true };
        let out = accent(&t, "x");
        assert!(out.starts_with("\x1b[38;5;33m"), "got: {:?}", out);
        assert!(out.ends_with("\x1b[0m"), "got: {:?}", out);
    }

    #[test]
    fn bold_uses_owo_colors() {
        // W3: bold は owo-colors 経由
        let t = Theme { uses_color: true, truecolor: true, uses_unicode: true, is_tty: true };
        let out = bold(&t, "x");
        assert!(out.contains("\x1b[1m"), "expected bold sequence, got: {:?}", out);
    }

    #[test]
    fn visible_len_strips_ansi() {
        assert_eq!(visible_len("hello"), 5);
        assert_eq!(visible_len("\x1b[1mhello\x1b[0m"), 5);
        // owo-colors fg-reset (39) も剥がせる
        assert_eq!(visible_len("\x1b[38;2;0;122;255mhello\x1b[39m"), 5);
    }

    #[test]
    fn theme_color_mode_never_disables_truecolor() {
        // W5: ColorMode::Never は supports-color の判定を完全上書き
        let opts = ThemeOptions { force_ascii: false, color: ColorMode::Never };
        let t = Theme::detect(opts);
        assert!(!t.uses_color);
        assert!(!t.truecolor);
    }
}
