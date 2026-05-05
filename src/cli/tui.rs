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
            logs: VecDeque::with_capacity(1000),
            focus: Pane::Sidebar,
            running: false,
            quit: false,
            last_msg: None,
            history_recent,
            theme,
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
                self.pipeline[stage as usize] = StageState::InProgress(msg.clone());
                self.push_log(format!("→ [{}] {}", stage.label(), msg));
            }
            PipelineUpdate::StageDone { stage, msg } => {
                self.pipeline[stage as usize] = StageState::Done(msg.clone());
                self.push_log(format!("✓ [{}] {}", stage.label(), msg));
            }
            PipelineUpdate::StageFail { stage, err } => {
                self.pipeline[stage as usize] = StageState::Failed(err.clone());
                self.push_log(format!("✗ [{}] {}", stage.label(), err));
            }
            PipelineUpdate::Finished => {
                self.running = false;
                self.last_msg = Some("実行完了".to_string());
            }
        }
    }

    fn reset_pipeline(&mut self) {
        for s in &mut self.pipeline {
            *s = StageState::Pending;
        }
    }
}

enum AppEvent {
    Key(KeyEvent),
    Tick,
    Pipeline(PipelineUpdate),
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
    match k.code {
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

    // Status bar
    let status = Paragraph::new(Line::from(vec![
        Span::styled("j/k", Style::default().fg(ACCENT)),
        Span::styled(" Move   ", Style::default().fg(SECONDARY)),
        Span::styled("Enter", Style::default().fg(ACCENT)),
        Span::styled(" Run   ", Style::default().fg(SECONDARY)),
        Span::styled("Tab", Style::default().fg(ACCENT)),
        Span::styled(" Pane   ", Style::default().fg(SECONDARY)),
        Span::styled("q", Style::default().fg(ACCENT)),
        Span::styled(" Quit", Style::default().fg(SECONDARY)),
        Span::raw("   "),
        Span::styled(
            if app.running { "● running" } else { "○ idle" },
            Style::default().fg(if app.running { SUCCESS } else { SECONDARY }),
        ),
    ]));
    f.render_widget(status, outer[2]);
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
    let lines: Vec<Line> = (0..5)
        .map(|i| {
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
            Line::from(vec![
                Span::styled(format!(" {} ", icon), Style::default().fg(color)),
                Span::styled(
                    format!("{:<10}", stage.label()),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(msg.to_string(), Style::default().fg(SECONDARY)),
            ])
        })
        .collect();

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
