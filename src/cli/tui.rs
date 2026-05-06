//! ratatui ベースの TUI モード (`note-auto tui`) — v0.8.0 Phase 2-B (W7-B)。
//!
//! ## 設計
//!
//! - macOS Big Sur dark テーマ風 3 ペイン: 左 Sidebar (カテゴリ) / 右上 Pipeline (5 stage 進捗) / 右下 Logs (tail)
//! - Aqua (#007AFF) アクセント、Rounded ボーダー
//! - キーバインド: `j/k` 移動 / `Enter` 実行 / `Tab` ペイン切替 / `q` 終了
//!
//! ## アーキテクチャ
//!
//! ```text
//!     ┌──────────────────────┐
//!     │ App state            │◄─── tokio::mpsc::UnboundedReceiver<AppEvent>
//!     └──────┬───────────────┘
//!            │ render frame (60Hz)
//!            ▼
//!     ┌──────────────┐         ┌──────────────────────────────┐
//!     │ ratatui      │         │ worker tokio task            │
//!     │ Frame        │         │  - daemon::execute_cycle()   │
//!     └──────────────┘         │  - PipelineProgress::new_tui │
//!                              │    で UnboundedSender に push │
//!                              └──────────────────────────────┘
//! ```
//!
//! W7-B: 静的 UI + Enter で 1 サイクル実行統合まで。daemon 連携は W7-C で別 PR。

use std::collections::VecDeque;
use std::io;
use std::time::Duration;

use anyhow::Result;
use ratatui::{
    backend::CrosstermBackend,
    crossterm::{
        event::{DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind},
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    },
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Terminal,
};
use tokio::sync::mpsc;

use crate::config::Config;
use crate::display::{PipelineUpdate, Stage, Theme};

// ─────────────────────────────────────────────────────────────────────────────
// State

/// カテゴリ定義 (固定リスト、config.toml はカテゴリ毎ロード)
const CATEGORIES: &[Category] = &[
    Category { slug: "note", display: "● note", default_top: 1 },
    Category { slug: "x", display: "● x", default_top: 3 },
    Category { slug: "google", display: "● google", default_top: 3 },
    Category { slug: "hn", display: "● hn", default_top: 3 },
    Category { slug: "konbini", display: "● konbini", default_top: 3 },
    Category { slug: "hyakkin", display: "● hyakkin", default_top: 3 },
    Category { slug: "gnews", display: "● gnews", default_top: 3 },
    Category { slug: "all", display: "● ALL", default_top: 7 },
];

#[derive(Clone, Copy, Debug)]
struct Category {
    slug: &'static str,
    display: &'static str,
    default_top: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pane {
    Sidebar,
    Pipeline,
    Logs,
}

/// 5 stage の表示状態
#[derive(Clone, Debug)]
enum StageState {
    Pending,
    InProgress(String),
    Done(String),
    Failed(String),
}

/// アプリ全体の state (immutable update pattern)
struct App {
    selected_idx: usize,
    list_state: ListState,
    pipeline: [StageState; 5],
    /// P4: 各 stage の InProgress 開始時刻 (elapsed 計測用、Done/Failed 時に差分計算)
    stage_started_at: [Option<std::time::Instant>; 5],
    logs: VecDeque<String>,
    focus: Pane,
    /// 実行中フラグ — 二重起動防止
    running: bool,
    /// 終了フラグ
    quit: bool,
    /// 最後の実行結果メッセージ (footer 表示用)
    last_msg: Option<String>,
    /// W7-C: 起動時に History::load() で取得した直近 5 件 (Logs ペイン上に表示)
    history_recent: Vec<String>,
    /// W7-C: theme (border_type 切替に使用)
    theme: Theme,
    /// P4: ヘルプオーバーレイ表示中 (`?` キーでトグル)
    help_overlay: bool,
    /// W7-F: 直近の StageFail / WorkerDone Err の詳細 (e キーで modal 展開)
    error_detail: Option<ErrorDetail>,
    /// W7-F: エラー詳細モーダル表示中 (`e` キーでトグル)
    error_overlay: bool,
    /// W7-G: stage 配下の sub-bar 状態。key = (Stage, label)、value = SubBarItem
    /// 例: (Stage::Fetch, "hn") → "hn fetched 30 件" の done state
    /// 完了/失敗した sub-bar も保持し、stage_done で当該 stage の sub-bars を一括 finish しない
    /// (個別のラベル毎の done/fail を render に反映、最終 stage 完了時もそのまま表示継続)
    sub_bars: std::collections::HashMap<(Stage, String), SubBarItem>,
}

/// W7-G: sub-bar の表示状態 (Pending/InProgress/Done/Failed)
#[derive(Clone, Debug)]
struct SubBarItem {
    state: SubBarState,
    msg: String,
    /// stage 配下の表示順序を維持するための insertion order (HashMap は順序保証しないため)
    order: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SubBarState {
    InProgress,
    Done,
    Failed,
}

/// W7-F: エラー詳細情報 (StageFail / WorkerDone Err 時に保持)
struct ErrorDetail {
    stage: Option<Stage>,
    msg: String,
    timestamp: chrono::DateTime<chrono::Local>,
}

impl App {
    fn new(theme: Theme) -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));

