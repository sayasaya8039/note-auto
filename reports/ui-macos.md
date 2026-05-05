# note-auto UI/macOS風 設計案 v2

> **Status**: 設計確定待ち（実装は team-lead 統合プラン承認後）
> **Owner**: ui-macos teammate
> **Target version**: v0.7.6 (Phase 1 — CLI 改善のみ)
> **編集スコープ**: `src/display.rs`(新規) / `src/logging.rs`(拡張) / `README.md`(UI節追加) / `main.rs` は `mod display;` 追加と `display::xxx()` 置換のみ

---

## 1. 確定方針サマリ（commander 裁定 / 2026-05-05）

| # | 項目 | 確定値 |
|---|------|--------|
| 1 | リリース | v0.7.6 として独立 PR |
| 2 | bat 9本 | `note-auto run --category <name>` を呼ぶ薄い shim 化（Task Scheduler 互換維持） |
| 3 | 配色 | Big Sur dark + truecolor 検出 + `NO_COLOR` 対応 + `--ascii` フラグ併設 |
| 4 | TUI | DEFERRED（Phase 1 後にユーザ反応で再判断） |
| 5 | フォント | Cascadia Code 前提、`--ascii` で旧 cmd 用 fallback |
| 6 | 衝突回避 | `src/display.rs` 抽出方式（quality は main ロジック、ui-macos は表示層） |

---

## 2. 依存追加（Cargo.toml — lowlevel が一括コミット）

```toml
indicatif    = "0.17"   # MultiProgress / ProgressBar / ProgressStyle / spinner
console      = "0.15"   # Term capability detection (truecolor / NO_COLOR / TTY)
owo-colors   = "4"      # truecolor ANSI / supports_color::Stream 連携
comfy-table  = "7"      # rounded UTF-8 table / preset UTF8_FULL_CONDENSED
supports-color = "3"    # NO_COLOR / FORCE_COLOR / CI 自動判定（owo-colors と連携）
```

> **Note**: `dialoguer` / `ratatui` / `crossterm` は Phase 2/3 で追加。Phase 1 では含めない。

---

## 3. `src/display.rs` API 草案

### 3.1 モジュール構成

```
src/display.rs
├── palette       (struct/const) — Big Sur dark カラーパレット
├── glyphs        (struct/const) — Unicode/ASCII 二重定義のシンボル
├── theme         (struct)       — 実行時テーマ（capabilities + flag）
├── progress      (mod)          — MultiProgress 系ヘルパ
├── table         (mod)          — comfy-table プリセット
└── summary       (mod)          — RunSummary 整形
```

### 3.2 公開 API（関数シグネチャのみ — 実装は GO 後）

#### 初期化 / テーマ

```rust
/// Theme を一度だけ生成。CLI 引数 + 環境変数 + capability から決定する。
/// 呼び出し: main.rs エントリポイント直後で 1 回だけ。
pub fn init(opts: ThemeOptions) -> Theme;

#[derive(Clone, Copy, Default)]
pub struct ThemeOptions {
    pub force_ascii: bool,   // CLI --ascii で true
    pub force_color: Option<bool>, // CLI --color=always|never|auto（auto=None）
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub uses_color: bool,    // truecolor 可否（NO_COLOR 尊重）
    pub uses_unicode: bool,  // !force_ascii && stdout が UTF-8 capable
    pub is_tty: bool,        // 非 TTY（リダイレクト時）は spinner 抑止
}
```

決定ロジック（仕様）:

1. `--ascii` 指定 → `uses_unicode=false`
2. `NO_COLOR` 環境変数あり → `uses_color=false`（最優先・spec.org 準拠）
3. `console::Term::stdout().features().colors_supported()` && `supports_color::on(Stream::Stdout).has_basic` → `uses_color=true`
4. 非 TTY（パイプ/リダイレクト）→ spinner/プログレスバーを抑止、行ベース出力に切替

#### Palette（Big Sur dark）

