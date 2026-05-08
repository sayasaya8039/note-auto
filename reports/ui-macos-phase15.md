# note-auto v0.7.7 (Phase 1.5) 設計案 v3 — UI 5 deps wire-up

> **Status**: 設計確定待ち（commander → team-lead 上申後に実装 GO）
> **Owner**: ui-macos teammate
> **Target version**: v0.7.7
> **編集スコープ**: `src/display.rs` 拡張のみ + `src/main.rs` の呼び出し置換のみ
> **依存**: 既に Cargo.toml に declare 済 (PR-A で追加) — `indicatif=0.17` / `console=0.15` / `owo-colors=4` / `comfy-table=7` / `supports-color=3`

---

## 1. ゴール

v0.7.6 で stdlib 実装した display.rs を、**API 互換を保ちつつ** 5 deps へ移行:

| ID | crate | 置換対象 (display.rs 内) |
|----|-------|------------------------|
| W1 | `indicatif` | `PipelineProgress` の自前 stderr 出力 → `MultiProgress` + `ProgressBar` |
| W2 | `comfy-table` | `print_done` の自前 UTF-8 罫線テーブル → `Table::with_preset` |
| W3 | `owo-colors` | `palette::Rgb` + `rgb_fg` 関数 → `owo_colors::Rgb` + `OwoColorize` トレイト |
| W4 | `console` | `Theme::detect` 内の `is_terminal` / unicode 検出 → `console::Term::stdout().features()` |
| W5 | `supports-color` | `truecolor_supported` 自前検出 → `supports_color::on(Stream::Stdout)` |

**API 互換性の原則**: `Theme` / `print_banner` / `print_check` / `print_done` / `glyphs` の呼び出し側 (main.rs / 将来の writer/publish) は **無変更**で動く。内部実装のみ差し替え。

---

## 2. 公開 API 拡張仕様

### 2.1 既存 API（v0.7.6 から維持）

呼び出し側の互換性のため、以下のシグネチャは**変更しない**:

```rust
pub fn init(opts: ThemeOptions) -> Theme;          // Theme::init
pub fn current() -> Theme;                          // Theme::current
pub fn glyphs(theme: &Theme) -> Glyphs;
pub fn parse_color_mode(s: &str) -> Result<ColorMode, String>;

pub fn accent(theme: &Theme, s: &str) -> String;
pub fn success(theme: &Theme, s: &str) -> String;
pub fn warning(theme: &Theme, s: &str) -> String;
pub fn error_color(theme: &Theme, s: &str) -> String;
pub fn dim(theme: &Theme, s: &str) -> String;
pub fn bold(theme: &Theme, s: &str) -> String;

pub fn print_banner(theme: &Theme, version: &str);
pub fn print_check(theme: &Theme, msg: &str);
pub fn print_warning(theme: &Theme, msg: &str);
pub fn print_error(theme: &Theme, msg: &str);
pub fn print_info(theme: &Theme, msg: &str);
pub fn print_skipped(theme: &Theme, reason: &str);
pub fn print_done(theme: &Theme, summary: &RunSummary);
```

内部実装のみ owo-colors / console / supports-color / comfy-table に差し替え。

### 2.2 新規追加 API（W1: indicatif Pipeline）

`PipelineProgress` を本格化。複数同時 task 対応。**writer/publish 側 (quality 担当) から呼ぶ public API はこれ**:

