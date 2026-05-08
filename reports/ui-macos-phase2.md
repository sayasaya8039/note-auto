# Phase 2 ui-macos 調査レポート — ratatui TUI 設計案

> **Status**: 設計確定待ち（commander → team-lead 上申後に実装 GO）
> **Owner**: ui-macos teammate
> **Target version**: v0.8.0
> **編集スコープ予定**: 新規 `src/cli/tui.rs` + `src/main.rs` (Cli subcommand 追加) + `src/display.rs` (PipelineProgress Frame 経路追加)
> **依存追加**: `ratatui = "0.28"` 1 件のみ（`crossterm` は v0.7.7 で indicatif 経由 transitively 既収録）

---

## 1. TUI ゴール

note-auto を **bat 9 本不要** + **GUI 的な操作性** で運用可能にする。

| 現状 (v0.7.7) | TUI 後 (v0.8.0 目標) |
|--------------|---------------------|
| bat 9本 (run-{note,x,...}.bat) → 個別ダブルクリック | `note-auto tui` 一発で起動 → カテゴリ選択 → 実行 |
| cmd.exe 黒画面で indicatif progress 流れるだけ | Finder 風 3 ペイン (Sidebar + Progress + Log) |
| 実行中の途中状態 = stderr に流れた最後の数行のみ | フルスクリーン UI で 5 stage の進捗 + 7 source の子 bar をリアルタイム表示 |
| daemon 常駐 (cron 07:00 JST) は完全 black-box | TUI で attach して進行を監視可能（任意） |
| 操作: Ctrl+C 終了のみ | `j/k` 移動 / `Enter` 実行 / `Tab` ペイン切替 / `r` リフレッシュ / `q` 終了 |

---

## 2. 画面レイアウト仕様

### 2.1 3 ペイン構成 (デフォルト 120 cols × 30 rows 想定)

```
╭──────────────────────────────────────────────────────────────────────────────╮
│  note-auto v0.8.0                      ⏱ 14:23:05    🌐 connected   ⚙ Run   │  ← Title bar (top, 1 row)
├────────────────────┬─────────────────────────────────────────────────────────┤
│  Categories        │  Pipeline                                               │
│ ─────────────────  │  ╭───────────────────────────────────────────────────╮  │
│ ► ● note    [1]    │  │  ✓ fetch       7 sources · 124 items              │  │
│   ● x       [3]    │  │  ✓ score       3 selected (skipped 2 dup)         │  │
│   ● google  [3]    │  │  ⠋ write       AI generating: 1/3 (slug: xxx)     │  │  ← top-right
│   ● hn      [3]    │  │  ◯ publish     pending                            │  │     (Progress, 12 rows)
│   ● konbini [3]    │  │  ◯ notify      pending                            │  │
│   ● hyakkin [3]    │  ╰───────────────────────────────────────────────────╯  │
│   ● gnews   [3]    │                                                         │
│   ● ─────          │                                                         │
│   ● ALL     [7]    │                                                         │
│                    │                                                         │
│  Last run:         ├─────────────────────────────────────────────────────────┤
│   note 2h ago ✓    │  Logs (tail)                                            │
│   x   3h ago ✓     │  14:23:01 INFO note-auto v0.8.0 起動                    │
│   gnews 1d ✗ E429  │  14:23:02 INFO fetched source="hn" count=30             │  ← bottom-right
│                    │  14:23:03 INFO fetched source="note" count=12           │     (Logs tail, 14 rows)
│                    │  14:23:04 INFO trends.json を出力 count=3               │
│                    │  14:23:05 DEBUG writer::run start                       │
│                    │ ▼ scroll: PgUp/PgDn                                     │
├────────────────────┴─────────────────────────────────────────────────────────┤
│  j/k Move   Enter Run   Tab Pane   r Refresh   c Clear log   q Quit          │  ← Status bar (bottom, 1 row)
╰──────────────────────────────────────────────────────────────────────────────╯
```

### 2.2 macOS 風スタイリング