```rust
pub mod palette {
    /// CSS-style truecolor 値。NO_COLOR 時は無視。
    pub const ACCENT:      Rgb = Rgb(0, 122, 255);   // #007AFF systemBlue
    pub const SUCCESS:     Rgb = Rgb(48, 209, 88);   // #30D158 systemGreen (Big Sur)
    pub const WARNING:     Rgb = Rgb(255, 159, 10);  // #FF9F0A systemOrange
    pub const ERROR:       Rgb = Rgb(255, 69, 58);   // #FF453A systemRed (Big Sur)
    pub const SECONDARY:   Rgb = Rgb(142, 142, 147); // #8E8E93 secondaryLabel
    pub const TERTIARY:    Rgb = Rgb(99, 99, 102);   // #636366 tertiaryLabel
    pub const BG_ELEVATED: Rgb = Rgb(44, 44, 46);    // #2C2C2E (informational)

    pub struct Rgb(pub u8, pub u8, pub u8);
}
```

8-bit fallback（uses_color=true && truecolor 不可時）:

| Truecolor | 8-bit ANSI | 名前 |
|-----------|-----------|------|
| #007AFF   | 33        | bright blue |
| #30D158   | 35 (green)→ 10 | bright green |
| #FF9F0A   | 214       | orange |
| #FF453A   | 203       | bright red |
| #8E8E93   | 245       | grey |

#### Glyphs（Unicode / ASCII 二重定義）

```rust
pub struct Glyphs {
    pub check:    &'static str, // "✓" / "OK"
    pub cross:    &'static str, // "✗" / "FAIL"
    pub warn:     &'static str, // "⚠" / "!"
    pub info:     &'static str, // "ⓘ" / "i"
    pub bullet:   &'static str, // "●" / "*"
    pub arrow:    &'static str, // "▸" / ">"
    pub dot_dim:  &'static str, // "·" / "."
    pub box_tl:   &'static str, // "╭" / "+"
    pub box_tr:   &'static str, // "╮" / "+"
    pub box_bl:   &'static str, // "╰" / "+"
    pub box_br:   &'static str, // "╯" / "+"
    pub h_line:   &'static str, // "─" / "-"
    pub v_line:   &'static str, // "│" / "|"
}
pub fn glyphs(theme: &Theme) -> Glyphs;
```

#### Progress

```rust
/// 全体オーケストレータ。RAII — drop で finish_and_clear。
pub struct PipelineProgress { /* MultiProgress + 5 stages */ }

pub fn pipeline_start(theme: &Theme) -> PipelineProgress;

impl PipelineProgress {
    /// 5 段階: fetch → score → write → publish → notify
    pub fn stage(&self, stage: Stage) -> StageBar;
    /// ソース別の子バー（fetch 段階で使用）
    pub fn source_bar(&self, source: SourceKind, total: u64) -> SourceBar;
    pub fn finish(self, summary: &RunSummary);
}

#[derive(Clone, Copy)]
pub enum Stage { Fetch, Score, Write, Publish, Notify }

#[derive(Clone, Copy)]
pub enum SourceKind {
    Note, X, Google, Hn, Konbini, Hyakkin, Gnews,
}

pub struct StageBar { /* ProgressBar */ }
impl StageBar {
    pub fn tick_msg(&self, msg: impl Into<String>);
    pub fn done(self);            // ✓ + accent 色
    pub fn fail(self, err: &str); // ✗ + error 色
}

pub struct SourceBar { /* ProgressBar */ }
impl SourceBar {
    pub fn inc(&self, delta: u64);
    pub fn set_message(&self, msg: impl Into<String>);
    pub fn done(self, fetched: usize);
}
```

#### Spinner プロファイル