        // W7-C: 起動時に History::load() で直近 5 件を取得 (entries は append 順なので末尾が新しい)
        let history_recent: Vec<String> = crate::history::History::load(None)
            .ok()
            .map(|h| {
                h.entries
                    .iter()
                    .rev()
                    .take(5)
                    .map(|e| format!("• {} [{}]", short_str(&e.title, 30), e.date))
                    .collect()
            })
            .unwrap_or_default();

        Self {
            selected_idx: 0,
            list_state,
            pipeline: [
                StageState::Pending,
                StageState::Pending,
                StageState::Pending,
                StageState::Pending,
                StageState::Pending,
            ],
            stage_started_at: [None, None, None, None, None],
            logs: VecDeque::with_capacity(1000),
            focus: Pane::Sidebar,
            running: false,
            quit: false,
            last_msg: None,
            history_recent,
            theme,
            help_overlay: false,
            error_detail: None,
            error_overlay: false,
            sub_bars: std::collections::HashMap::new(),
        }
    }

    fn select_next(&mut self) {
        if self.selected_idx + 1 < CATEGORIES.len() {
            self.selected_idx += 1;
            self.list_state.select(Some(self.selected_idx));
        }
    }

    fn select_prev(&mut self) {
        if self.selected_idx > 0 {
            self.selected_idx -= 1;
            self.list_state.select(Some(self.selected_idx));
        }
    }

    fn cycle_focus(&mut self) {
        self.focus = match self.focus {
            Pane::Sidebar => Pane::Pipeline,
            Pane::Pipeline => Pane::Logs,
            Pane::Logs => Pane::Sidebar,
        };
    }

    fn push_log(&mut self, line: String) {
        if self.logs.len() >= 1000 {
            self.logs.pop_front();
        }
        self.logs.push_back(line);
    }

    fn apply_update(&mut self, upd: PipelineUpdate) {
        match upd {
            PipelineUpdate::StageStart { stage, msg } => {
                let i = stage as usize;
                self.pipeline[i] = StageState::InProgress(msg.clone());
                // P4: stage 開始時刻を記録 (elapsed 計測用)
                self.stage_started_at[i] = Some(std::time::Instant::now());
                self.push_log(format!("→ [{}] {}", stage.label(), msg));
            }
            PipelineUpdate::StageDone { stage, msg } => {
                let i = stage as usize;
                let elapsed_str = self.stage_started_at[i]
                    .map(|t| format_elapsed(t.elapsed()))
                    .unwrap_or_default();
                let combined = if elapsed_str.is_empty() {
                    msg.clone()
                } else {
                    format!("{msg}  ({elapsed_str})")
                };
                self.pipeline[i] = StageState::Done(combined.clone());
                self.push_log(format!("✓ [{}] {}", stage.label(), combined));
            }
            PipelineUpdate::StageFail { stage, err } => {
                let i = stage as usize;
                let elapsed_str = self.stage_started_at[i]
                    .map(|t| format_elapsed(t.elapsed()))
                    .unwrap_or_default();
                let combined = if elapsed_str.is_empty() {
                    err.clone()
                } else {
                    format!("{err}  ({elapsed_str})")
                };
                self.pipeline[i] = StageState::Failed(combined.clone());
                self.push_log(format!("✗ [{}] {}", stage.label(), combined));
                // W7-F: 最新の StageFail を error_detail に保持 (e キーで modal 展開)
                self.error_detail = Some(ErrorDetail {
                    stage: Some(stage),
                    msg: err,
                    timestamp: chrono::Local::now(),
                });
            }
            PipelineUpdate::Finished => {
                self.running = false;
                self.last_msg = Some("実行完了".to_string());
            }
            // W7-G: sub-bar イベント処理 — 階層表示用 state 更新
            PipelineUpdate::SubStart { stage, label } => {
                let order = self.sub_bars.len();
                self.sub_bars.insert((stage, label.clone()), SubBarItem {
                    state: SubBarState::InProgress,
                    msg: String::new(),
                    order,
                });
            }
            PipelineUpdate::SubTick { stage, label, msg } => {
                if let Some(item) = self.sub_bars.get_mut(&(stage, label.clone())) {
                    item.msg = msg;
                }
            }
            PipelineUpdate::SubDone { stage, label, msg } => {
                // Borrow checker: entry() の前に len() を取得 (entry は &mut self.sub_bars を奪う)
                let order_init = self.sub_bars.len();
                let item = self.sub_bars
                    .entry((stage, label.clone()))
                    .or_insert_with(|| SubBarItem {
                        state: SubBarState::Done,
                        msg: String::new(),
                        order: order_init,
                    });
                item.state = SubBarState::Done;
                item.msg = msg.clone();
                self.push_log(format!("  ↳ [{}/{}] {}", stage.label(), label, msg));
            }
            PipelineUpdate::SubFail { stage, label, err } => {
                let order_init = self.sub_bars.len();
                let item = self.sub_bars
                    .entry((stage, label.clone()))
                    .or_insert_with(|| SubBarItem {
                        state: SubBarState::Failed,
                        msg: String::new(),
                        order: order_init,
                    });
                item.state = SubBarState::Failed;
                item.msg = err.clone();
                self.push_log(format!("  ↳ ✗ [{}/{}] {}", stage.label(), label, err));
            }
        }
    }

    fn reset_pipeline(&mut self) {
        for s in &mut self.pipeline {
            *s = StageState::Pending;
        }
        self.stage_started_at = [None, None, None, None, None];
        // W7-G: 新規実行開始時に sub_bars もクリア
        self.sub_bars.clear();
    }
}