```rust
/// パイプライン全体の進捗オーケストレータ。
/// 1 度だけ生成し、`Arc` 経由で writer/publish/notify に渡す。
/// drop 時に `clear()` を自動実行。
pub struct PipelineProgress { /* MultiProgress + 5 stage bars */ }

impl PipelineProgress {
    /// `total_articles` は publish までの総記事数 (top-N)。
    pub fn new(theme: &Theme, total_articles: u64) -> Arc<Self>;

    // === Stage 1: Fetch (7 sources 並列) ===
    /// 各ソース用の子バーを取得。ソースがいない場合は no-op バー。
    pub fn fetch_source(&self, source: SourceKind) -> SourceBar;
    /// 全ソース完了後に呼ぶ。stage 行を ✓ で締める。
    pub fn fetch_done(&self, total_items: usize);

    // === Stage 2: Score ===
    pub fn score_start(&self, candidates: usize);
    pub fn score_done(&self, selected: usize);

    // === Stage 3: Write (記事ごと並列) ===
    /// 記事 1 本の進捗バー。AI 執筆 → 画像生成 → 保存の 3 phase を表現。
    pub fn write_article(&self, slug: &str, title: &str) -> ArticleBar;
    pub fn write_done(&self, written: usize);

    // === Stage 4: Publish (note + X 並列) ===
    pub fn publish_article(&self, slug: &str) -> PublishBar;
    pub fn publish_done(&self, summary: &RunSummary);

    // === Stage 5: Notify ===
    pub fn notify_start(&self);
    pub fn notify_done(&self);

    // === Termination ===
    /// 全バーを finish + summary table を stdout に出す。
    pub fn finish(&self, summary: &RunSummary);
    /// エラー時の中断。残りバーを赤で abandon。
    pub fn abort(&self, err: &str);
}

#[derive(Clone, Copy, Debug)]
pub enum SourceKind {
    Note, X, Google, Hn, Konbini, Hyakkin, Gnews,
}

/// 1 ソースの fetch 進捗
pub struct SourceBar { /* ProgressBar */ }
impl SourceBar {
    pub fn inc(&self, delta: u64);
    pub fn set_msg(&self, msg: impl Into<Cow<'static, str>>);
    pub fn done(self, fetched: usize);
    pub fn fail(self, err: &str);
}

/// 1 記事の write 進捗 (AI → image → save)
pub struct ArticleBar { /* ProgressBar */ }
impl ArticleBar {
    pub fn phase(&self, phase: WritePhase);
    pub fn done(self, char_count: usize);
    pub fn fail(self, err: &str);
}

#[derive(Clone, Copy, Debug)]
pub enum WritePhase { ResearchAi, BodyAi, Image, Save }

/// 1 記事の publish 進捗 (note + X)
pub struct PublishBar { /* ProgressBar */ }
impl PublishBar {
    pub fn note(&self, status: PublishStatus);
    pub fn x_post(&self, status: PublishStatus);
    pub fn done(self);
    pub fn fail(self, err: &str);
}

#[derive(Clone, Copy, Debug)]
pub enum PublishStatus { Pending, InProgress, Posted, Skipped, Failed }
```

**indicatif Style**:

```rust
// Stage bar (5 個):  [{prefix:.cyan} {wide_msg:.dim}] {pos}/{len}
// Source bar:        ↳ {prefix:.dim} {spinner:.cyan} {wide_msg:.dim} {pos}/{len}
// Article bar:       ↳ {prefix:.dim} {bar:30.green/dim} {percent}% {wide_msg}
// Spinner: tick_strings = ["⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"] (--ascii で "|/-\\")
// 描画 fps: 12 (250ms)、--no-tty 時は完全静音
```

### 2.3 新規追加 API（W2: comfy-table）

```rust
/// 公開 API は維持しつつ内部実装を comfy-table へ置換。
pub fn print_done(theme: &Theme, summary: &RunSummary);

/// scoring 段階で選定された記事の表（main.rs / writer から呼ぶ）
pub fn print_scoring_table(theme: &Theme, selected: &[scoring::SelectedTrend]);

/// 任意の表を自由に組む下位 API（quality 担当が writer 内で使う想定）
pub struct SummaryRow {
    pub idx: usize,
    pub slug: String,
    pub title: String,
    pub chars: usize,
    pub note_status: String,
    pub x_status: String,
    pub elapsed_ms: u64,
}
pub fn render_summary_table(theme: &Theme, rows: &[SummaryRow]) -> String;
```

**comfy-table preset**:
```
- theme.uses_unicode = true:  Preset::UTF8_FULL_CONDENSED + ContentArrangement::Dynamic
- theme.uses_unicode = false: Preset::ASCII_MARKDOWN
- 列ヘッダ: bold + accent
- アクセント列 (note/X status): success/error/dim 着色
```

### 2.4 W3: owo-colors 統合

```rust
// 内部
use owo_colors::{OwoColorize, Rgb, Style};

pub mod palette {
    use owo_colors::Rgb;
    pub const ACCENT:    Rgb = Rgb(0, 122, 255);
    pub const SUCCESS:   Rgb = Rgb(48, 209, 88);
    pub const WARNING:   Rgb = Rgb(255, 159, 10);
    pub const ERROR_C:   Rgb = Rgb(255, 69, 58);
    pub const SECONDARY: Rgb = Rgb(142, 142, 147);
    pub const TERTIARY:  Rgb = Rgb(99, 99, 102);
}

// 既存 accent/success/warning/error_color/dim/bold は中身を:
//   if !theme.uses_color { s.to_string() }
//   else if theme.truecolor { s.color(palette::ACCENT).to_string() }
//   else { s.bright_blue().to_string() }   // 8-bit fallback
// に書き換え。シグネチャ不変。
```