| 用途 | tick_chars | interval(ms) | 備考 |
|------|-----------|--------------|------|
| `BRAILLE` (default) | `"⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"` | 80 | macOS 風 / indicatif 標準 |
| `DOTS_BIG_SUR` | `"●○○○ ○●○○ ○○●○ ○○○●"` (space区切り) | 120 | accent ドット流れ |
| `BEACHBALL` | `"◐◓◑◒"` | 100 | 旧 macOS BeachBall |
| `ASCII` (--ascii時) | `"|/-\\"` | 100 | 旧 cmd / NO_UNICODE |

ProgressStyle template 例（仕様のみ）:

```
{prefix:.bold.cyan} {spinner:.cyan} {msg:.dim} {elapsed:.dim}
```

ASCII 時:

```
[{prefix}] {spinner} {msg} ({elapsed})
```

#### Table

```rust
pub fn render_summary(theme: &Theme, summary: &RunSummary) -> String;

/// 列: # | Slug | Title | Chars | note | X | Δsec
/// preset: UTF8_FULL_CONDENSED / ASCII_MARKDOWN (--ascii)
/// アクセント列 (note/X status) は palette::SUCCESS / ERROR / SECONDARY で着色
```

#### Summary banner

```rust
pub fn print_banner(theme: &Theme, version: &str);          // 起動時 1 行ロゴ
pub fn print_done(theme: &Theme, summary: &RunSummary);     // 終了時 box + table
pub fn print_skipped(theme: &Theme, reason: &str);          // dry-run 時など
```

起動バナー仕様（uses_unicode=true）:

```
╭─ note-auto v0.7.6 ──────────────────────────────────────╮
│  ● fetch   ● score   ● write   ● publish   ● notify     │
╰──────────────────────────────────────────────────────────╯
```

ASCII 版:

```
+- note-auto v0.7.6 ---------------------------------------+
|  * fetch   * score   * write   * publish   * notify     |
+----------------------------------------------------------+
```

---

## 4. `src/logging.rs` 拡張仕様

### 4.1 二重出力（人間 / 機械）

| 出力先 | 層 | 用途 |
|-------|----|------|
| stdout (TTY) | `display::progress` 経由 | 人間向け視覚フィードバック |
| `logs/<timestamp>.log` | `tracing_subscriber::fmt::layer().json()` | 機械可読・既存 bat の log 蓄積互換 |
| stderr | `fmt::layer().compact()` (ERROR/WARN のみ) | パイプ時の最終手段 |

### 4.2 progress ブリッジ

`tracing` フィールド `stage="fetch" source="note"` を MakeWriter で拾い、`PipelineProgress` の対応 `StageBar.tick_msg()` に流す。

```rust
pub fn init(theme: &Theme, log_dir: &Path) -> tracing::subscriber::DefaultGuard;

// 内部:
//   - JSON layer    → file appender (rolling: hourly)
//   - Progress layer → display::PROGRESS_HANDLE (LazyLock<Mutex<...>>)
//   - Stderr layer  → ERROR/WARN のみ
```

`EnvFilter` は既存の `info,note_auto=debug` を維持。`RUST_LOG` 上書き可能。

### 4.3 既存 println! の取り扱い

- `main.rs` の `println!("✓ ...")` → `display::print_done(...)` に置換
- `tracing::info!(path = %p, count = n, "trends.json を出力")` → そのまま維持。progress layer が拾う。

---

## 5. `main.rs` 編集スコープ（衝突回避）

ui-macos が触れて良い行は **以下のみ**:

```rust
// 追加 (top of file):
mod display;

// init 直後（90 行目付近）:
let theme = display::init(display::ThemeOptions {
    force_ascii: cli.ascii,
    force_color: cli.color,
});
let _guard = logging::init(&theme, &cfg.log_dir);
display::print_banner(&theme, env!("CARGO_PKG_VERSION"));

// 各 println!("✓ ...") → display::print_xxx(&theme, ...) 置換
```

quality 担当領域（変更禁止）:

- match arm 内のロジック（trends::fetch_all / writer::run / publish::publish_all 呼出順序など）
- HTTP client 共有・dedup HashMap 等の最適化
- error path の context 追加