| 要素 | 実装 |
|------|------|
| **角丸ブロック** | `Block::bordered().border_type(BorderType::Rounded)` |
| **アクセント (#007AFF systemBlue)** | `Style::default().fg(Color::Rgb(0, 122, 255))` |
| **Selected 行塗り (Aqua reverse)** | `Style::default().add_modifier(Modifier::REVERSED).fg(Color::Rgb(0, 122, 255))` |
| **8pt grid 間隔** | `Layout::default().margin(1)` (4px 相当) |
| **Title bar の chrono** | `chrono::Local::now()` を 100ms tick で更新 |
| **Spinner (write 中等)** | display.rs と同じ `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏` を ratatui Span で出す |
| **Status bar (キー hint)** | dim color (#8E8E93) で表示、 main color と区別 |
| **NO_COLOR / --ascii 互換** | display.rs の Theme 検出を流用、ratatui Color::Reset で塗りなしに切替 |

### 2.3 キーバインド一覧

| 場面 | キー | アクション |
|------|------|-----------|
| Sidebar focus | `j` / `↓` | カテゴリ次へ |
| Sidebar focus | `k` / `↑` | カテゴリ前へ |
| Sidebar focus | `Enter` | 選択カテゴリで `daemon::execute_cycle` 起動 (Run mode) |
| Sidebar focus | `Space` | dry-run トグル → Enter で `--dry-run` 実行 |
| Logs focus | `PgUp` / `PgDn` | 1 ページスクロール |
| Logs focus | `g` / `G` | top / bottom ジャンプ |
| Logs focus | `c` | ログクリア (UI 表示のみ、ファイルは保持) |
| 全画面 | `Tab` | Sidebar ↔ Progress ↔ Logs ペイン切替 |
| 全画面 | `r` | UI 再描画 (端末リサイズ後) |
| 全画面 | `q` / `Esc` | 終了 (実行中なら確認ダイアログ) |
| 全画面 | `?` | キーバインド help モーダル |

---

## 3. 公開 API シグネチャ

### 3.1 main.rs Cli 拡張

```rust
#[derive(Subcommand)]
enum Command {
    // ... 既存
    /// インタラクティブ TUI モード (Finder 風 3 ペイン)
    Tui {
        /// 起動時にフォーカスするカテゴリ (省略時 note)
        #[arg(long)]
        category: Option<String>,
    },
}
```

### 3.2 cli::tui モジュール

```rust
// src/cli/mod.rs (新規 sub-module)
// src/cli/tui.rs (新規)

pub async fn run(cfg: &Config, opts: TuiOptions) -> anyhow::Result<()>;

pub struct TuiOptions {
    pub initial_category: Option<String>,
}
```

### 3.3 内部 State Machine

```rust
// src/cli/tui.rs

/// TUI app state (immutable update pattern)
pub struct App {
    /// 現在選択中の categoryインデックス
    pub selected_idx: usize,
    /// 7 sources + ALL の表示用カテゴリリスト
    pub categories: Vec<Category>,
    /// 実行中の PipelineProgress 状態 (snapshot)
    pub pipeline: Option<PipelineSnapshot>,
    /// ログバッファ (ring buffer, 1000 行)
    pub logs: VecDeque<LogLine>,
    /// 履歴サマリ (last_run / status)
    pub history: HistorySummary,
    /// フォーカス中のペイン
    pub focus: Pane,
    /// dry-run トグル
    pub dry_run: bool,
    /// 終了確認ダイアログ表示中
    pub quit_dialog: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pane { Sidebar, Progress, Logs }

pub struct Category {
    pub slug: String,        // "note" / "x" / "google" / etc.
    pub display: String,     // "● note"
    pub default_top: usize,  // 1 / 3 / 7
    pub last_run: Option<HistoryEntry>,
}

pub struct PipelineSnapshot {
    pub stages: [StageState; 5],  // Fetch/Score/Write/Publish/Notify
    pub current_msg: String,
    pub started: std::time::Instant,
}

pub enum StageState {
    Pending,
    InProgress { msg: String, since: std::time::Instant },
    Done { msg: String, took: std::time::Duration },
    Failed { err: String },
}

#[derive(Clone)]
pub struct LogLine {
    pub ts: chrono::DateTime<chrono::Local>,
    pub level: tracing::Level,
    pub msg: String,
}

/// イベント駆動 (crossterm::event::Event を解釈)
pub enum AppEvent {
    /// crossterm 入力イベント
    Key(crossterm::event::KeyEvent),
    /// パイプラインから push されるイベント
    PipelineUpdate(PipelineUpdate),
    /// tracing から push されるログ
    Log(LogLine),
    /// 100ms tick (時刻更新等)
    Tick,
    /// 端末リサイズ
    Resize(u16, u16),
}

pub enum PipelineUpdate {
    StageStart { stage: display::Stage, msg: String },
    StageDone { stage: display::Stage, msg: String },
    StageFail { stage: display::Stage, err: String },
}
```

### 3.4 daemon との接続戦略

```
┌───────────────────────────────────────────────────────────────┐
│ note-auto tui (foreground)                                    │
│                                                               │
│  ┌──────────────┐                                             │
│  │ App state    │◄──── tokio::sync::mpsc::Receiver<AppEvent>  │
│  └──────┬───────┘                                             │
│         │ render frame                                        │
│         ▼                                                     │
│  ┌──────────────┐         ┌──────────────────────────────┐    │
│  │ ratatui      │         │  worker tokio task           │    │
│  │ Frame        │         │  ─ daemon::execute_cycle()   │    │
│  └──────────────┘         │     (既存ロジック流用)       │    │
│                           │  ─ tracing event を AppEvent │    │
│                           │     ::Log に翻訳して push    │    │
│                           │  ─ PipelineProgress を       │    │
│                           │     mpsc::Sender 経由 push   │    │
│                           └──────────────────────────────┘    │
└───────────────────────────────────────────────────────────────┘
```

- TUI と既存 daemon (cron 07:00 JST 起動) は **別プロセス** で並走
- TUI 側からの実行は **`daemon::execute_cycle` 関数を直接呼ぶ** (既存ロジック流用、追加 wire 不要)
- TUI で実行中に cron が並列起動した場合: ファイルロック (`drafts/YYYY-MM-DD/.lock`) で衝突回避

---

## 4. 段階分割提案 (3 PR)

### W7-A: 静的 UI (起動 → 表示 → 終了)
**目的**: 画面骨格と入力ハンドリングだけ。実行は未統合。

| 実装 | 行数 | 時間 |
|------|------|------|
| `src/cli/tui.rs` 新規 (App / Pane / render) | +400 | 4h |
| `src/main.rs` Cli `Tui` variant 追加 + `cli::tui::run` 呼び出し | +20 | 30min |
| `Cargo.toml` `ratatui = "0.28"` 追加 (lowlevel 担当) | +1 | 15min |
| smoke: `note-auto tui` で起動 → `q` で終了するだけ | - | - |

**完了基準**: 起動できる / 表示崩れない / `q` で正常終了。

### W7-B: 実行統合 (Enter で `daemon::execute_cycle` 起動 → 進捗取り込み)
**目的**: PipelineProgress を ratatui Frame と接続。

| 実装 | 行数 | 時間 |
|------|------|------|
| `src/display.rs` に `PipelineProgressEmitter` trait 追加 (現 indicatif 出力 / TUI mpsc 出力を選択可能に) | +50/-20 | 2h |
| `src/cli/tui.rs` に worker tokio task + mpsc 経路追加 | +200 | 3h |
| tracing → mpsc::Sender<AppEvent::Log> bridge layer 実装 | +80 | 2h |
| smoke: `Enter` で `note --top 1` を実行 → 進捗が右ペインに流れる | - | - |

**完了基準**: 5 stage 進捗が **リアルタイム** に右上ペインに反映、ログが右下ペインに tail される。

### W7-C: daemon 連携 / 履歴表示 / dry-run
**目的**: 体験を完成させる。

| 実装 | 行数 | 時間 |
|------|------|------|
| `History` (`src/history.rs` 既存) を読んで Sidebar last_run 表示 | +80 | 2h |
| `Space` で dry-run トグル + status bar 表示 | +40 | 1h |
| `?` ヘルプモーダル + 終了確認ダイアログ | +100 | 2h |
| `--ascii` フラグ反映 (ratatui の border_type を Plain に切替) | +20 | 30min |
| smoke 拡張: dry-run / NO_COLOR / --ascii / リサイズ / scroll | - | - |

**完了基準**: 9 bat の機能を完全代替できる + cron 並走しても破綻しない。

合計: **3 PR / +約 1000 行 / 約 17 時間 (2-3 日)**

---

## 5. v0.7.7 display.rs との関係

### 5.1 PipelineProgress API 互換維持戦略

現状 (v0.7.7):
```rust
pub struct PipelineProgress {
    multi: indicatif::MultiProgress,
    bars: [indicatif::ProgressBar; 5],
    theme: Theme,
}

impl PipelineProgress {
    pub fn stage_start(&self, stage: Stage, msg: &str);
    pub fn stage_done(&self, stage: Stage, msg: &str);
    pub fn stage_fail(&self, stage: Stage, err: &str);
}
```

v0.8.0 提案: **Backend trait** で実装を抽象化、API シグネチャ無変更。

```rust
pub trait PipelineBackend: Send + Sync {
    fn stage_start(&self, stage: Stage, msg: &str);
    fn stage_done(&self, stage: Stage, msg: &str);
    fn stage_fail(&self, stage: Stage, err: &str);
    fn finish(&self);
}

pub struct IndicatifBackend { /* 現 v0.7.7 実装 */ }
impl PipelineBackend for IndicatifBackend { ... }

pub struct TuiBackend {
    sender: tokio::sync::mpsc::Sender<PipelineUpdate>,
}
impl PipelineBackend for TuiBackend { ... }

pub struct PipelineProgress {
    backend: Box<dyn PipelineBackend>,
}

impl PipelineProgress {
    /// 既存の v0.7.7 互換コンストラクタ — indicatif backend を使う
    pub fn new(theme: Theme) -> Self { ... }
    /// TUI 用コンストラクタ — mpsc backend を使う
    pub fn new_tui(sender: tokio::sync::mpsc::Sender<PipelineUpdate>, theme: Theme) -> Self { ... }
    // 公開 API (stage_start/done/fail) は backend に delegate、シグネチャ無変更
}
```

これにより:
- main.rs / daemon.rs / writer.rs の既存呼び出しは**完全に無変更**
- TUI から呼ぶときだけ `PipelineProgress::new_tui(...)` を使う
- `cli::tui::run` 内で worker task が `daemon::execute_cycle(cfg)` を呼ぶ前に thread-local で TuiBackend に差し替える方法もあり (検討)

### 5.2 indicatif vs ratatui の競合回避

- TUI モード時は indicatif を **完全 disable** (DrawTarget::hidden)
- ratatui が画面全体を draw、indicatif の描画は別経路
- `IndicatifBackend` を使うのは非 TUI モード (既存 cli) のみ

### 5.3 tracing 出力の取り扱い

- 通常モード: tracing → stderr (v0.7.7 既存)
- TUI モード: tracing → mpsc::Sender<AppEvent::Log> + stderr
  - `tracing-subscriber` の custom layer で intercept
  - 二重出力にする (TUI 上 + ファイル log) のはオプション化、デフォルトは TUI のみ

---

## 6. 想定リスク 5 項目

### R1: crossterm vs Windows ConHost 互換
- **症状**: 旧 cmd.exe (Windows 10 以前) で alt screen が effective に動かず、TUI 表示が崩れる
- **対策**:
  - TUI 起動前に `console::Term::stdout().features()` で確認、未対応なら `tracing::error!` + 早期 exit + 通常 CLI モードへの誘導
  - Windows Terminal / Cascadia Code 前提を README で明記
  - `--ascii` フラグ対応 (border_type::Plain)

### R2: ratatui rendering と stdout/stderr 競合
- **症状**: TUI 起動中に worker task が `println!` する (writer 内部の debug print 等) と alt screen が破壊
- **対策**:
  - TUI 起動時に `std::io::stdout()` を mpsc::Sender に redirect する custom Writer
  - もしくは worker task 内では tracing のみ使うルール (writer/publish 内の `println!` を排除)
  - 既存 `print_check` / `print_done` 等は TUI 中は no-op に切替

### R3: daemon 並行時のロック・状態共有
- **症状**: TUI と cron が同じ `drafts/YYYY-MM-DD/` に書き込もうとする
- **対策**:
  - ファイルロック (`drafts/YYYY-MM-DD/.lock` を `flock` 同等で取得) を `daemon::execute_cycle` 冒頭に追加
  - 既に LOCK が取られていれば `tracing::warn!` + skip
  - 履歴 `history.json` は v0.7.7 で atomic write 化済 (Q3) なので race なし

### R4: 既存 CLI フラグとの競合
- **症状**: `--ascii` / `--color` / `--category` / `--config` を TUI モードでどう適用するか
- **対策**:
  - `--ascii` → ratatui border_type Plain + alt charset 無効化
  - `--color=never` → ratatui Color::Reset 全面適用 (NO_COLOR 等価)
  - `--category` → 起動時の selected_idx 初期値
  - `--config` → cfg::load 経由で TuiOptions に渡す (既存挙動)

### R5: Cargo.toml 依存追加ポリシー
- **症状**: ratatui 追加でバイナリ +2-3MB 増、Phase 1.5 完了時の 5 deps wire-up 方針との整合確認
- **対策**:
  - ratatui のみ追加 (crossterm は v0.7.7 で indicatif 経由で既収録)
  - Cargo.toml 編集は lowlevel 領域なので、Phase 2 タスク表で lowlevel に依頼
  - `[features]` で `tui` feature 化することも検討 (デフォルト ON、`cargo build --no-default-features` で TUI 抜きビルド可)

---

## 7. 工数見積 + 採用判断

### 段階別見積

| Phase | 内容 | 行数 | 工数 |
|-------|------|------|------|
| W7-A | 静的 UI (骨格 + 入力 + 終了) | +420 | 4.75h |
| W7-B | 実行統合 (PipelineProgress backend trait + worker task) | +330/-20 | 7h |
| W7-C | 完成度 (history 表示 / dry-run / モーダル / --ascii) | +240 | 5.5h |
| **合計** | | **+990/-20** | **~17h** (2-3 日) |

### ROI 判断

| 観点 | TUI 採用 | indicatif で十分 |
|------|---------|----------------|
| **bat 9 本撤廃** | ◎ `note-auto tui` 一発 | △ shim 化済 (v0.7.6) で実害低 |
| **進捗可視化** | ◎ 5 stage + 7 source + tail log の panorama | ○ stderr に流れる行ベース、cmd でも見える |
| **daemon 監視** | ◎ TUI で別 pane で attach | ✗ 完全 black-box (logs/*.log を tail するのみ) |
| **学習コスト** | △ 操作覚える必要あり | ◎ 標準 CLI |
| **バイナリサイズ** | △ +2-3MB | ◎ +0 |
| **保守コスト** | △ ratatui rendering バグ対応必要 | ◎ stable |

### 推奨

- **Phase 2 着手対象に推奨**。ただし以下条件付き:
  1. **W7-A (静的 UI) を v0.8.0-rc1 として独立 PR にする**: 実装リスクを最小化、TUI が起動できる時点でリリース可能。
  2. **W7-B (実行統合) を v0.8.0-rc2** で。indicatif との backend trait 化は他フェーズへの副作用が小さく、安全。
  3. **W7-C (完成度) を v0.8.0 GA** で。ここまで来て初めて bat 撤廃宣言。
  4. **段階的 release tag**: `v0.8.0-rc1` / `v0.8.0-rc2` / `v0.8.0` で fail fast。
  5. **ratatui を `[features] tui = ["dep:ratatui"]` 化**: TUI 不要環境ではビルドサイズ最小化可能。

### 代替案 (もし TUI 不採用なら)

- **dialoguer ベースの interactive モード**: 設計案 v2 の Phase 2 で挙げた fzf 風カテゴリ multi-select。実装 1 日。bat 撤廃の 80% を低コストで達成。
- **TUI と並行で着手も可能**: dialoguer interactive (W8) を W7-A と並行リリース、ユーザの好みで使い分け。

---

## 8. 編集マトリクス (Phase 2 確定時の予定)

| ファイル | 編集者 | 変更内容 |
|---------|--------|---------|
| `src/cli/mod.rs` (新規) | ui-macos | `pub mod tui;` だけのファサード |
| `src/cli/tui.rs` (新規) | ui-macos | App / render / event loop / mpsc bridge |
| `src/display.rs` | ui-macos | `PipelineBackend` trait + `IndicatifBackend` / `TuiBackend` |
| `src/main.rs` | ui-macos | `Command::Tui` variant + dispatch |
| `Cargo.toml` | lowlevel | `ratatui = "0.28"` 追加 (option: `[features] tui = ["dep:ratatui"]`) |
| `src/writer/mod.rs` / `src/publish/mod.rs` | (触らず) | display::PipelineProgress 経由なので無変更 |
| `src/daemon.rs` | (触らず) | 既存 wire を流用 |
| `src/history.rs` | (触らず) | 既存 `History::load` を tui から read-only で呼ぶのみ |
| `README.md` | ui-macos | TUI 操作ガイド + キーバインド一覧 + Cascadia 推奨表記 |

衝突リスク: **ゼロ** (lowlevel は Cargo.toml のみ、quality は writer/publish 既存ロジック維持、ui-macos は新規 cli モジュールが主)。

---

## 9. 未確定 / commander 判断仰ぎ事項

1. **採用是非**: TUI を Phase 2 で実装するか、dialoguer interactive (W8) で軽量代替するか、両方やるか
2. **段階分割の粒度**: W7-A/B/C を 3 PR 分割でよいか、1 PR 大型でいくか
3. **`[features] tui` 化**: ratatui を default ON にするか、opt-in (`--features tui`) にするか
4. **daemon 並行ロック**: TUI 起動中に cron も走るときの対処 (R3 の対策案で OK か、別案あるか)
5. **既存 indicatif との切替方式**: `PipelineProgress::new()` (indicatif) と `PipelineProgress::new_tui()` (mpsc) の 2 系統で十分か、unified コンストラクタ + Theme から自動判定か

---

## 10. ロードマップ (採用前提)

```
Phase 2-A (1 週 / v0.8.0-rc1): W7-A 静的 UI
   ├─ ratatui 0.28 dep 追加 (lowlevel)
   ├─ cli/tui.rs 新規 (App + render + event loop)
   ├─ main.rs に Command::Tui 追加
   └─ smoke: 起動 → q 終了

Phase 2-B (1 週 / v0.8.0-rc2): W7-B 実行統合
   ├─ display.rs に PipelineBackend trait
   ├─ TuiBackend / IndicatifBackend 実装
   ├─ tracing → mpsc bridge
   └─ smoke: Enter で実 fetch → 進捗取り込み

Phase 2-C (3-5 日 / v0.8.0): W7-C 完成度
   ├─ History 表示 / dry-run トグル / ヘルプモーダル / --ascii 対応
   ├─ daemon 並行ロック (.lock file)
   └─ smoke: dry-run + scroll + リサイズ + cron 並走

Phase 2 完了 (v0.8.0 GA): bat 9 本撤廃宣言可能 + Cascadia Code 推奨を README で明記
```

---

> **次アクション**: 本設計案を commander → team-lead 上申 → 採用判断 + Phase 2 タスク表確定 → W7-A 実装 GO。
> 実装はそれまで保留。