### 2.5 W4: console 統合

```rust
// 内部
use console::Term;

fn detect_tty() -> bool {
    Term::stdout().is_term()
}
fn detect_unicode_capable() -> bool {
    Term::stdout().features().wants_emoji() || cfg!(unix)
    // Windows Terminal は wants_emoji() = true、ConHost は false
}
```

### 2.6 W5: supports-color 統合

```rust
// 内部
use supports_color::{on, Stream};

fn detect_truecolor() -> bool {
    on(Stream::Stdout).map(|s| s.has_16m).unwrap_or(false)
}
fn detect_color() -> bool {
    on(Stream::Stdout).is_some()
    // NO_COLOR は supports-color が自動判定
}
```

`Theme::detect` は以下の流れに置換:

```rust
fn detect(opts: ThemeOptions) -> Self {
    let is_tty = console::Term::stdout().is_term();

    let uses_color = match opts.color {
        ColorMode::Always =>
            !std::env::var_os("NO_COLOR").map(|v| !v.is_empty()).unwrap_or(false),
        ColorMode::Never  => false,
        ColorMode::Auto   => supports_color::on(Stream::Stdout).is_some(),
    };

    let truecolor = uses_color
        && supports_color::on(Stream::Stdout).map(|s| s.has_16m).unwrap_or(false);

    let uses_unicode = !opts.force_ascii
        && (console::Term::stdout().features().wants_emoji() || cfg!(unix));

    Theme { uses_color, truecolor, uses_unicode, is_tty }
}
```

---

## 3. quality (writer/publish) からの呼び出し例 — wire 仕様

quality 担当が `writer::run` / `publish::publish_all` をどう拡張するかの提示。display.rs の API は**この呼び出しを支える**ように設計する。

### 3.1 writer::run wire 例

```rust
// src/writer/mod.rs (quality が編集)
pub async fn run(
    cfg: &Config,
    trends: &[scoring::SelectedTrend],
    out_dir: &Path,
    progress: &display::PipelineProgress,  // ← 追加引数
) -> Result<Vec<WrittenArticle>> {
    progress.score_done(trends.len());

    let futures = trends.iter().map(|t| {
        let bar = progress.write_article(&t.slug, &t.title);
        async move {
            bar.phase(WritePhase::ResearchAi);
            // ... grok research ...
            bar.phase(WritePhase::BodyAi);
            // ... opus write ...
            bar.phase(WritePhase::Image);
            // ... image generation ...
            bar.phase(WritePhase::Save);
            // ... save ...
            let article = ...;
            bar.done(article.char_count);
            Ok(article)
        }
    });
    let results = futures::future::try_join_all(futures).await?;
    progress.write_done(results.len());
    Ok(results)
}
```

### 3.2 publish::publish_all wire 例

```rust
// src/publish/mod.rs (quality が編集)
pub async fn publish_all(
    cfg: &Config,
    articles: &[WrittenArticle],
    progress: &display::PipelineProgress,  // ← 追加引数
) -> Result<Vec<PublishResult>> {
    let futures = articles.iter().map(|a| {
        let bar = progress.publish_article(&a.slug);
        async move {
            bar.note(PublishStatus::InProgress);
            let note_url = note::publish(cfg, a).await;
            bar.note(if note_url.is_ok() { PublishStatus::Posted } else { PublishStatus::Failed });

            bar.x_post(PublishStatus::InProgress);
            let x_url = x_post::announce(cfg, a).await;
            bar.x_post(if x_url.is_ok() { PublishStatus::Posted } else { PublishStatus::Failed });

            bar.done();
            Ok(PublishResult { ... })
        }
    });
    futures::future::try_join_all(futures).await
}
```

### 3.3 trends::fetch_all wire 例