---

## 6. CLI 拡張仕様

### 6.1 グローバルフラグ追加

```rust
#[derive(Parser)]
struct Cli {
    #[arg(long, global = true)]
    ascii: bool,
    #[arg(long, value_enum, global = true, default_value = "auto")]
    color: ColorMode,  // Auto | Always | Never
    // ...既存
}
```

### 6.2 `run --category` サブコマンド（bat shim 受け）

```rust
Run {
    /// note | x | google | hn | konbini | hyakkin | gnews | all
    #[arg(long)]
    category: Option<String>,
    // ...既存 out/top/dry_run
}
```

`--category` 指定時の挙動:

1. `configs/<category>.toml` を強制適用（`--config` 上書き）
2. `cfg.schedule.daily_top` を category デフォルト値で上書き
3. display にカテゴリラベルを渡す（accent 色付け）

### 6.3 bat shim テンプレート（仕様）

```bat
@echo off
chcp 65001 > nul
cd /d "%~dp0"
if not exist logs mkdir logs
for /f ... LOGFILE 生成
".\target\release\note-auto.exe" run --category <CAT> >> "%LOGFILE%" 2>&1
exit /b %errorlevel%
```

差分は `<CAT>` 1 トークンのみ → 9 bat × 同一テンプレート。

---

## 7. テスト観点（仕様 — 実装後に追加）

| ケース | 期待 |
|--------|------|
| `NO_COLOR=1 cargo run -- once` | ANSI シーケンスゼロ、Unicode は維持 |
| `cargo run -- --ascii once` | Unicode シンボル/罫線が ASCII 化 |
| `cargo run -- --color=never once` | NO_COLOR と同等 |
| `cargo run -- once 2>&1 \| tee out.txt` | 非 TTY 検出 → spinner 抑止、行ログ |
| Cascadia Code 不在 cmd.exe | `--ascii` で読める前提 / Unicode 時は化け許容 |
| `RUST_LOG=trace` | JSON ファイル log に trace 流出、stdout は info 以上 |

---

## 8. ロールアウト チェックリスト（v0.7.6 PR）

- [ ] `Cargo.toml`: indicatif/console/owo-colors/comfy-table/supports-color 追加（lowlevel コミット）
- [ ] `src/display.rs` 新規（ui-macos）
- [ ] `src/logging.rs` 拡張（ui-macos）
- [ ] `src/main.rs`: `mod display;` + Cli フラグ + 置換のみ（quality 領域は触らない）
- [ ] `run-*.bat`: shim 化（9本 → 同一テンプレート + `<CAT>` のみ差分）
- [ ] `README.md`: "UI / macOS dark theme" 節追加（screenshot / NO_COLOR / --ascii 説明）
- [ ] バイナリサイズ計測（事前: 〜MB / 事後: 〜MB / 差: <1MB 目標）
- [ ] `cargo zigbuild --release -p note-auto` で CI 通過確認

---

## 9. 未確定 / team-lead 統合プラン待ち

1. **dialoguer 採用可否**（Phase 2 — `note-auto interactive` 用）— Phase 1 含めるか別 PR か
2. **logs/ rotation 戦略** — 既存は無制限蓄積。tracing-appender の hourly/daily どちらか
3. **JSON log のスキーマ固定**（{ts, level, stage, source, slug, ms, msg}）— 機械処理する側の要望次第
4. **Windows Terminal Profile 自動設定**（Cascadia Code フォントの導入手順）— README に手順記載のみで充分か
5. **multi-line spinner と Cascadia Code 等幅互換** — Braille 文字幅が cmd では化けるケース最終確認

これら 5 点は実装着手前に commander 経由で team-lead 裁定を受ける。

---

> **次アクション**: アイドル待機。lowlevel/quality レポート受信後、commander が統合 Phase 1 タスク表を作成 → team-lead 承認 → 実装 GO。