/// P4: Duration を人間向け短縮表示 ("1.2s" / "234ms" / "45µs")
fn format_elapsed(d: std::time::Duration) -> String {
    let nanos = d.as_nanos();
    if nanos >= 1_000_000_000 {
        format!("{:.1}s", d.as_secs_f64())
    } else if nanos >= 1_000_000 {
        format!("{}ms", d.as_millis())
    } else if nanos >= 1_000 {
        format!("{}µs", nanos / 1_000)
    } else {
        format!("{nanos}ns")
    }
}

enum AppEvent {
    Key(KeyEvent),
    Tick,
    Pipeline(PipelineUpdate),
    /// W7-D: tracing イベントを TUI Logs ペインに反映するためのバリアント。
    /// 本 PR-N では受信側 (push_log への forwarding) のみ実装、subscriber 側の発火経路
    /// (`TuiTracingLayer` 等) は次 PR で wire 予定のため、現時点では送信側ゼロで dead_code 警告が出る。
    #[allow(dead_code)]
    Log(String),
    WorkerDone(Result<String, String>),
}

// ─────────────────────────────────────────────────────────────────────────────
// Entry point

/// `note-auto tui` の起動エントリポイント。alt screen + raw mode を取得し、
/// イベントループを回して終了 (`q` / Ctrl-C / panic 時) に必ず後処理する。
pub async fn run(_cfg: &Config, theme: Theme) -> Result<()> {
    // Terminal セットアップ (alt screen + raw mode)
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // RAII でクリーンアップを保証する guard
    let result = run_app(&mut terminal, theme).await;

    // 後処理: panic 時にも実行されるよう、明示的に呼ぶ
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

async fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    theme: Theme,
) -> Result<()> {
    let mut app = App::new(theme);

    // 内部 channel: AppEvent
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AppEvent>();

    // crossterm input task: blocking poll → AppEvent::Key
    let input_tx = event_tx.clone();
    let _input_handle = tokio::task::spawn_blocking(move || loop {
        if ratatui::crossterm::event::poll(Duration::from_millis(100)).unwrap_or(false) {
            if let Ok(Event::Key(k)) = ratatui::crossterm::event::read() {
                if k.kind == KeyEventKind::Press {
                    if input_tx.send(AppEvent::Key(k)).is_err() {
                        break;
                    }
                }
            }
        }
    });

    // tick task: 100ms 間隔で AppEvent::Tick (時刻表示等の用途、本実装では未使用だが拡張余地)
    let tick_tx = event_tx.clone();
    let _tick_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        loop {
            interval.tick().await;
            if tick_tx.send(AppEvent::Tick).is_err() {
                break;
            }
        }
    });

    // 起動メッセージ
    app.push_log("note-auto TUI 起動。j/k 移動 / Enter 実行 / Tab ペイン / q 終了".to_string());

    // W7-C: History の直近 5 件を Logs に流して可視化
    if !app.history_recent.is_empty() {
        app.push_log("─── 最近の履歴 ───".to_string());
        let recent = app.history_recent.clone();
        for line in recent {
            app.push_log(line);
        }
        app.push_log("──────────────".to_string());
    }

    while !app.quit {
        // Render
        terminal.draw(|f| ui(f, &mut app))?;

        // Event 受信
        if let Some(ev) = event_rx.recv().await {
            match ev {
                AppEvent::Key(k) => handle_key(&mut app, k, &event_tx).await,
                AppEvent::Tick => {} // 現状未使用
                AppEvent::Pipeline(upd) => app.apply_update(upd),
                AppEvent::Log(line) => app.push_log(line),
                AppEvent::WorkerDone(res) => {
                    app.running = false;
                    match res {
                        Ok(msg) => {
                            app.last_msg = Some(msg.clone());
                            app.push_log(format!("✓ {}", msg));
                        }
                        Err(err) => {
                            app.last_msg = Some(format!("失敗: {err}"));
                            app.push_log(format!("✗ {}", err));
                            // W7-F: WorkerDone Err も error_detail に保持
                            app.error_detail = Some(ErrorDetail {
                                stage: None,
                                msg: err,
                                timestamp: chrono::Local::now(),
                            });
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

async fn handle_key(
    app: &mut App,
    k: KeyEvent,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
) {
    use ratatui::crossterm::event::KeyModifiers;

    // P4: ヘルプオーバーレイ表示中はほぼ全キー無効化、`?`/`Esc`/`q`/`Enter` のみ受け付ける
    if app.help_overlay {
        match k.code {
            KeyCode::Char('?')
            | KeyCode::Char('q')
            | KeyCode::Esc
            | KeyCode::Enter => {
                app.help_overlay = false;
            }
            _ => {}
        }
        return;
    }

    // W7-F: エラーオーバーレイ表示中も同様に `e`/`Esc`/`q`/`Enter` のみ受け付ける
    if app.error_overlay {
        match k.code {
            KeyCode::Char('e')
            | KeyCode::Char('q')
            | KeyCode::Esc
            | KeyCode::Enter => {
                app.error_overlay = false;
            }
            _ => {}
        }
        return;
    }

    // P4: Ctrl+L で Logs ペインクリア (Bash の clear と同じセマンティクス)
    if k.modifiers.contains(KeyModifiers::CONTROL) && matches!(k.code, KeyCode::Char('l') | KeyCode::Char('L')) {
        app.logs.clear();
        app.push_log("(Logs cleared)".to_string());
        return;
    }

    match k.code {
        // P4: ? でヘルプオーバーレイ
        KeyCode::Char('?') => {
            app.help_overlay = true;
        }
        // W7-F: e でエラー詳細オーバーレイトグル (error_detail があれば)
        KeyCode::Char('e') | KeyCode::Char('E') => {
            if app.error_detail.is_some() {
                app.error_overlay = true;
            } else {
                app.push_log("(エラー履歴なし — 失敗してから再度 e で詳細表示)".to_string());
            }
        }
        KeyCode::Char('q') | KeyCode::Esc => {
            if !app.running {
                app.quit = true;
            }
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if app.focus == Pane::Sidebar {
                app.select_next();
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if app.focus == Pane::Sidebar {
                app.select_prev();
            }
        }
        KeyCode::Tab => app.cycle_focus(),
        KeyCode::Enter => {
            if app.running {
                app.push_log("既に実行中です (.lock 衝突防止)".to_string());
                return;
            }
            app.running = true;
            app.reset_pipeline();
            let cat = CATEGORIES[app.selected_idx];
            app.push_log(format!("→ {} (top {}) 実行開始", cat.slug, cat.default_top));

            let tx = event_tx.clone();
            tokio::spawn(async move {
                let res = run_pipeline_real(cat, tx.clone()).await;
                let _ = tx.send(AppEvent::WorkerDone(res.map_err(|e| e.to_string())));
            });
        }
        _ => {}
    }
}

/// W7-C: 実 `daemon::execute_cycle_with_progress` 統合。
/// 選択された category に基づいて configs/<slug>.toml を再ロードし、
/// `PipelineProgress::new_tui(pu_tx)` 経由で TuiBackend を渡して実行。
/// `daemon::execute_cycle_with_progress` 内で `.note-auto.lock` も自動取得される。
async fn run_pipeline_real(
    cat: Category,
    app_tx: mpsc::UnboundedSender<AppEvent>,
) -> Result<String> {
    // category に応じて config を再ロード
    let cfg_path = if cat.slug == "all" {
        std::path::PathBuf::from("config.toml")
    } else {
        std::path::PathBuf::from("configs").join(format!("{}.toml", cat.slug))
    };
    let mut cfg = crate::config::Config::load(&cfg_path)?;
    cfg.schedule.daily_top = cat.default_top;

    // PipelineProgress 用の専用 channel
    let (pu_tx, mut pu_rx) = mpsc::unbounded_channel::<crate::display::PipelineUpdate>();

    // pu_rx → AppEvent::Pipeline へ forward する task
    let forward_tx = app_tx.clone();
    let forward = tokio::spawn(async move {
        while let Some(upd) = pu_rx.recv().await {
            if forward_tx.send(AppEvent::Pipeline(upd)).is_err() {
                break;
            }
        }
    });

    // TuiBackend で進捗を mpsc に push する PipelineProgress
    let progress = crate::display::PipelineProgress::new_tui(pu_tx);

    // 実 daemon パイプライン (.lock 取得 → fetch → score → write → publish → notify)
    let summary = crate::daemon::execute_cycle_with_progress(&cfg, &progress).await?;

    // Drop で finish() が呼ばれ、PipelineUpdate::Finished が forward に流れる
    drop(progress);
    let _ = forward.await;

    Ok(format!(
        "{} top {} 完了: {} 記事 / {}s / {} 文字",
        cat.slug,
        cat.default_top,
        summary.articles.len(),
        summary.duration_secs,
        summary.total_chars
    ))
}

// ─────────────────────────────────────────────────────────────────────────────
// Render — Big Sur dark テーマ風

const ACCENT: Color = Color::Rgb(0, 122, 255); // #007AFF
const SUCCESS: Color = Color::Rgb(48, 209, 88); // #30D158
const ERROR_C: Color = Color::Rgb(255, 69, 58); // #FF453A
const SECONDARY: Color = Color::Rgb(142, 142, 147); // #8E8E93

fn ui(f: &mut ratatui::Frame, app: &mut App) {
    // 全体: Title (1) / Body (rest) / Status (1)
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)])
        .split(f.area());

    // Title bar
    let title = Paragraph::new(Line::from(vec![
        Span::styled("note-auto ", Style::default().add_modifier(Modifier::BOLD).fg(ACCENT)),
        Span::raw(format!("v{}", env!("CARGO_PKG_VERSION"))),
        Span::raw("    "),
        Span::styled("⚙ TUI", Style::default().fg(SECONDARY)),
    ]));
    f.render_widget(title, outer[0]);

    // Body: Sidebar (30 cols) | Right pane
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(28), Constraint::Min(0)])
        .split(outer[1]);

    render_sidebar(f, body[0], app);

    // Right pane: Pipeline (top 12 rows) / Logs (rest)
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(12), Constraint::Min(0)])
        .split(body[1]);

    render_pipeline(f, right[0], app);
    render_logs(f, right[1], app);

    // Status bar (P4: ?Help / Ctrl+L Clear-logs を追加)
    let status = Paragraph::new(Line::from(vec![
        Span::styled("j/k", Style::default().fg(ACCENT)),
        Span::styled(" Move  ", Style::default().fg(SECONDARY)),
        Span::styled("Enter", Style::default().fg(ACCENT)),
        Span::styled(" Run  ", Style::default().fg(SECONDARY)),
        Span::styled("Tab", Style::default().fg(ACCENT)),
        Span::styled(" Pane  ", Style::default().fg(SECONDARY)),
        Span::styled("?", Style::default().fg(ACCENT)),
        Span::styled(" Help  ", Style::default().fg(SECONDARY)),
        Span::styled("Ctrl-L", Style::default().fg(ACCENT)),
        Span::styled(" ClearLog  ", Style::default().fg(SECONDARY)),
        // W7-F: e Error ヒント (error_detail 有なら ERROR_C 強調、無なら dim)
        Span::styled(
            "e",
            Style::default().fg(if app.error_detail.is_some() { ERROR_C } else { SECONDARY }),
        ),
        Span::styled(" Error  ", Style::default().fg(SECONDARY)),
        Span::styled("q", Style::default().fg(ACCENT)),
        Span::styled(" Quit", Style::default().fg(SECONDARY)),
        Span::raw("  "),
        Span::styled(
            if app.running { "● running" } else { "○ idle" },
            Style::default().fg(if app.running { SUCCESS } else { SECONDARY }),
        ),
    ]));
    f.render_widget(status, outer[2]);

    // P4: ヘルプオーバーレイ (Toggle: `?`)
    if app.help_overlay {
        render_help_overlay(f, app);
    }
    // W7-F: エラー詳細オーバーレイ (Toggle: `e`)
    if app.error_overlay {
        render_error_overlay(f, app);
    }
}