```rust
// src/trends/mod.rs (quality が編集)
pub async fn fetch_all(
    cfg: &Config,
    progress: &display::PipelineProgress,  // ← 追加引数
) -> Result<Vec<TrendItem>> {
    let mut handles = Vec::new();

    if cfg.sources.note {
        let bar = progress.fetch_source(SourceKind::Note);
        handles.push(tokio::spawn(async move {
            // ...fetch...
            bar.done(items.len());
            items
        }));
    }
    // 同様に x / google / hn / konbini / hyakkin / gnews

    let all = futures::future::join_all(handles).await;
    progress.fetch_done(all.len());
    Ok(all.into_iter().flatten().collect())
}
```

### 3.4 main.rs / daemon.rs 側 (ui-macos が編集)

```rust
// Once コマンドや daemon::execute_cycle 内
let progress = display::PipelineProgress::new(&theme, cfg.schedule.daily_top as u64);

let trends = trends::fetch_all(&cfg, &progress).await?;
let selected = scoring::select_top(trends, top);
let written = writer::run(&cfg, &selected, &out_dir, &progress).await?;
let results = publish::publish_all(&cfg, &written, &progress).await?;
progress.notify_start();
publish::notify_summary(&cfg, &summary).await.ok();
progress.notify_done();
progress.finish(&summary);  // ← ここで comfy-table の summary が stdout に出る
```

---

## 4. lowlevel (scoring) からの呼び出し例 — wire 仕様

scoring は同期処理で 1 ファイル完結。progress を介さず、scoring 完了後に main から `display::print_scoring_table` を呼ぶ形でも十分。lowlevel 編集ゾーンは scoring.rs のみなので、表示は main.rs に任せる:

```rust
// src/scoring.rs (lowlevel が編集) — display 依存しない
pub fn select_top(items: Vec<TrendItem>, top: usize) -> Vec<SelectedTrend> { ... }

// src/main.rs (ui-macos が編集) — scoring 結果を表示
let selected = scoring::select_top(items, top);
display::print_scoring_table(&theme, &selected);
progress.score_done(selected.len());
```

---

## 5. ファイル別変更スコープ

| ファイル | 編集者 | 変更内容 |
|---------|--------|---------|
| `src/display.rs` | ui-macos | `#![allow(dead_code)]` 削除、5 deps 統合、新 API (PipelineProgress 等) 追加 |
| `src/main.rs` | ui-macos | `progress` 生成 + 各サブコマンドへの渡し、`print_scoring_table` 呼び出し |
| `src/daemon.rs` | ui-macos (display 呼び出しのみ) | `execute_cycle` 内で progress 生成 + 渡し |
| `src/trends/mod.rs` | quality | `fetch_all(&cfg, progress)` シグネチャ拡張 |
| `src/writer/mod.rs` | quality | `run(&cfg, trends, out_dir, progress)` シグネチャ拡張、各 phase で bar.phase() |
| `src/publish/mod.rs` | quality | `publish_all(&cfg, articles, progress)` シグネチャ拡張、note/X で bar 更新 |
| `src/scoring.rs` | lowlevel | display 非依存、ロジックのみ。scoring_table は main.rs 側で呼ぶ |
| `Cargo.toml` | lowlevel | deps バージョン更新のみ (大規模変更なし) |

⚠ ui-macos は writer/publish/trends のロジック内部 (LLM 呼び出し / HTTP / parse) には触らない。
⚠ quality は display.rs の中身には触らない。`progress` 引数を受けて API を呼ぶだけ。

---

## 6. 段階的ロールアウト戦略

5 deps を**全部一気に置換するとリスク大**なので、3 段階に分割推奨:

### Phase A (最小・低リスク・1 PR): W4 + W5 + W3
- `Theme::detect` の console + supports-color 統合
- `palette` の `owo_colors::Rgb` 化 + accent/success 等の中身置換
- 既存 `print_*` API 完全維持、外部から見て無変更
- 検証: smoke 5 種 (`--ascii` / `NO_COLOR=1` / `--color=always` / `--color=auto` / 通常) 全通過
- これで Cargo.toml 5 deps のうち 3 つが「実利用」になる

### Phase B (中・1 PR): W2 (comfy-table)
- `print_done` の自前 box 描画 → `Table::with_preset(UTF8_FULL_CONDENSED)`
- `print_scoring_table` 新規追加
- 検証: 表のレイアウトが Cascadia Code / cmd.exe (--ascii) で崩れないこと

### Phase C (大・1 PR): W1 (indicatif)
- `PipelineProgress` 全面書き換え + writer/publish の wire (quality 共同作業)
- 検証: tokio 並列 fetch でバー更新が racy にならないこと、非 TTY で完全静音