/// P4: 中央に help モーダルを描画 (Clear で背景を消してから Block + Paragraph を重ねる)
fn render_help_overlay(f: &mut ratatui::Frame, app: &App) {
    use ratatui::widgets::Clear;

    let area = f.area();
    let modal_w = 56u16.min(area.width.saturating_sub(4));
    let modal_h = 16u16.min(area.height.saturating_sub(4));
    let modal_x = area.x + (area.width.saturating_sub(modal_w)) / 2;
    let modal_y = area.y + (area.height.saturating_sub(modal_h)) / 2;
    let modal_area = Rect::new(modal_x, modal_y, modal_w, modal_h);

    f.render_widget(Clear, modal_area); // 背景クリア

    let lines = vec![
        Line::from(vec![Span::styled(
            " note-auto TUI — Help ",
            Style::default().add_modifier(Modifier::BOLD).fg(ACCENT),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  j / ↓     ", Style::default().fg(ACCENT)),
            Span::raw("カテゴリを下へ"),
        ]),
        Line::from(vec![
            Span::styled("  k / ↑     ", Style::default().fg(ACCENT)),
            Span::raw("カテゴリを上へ"),
        ]),
        Line::from(vec![
            Span::styled("  Enter     ", Style::default().fg(ACCENT)),
            Span::raw("選択カテゴリで pipeline 実行"),
        ]),
        Line::from(vec![
            Span::styled("  Tab       ", Style::default().fg(ACCENT)),
            Span::raw("ペイン切替 (Sidebar↔Pipeline↔Logs)"),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl-L    ", Style::default().fg(ACCENT)),
            Span::raw("Logs ペインクリア"),
        ]),
        Line::from(vec![
            Span::styled("  ?         ", Style::default().fg(ACCENT)),
            Span::raw("このヘルプを toggle"),
        ]),
        Line::from(vec![
            Span::styled("  q / Esc   ", Style::default().fg(ACCENT)),
            Span::raw("終了 (実行中はガード)"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  press ? / Enter / q / Esc to close",
            Style::default().fg(SECONDARY),
        )),
    ];

    let block = Block::default()
        .title(" Help ")
        .borders(Borders::ALL)
        .border_type(border_type_for(&app.theme))
        .border_style(Style::default().fg(ACCENT));
    let para = Paragraph::new(lines).block(block);
    f.render_widget(para, modal_area);
}

/// W7-F: エラー詳細モーダル — 中央 70% × 60% 枠で展開、wrap=true で多行 stack trace を表示。
/// 配色: ERROR_C border + bold タイトル、stage / timestamp / err 全文を縦に並べる。
fn render_error_overlay(f: &mut ratatui::Frame, app: &App) {
    use ratatui::widgets::Clear;

    let Some(detail) = &app.error_detail else { return; };

    let area = f.area();
    let modal_w = ((area.width as f32 * 0.70) as u16).max(40).min(area.width.saturating_sub(2));
    let modal_h = ((area.height as f32 * 0.60) as u16).max(10).min(area.height.saturating_sub(2));
    let modal_x = area.x + (area.width.saturating_sub(modal_w)) / 2;
    let modal_y = area.y + (area.height.saturating_sub(modal_h)) / 2;
    let modal_area = Rect::new(modal_x, modal_y, modal_w, modal_h);

    f.render_widget(Clear, modal_area); // 背景クリア

    // ヘッダー: 時刻 + stage
    let header_stage = match detail.stage {
        Some(s) => format!("stage: {}", s.label()),
        None => "stage: (worker level)".to_string(),
    };
    let mut lines = vec![
        Line::from(vec![Span::styled(
            " note-auto エラー詳細 ",
            Style::default().add_modifier(Modifier::BOLD).fg(ERROR_C),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  時刻: ", Style::default().fg(SECONDARY)),
            Span::raw(detail.timestamp.format("%Y-%m-%d %H:%M:%S").to_string()),
        ]),
        Line::from(vec![
            Span::styled("  種別: ", Style::default().fg(SECONDARY)),
            Span::raw(header_stage),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  ─── error message ───",
            Style::default().fg(SECONDARY),
        )),
    ];
    // err 本文を行単位で追加 (wrap=true で長行は自動折返しされる)
    for line in detail.msg.lines() {
        lines.push(Line::from(vec![Span::raw("  "), Span::raw(line.to_string())]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  press e / Enter / q / Esc to close",
        Style::default().fg(SECONDARY),
    )));

    let block = Block::default()
        .title(" Error Detail ")
        .borders(Borders::ALL)
        .border_type(border_type_for(&app.theme))
        .border_style(Style::default().fg(ERROR_C));
    let para = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    f.render_widget(para, modal_area);
}

fn render_sidebar(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let items: Vec<ListItem> = CATEGORIES
        .iter()
        .map(|c| {
            ListItem::new(Line::from(vec![
                Span::styled(c.display, Style::default().fg(ACCENT)),
                Span::raw(format!("  [{}]", c.default_top)),
            ]))
        })
        .collect();

    let focused = app.focus == Pane::Sidebar;
    let list = List::new(items)
        .block(
            Block::default()
                .title(" Categories ")
                .borders(Borders::ALL)
                .border_type(border_type_for(&app.theme))
                .border_style(if focused {
                    Style::default().fg(ACCENT)
                } else {
                    Style::default().fg(SECONDARY)
                }),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED).fg(ACCENT))
        .highlight_symbol("▸ ");
    f.render_stateful_widget(list, area, &mut app.list_state);
}

fn render_pipeline(f: &mut ratatui::Frame, area: Rect, app: &App) {
    // W7-G: 各 stage 行 + その配下の sub-bars を階層表示
    let mut lines: Vec<Line> = Vec::new();

    for i in 0..5 {
        let stage = match i {
            0 => Stage::Fetch,
            1 => Stage::Score,
            2 => Stage::Write,
            3 => Stage::Publish,
            _ => Stage::Notify,
        };
        let (icon, color, msg) = match &app.pipeline[i] {
            StageState::Pending => ("◯", SECONDARY, ""),
            StageState::InProgress(m) => ("⠿", ACCENT, m.as_str()),
            StageState::Done(m) => ("✓", SUCCESS, m.as_str()),
            StageState::Failed(m) => ("✗", ERROR_C, m.as_str()),
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", icon), Style::default().fg(color)),
            Span::styled(
                format!("{:<10}", stage.label()),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(msg.to_string(), Style::default().fg(SECONDARY)),
        ]));

        // W7-G: stage 配下の sub-bars を `↳ <label> <msg>` 形式で挿入順に表示
        let mut sub_items: Vec<(&String, &SubBarItem)> = app
            .sub_bars
            .iter()
            .filter(|((s, _), _)| *s == stage)
            .map(|((_, l), v)| (l, v))
            .collect();
        sub_items.sort_by_key(|(_, v)| v.order);
        for (label, item) in sub_items {
            let (sub_icon, sub_color) = match item.state {
                SubBarState::InProgress => ("⠿", ACCENT),
                SubBarState::Done => ("✓", SUCCESS),
                SubBarState::Failed => ("✗", ERROR_C),
            };
            let arrow = if app.theme.uses_unicode { "↳" } else { ">" };
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(arrow, Style::default().fg(SECONDARY)),
                Span::raw(" "),
                Span::styled(format!("{} ", sub_icon), Style::default().fg(sub_color)),
                Span::styled(
                    format!("{:<14}", label),
                    Style::default().fg(SECONDARY),
                ),
                Span::styled(item.msg.clone(), Style::default().fg(SECONDARY)),
            ]));
        }
    }

    let focused = app.focus == Pane::Pipeline;
    let block = Block::default()
        .title(" Pipeline ")
        .borders(Borders::ALL)
        .border_type(border_type_for(&app.theme))
        .border_style(if focused {
            Style::default().fg(ACCENT)
        } else {
            Style::default().fg(SECONDARY)
        });
    let para = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    f.render_widget(para, area);
}

fn render_logs(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let lines: Vec<Line> = app
        .logs
        .iter()
        .rev()
        .take(area.height.saturating_sub(2) as usize)
        .rev()
        .map(|s| Line::from(s.as_str()))
        .collect();

    let focused = app.focus == Pane::Logs;
    let block = Block::default()
        .title(format!(" Logs ({}) ", app.logs.len()))
        .borders(Borders::ALL)
        .border_type(border_type_for(&app.theme))
        .border_style(if focused {
            Style::default().fg(ACCENT)
        } else {
            Style::default().fg(SECONDARY)
        });
    let para = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    f.render_widget(para, area);
}

/// W7-C: theme.uses_unicode に応じた border_type 切替 (--ascii 反映)
fn border_type_for(theme: &Theme) -> BorderType {
    if theme.uses_unicode {
        BorderType::Rounded
    } else {
        BorderType::Plain
    }
}

/// 短縮ヘルパ (history 表示用)
fn short_str(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        let head: String = chars.into_iter().take(max.saturating_sub(1)).collect();
        format!("{head}…")
    }
}