合計 3 PR、各 200〜400 行規模、1 週間程度の見込み。

---

## 7. テスト計画

### 7.1 Phase A 完了時
- `cargo test --lib` で display 単体 7 + 新規 6 (palette truecolor / 8bit fallback / TTY 検出 / NO_COLOR / wants_emoji / ColorMode)
- smoke `note-auto --version` (起動時 Theme detect が落ちない)
- smoke `NO_COLOR=1 note-auto notify` (色なし)
- smoke `note-auto --ascii notify` (Unicode → ASCII)

### 7.2 Phase B 完了時
- 上記 + smoke `note-auto publish --from drafts/.../articles.json --dry-run` で table 出力検証
- comfy-table の最大幅が 120 cols 超えても破綻しないこと (列を省略表示)

### 7.3 Phase C 完了時
- 上記 + smoke `note-auto once --top 3 --dry-run` で並列 7 source / 3 article 進捗が見えること
- 非 TTY (`note-auto once 2>&1 | tee out.log`) で進捗バーが完全に消えること
- `RUST_LOG=trace` 時に tracing イベントが progress に干渉しないこと

---

## 8. 既知のリスク

1. **indicatif と tracing の出力競合**: tracing は stderr、indicatif も stderr。`indicatif_log_bridge` を使うか、tracing を非干渉化する必要。Phase C で対応。
2. **comfy-table の最大幅**: terminal width が小さい (50 cols 等) と列がカット。`ContentArrangement::Dynamic + Disabled.set_width(min, max)` で抑制。
3. **owo-colors の no_color サポート**: owo-colors 4.x は `if_supports_color` を使うと自動で NO_COLOR 尊重するが、`Theme.uses_color` で明示制御するため不要。
4. **console の Windows ConHost 検出**: 旧 cmd.exe 時に `wants_emoji() = false` → ASCII fallback が自動で効く想定。要実機テスト。
5. **Cargo.toml バージョン**: PR-A で declare 済の 5 deps。lowlevel が necessary なら patch バージョンを上げる程度。

---

## 9. 未確定 / commander 判断仰ぎ事項

1. **段階分割**: Phase A → B → C の 3 PR で良いか? 一気に 1 PR (大規模 review) でやるか?
2. **quality との同時編集**: writer/publish の wire は quality と同期して進める必要。タスク表で Phase C に「ui-macos: API 提供」「quality: 呼び出し wire」を並列 task にするか、逐次にするか。
3. **`#![allow(dead_code)]` 剥がしタイミング**: Phase A で wire 完了する API はそこで剥がす vs Phase C で全部剥がす。前者なら Phase B/C で再追加が発生する可能性。
4. **PipelineProgress 渡し方**: 引数で配るか (本案) / `Arc<OnceLock<PipelineProgress>>` で global に置くか。本案は引数派 (関数純度高)。
5. **scoring の表示位置**: 本案では main.rs 側で呼ぶが、scoring.rs 内で display を呼んでも良い (lowlevel 領域だが lib への依存が増える)。

これら 5 点は実装着手前に commander 経由で team-lead 裁定を受ける。

---

## 10. ロードマップ（参考）

```
Phase 1.5-A (3〜5日 / v0.7.7-rc1): W4 + W5 + W3
   ├─ Theme::detect を console + supports-color 化
   ├─ Palette を owo_colors::Rgb 化
   └─ print_* 内部の ANSI escape 直書き → owo-colors 経由

Phase 1.5-B (3〜5日 / v0.7.7-rc2): W2
   ├─ print_done を comfy-table 化
   ├─ print_scoring_table 新規
   └─ render_summary_table 公開 API

Phase 1.5-C (5〜7日 / v0.7.7): W1
   ├─ PipelineProgress 本格化 (MultiProgress + ProgressBar 多段)
   ├─ writer/publish/trends に progress 引数追加 (quality 連携)
   ├─ indicatif_log_bridge で tracing 干渉解消
   └─ #![allow(dead_code)] 完全削除

Phase 1.5 完了 (v0.7.7 GA): smoke 全通過 + 5 deps 全 wire-up + warning ≤ 7 件
```

---

> **次アクション**: 設計案 v3 を commander へ提出 → team-lead 裁定 → Phase A 着手 GO。実装はそれまで保留。
