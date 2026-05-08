# Phase 3 (v0.9.0) ui-macos 設計レポート — TUI 拡張続編

> **Status**: 設計確定待ち（commander → team-lead 上申後に実装 GO）
> **Owner**: ui-macos teammate
> **Target version**: v0.9.0
> **編集スコープ予定**: `src/cli/tui.rs` (主) + `src/display.rs` (PipelineBackend 拡張) + `src/logging.rs` (W7-D で indicatif-log-bridge 統合) + `src/main.rs` (Cli フラグ追加なら) + `Cargo.toml` (依存 1 件追加)
> **依存追加**: `indicatif-log-bridge = "0.2"` 1 件のみ（W7-D 採択時）

---

## 1. 候補から選択する 3 機能 (推奨)

候補 7 件を ROI で評価し、以下の **3 機能** を v0.9.0 で実装することを推奨:

| # | 機能 | ROI | 採択理由 |
|---|------|-----|---------|
| **W7-D** | indicatif-log-bridge 統合 (候補 3) | ★★★★★ | tracing log × ratatui/indicatif 競合は Phase 2 末から実機懸案。CLI/TUI 両モードに効果、依存 1 件で済む |
| **W7-E** | 進捗バー視覚改善 (候補 4) | ★★★★ | 設計案 v3 で deferred した「5 stage × 7 source bar」本格実装、TUI が「ratatui の本領発揮」段階に到達 |
| **W7-F** | エラー詳細 modal (候補 7) | ★★★ | P4 で実装済の `?` モーダル機構を流用、軽量・即体感価値、debug 効率改善 |

不採択 (Phase 4 以降に持ち越し):
- 候補 1 `c` config reload — 頻度低、cron daemon との同期が複雑
- 候補 2 `s` switch source — running 中断のクリーン shutdown 設計が重い
- 候補 5 dark/light theme 切替 — Big Sur dark 一本化で済む、light 需要小
- 候補 6 History 検索フィルタ — Logs ペインで grep ライク機能と統合した方が筋が良い (Phase 4 候補)

---

## 2. 各機能の設計

### W7-D: indicatif-log-bridge 統合 (~3h)

#### 動機

v0.8.1 までで判明している UX 課題:
- `note-auto fetch-trends` 等で indicatif の MultiProgress が描画中に `tracing::info!` イベントが発火すると、progress bar が tracing log で消される (cursor 移動の競合)
- TUI モードでは tracing が stderr に直書きするため alt screen と干渉しないが、`AppEvent::Log` 経由で TUI 内に取り込む経路がない (v0.8.1 まで未実装)

#### 設計

**Cargo.toml:**
```toml
indicatif-log-bridge = "0.2"
```

**`src/logging.rs` 拡張:**

```rust
// 新規ファクトリ: indicatif の MultiProgress を tracing layer に bridge
pub fn init_with_progress(theme: &Theme, multi: indicatif::MultiProgress) {
    use indicatif_log_bridge::LogWrapper;
    use tracing_subscriber::fmt::format::FmtSpan;

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,note_auto=debug"));

    let stderr_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_ansi(theme.uses_color)
        .with_span_events(FmtSpan::CLOSE);

    let subscriber = tracing_subscriber::registry().with(filter).with(stderr_layer);

    // LogWrapper で MultiProgress と協調 — log 行は MultiProgress::println 経由で
    // progress bar を一時 suspend して描画される
    LogWrapper::new(multi, subscriber).try_init().ok();
}

// 既存 init_with(&Theme) は維持 (TUI モード以外)
```

**`src/display.rs::IndicatifBackend::new(theme)` 拡張:**

```rust
impl IndicatifBackend {
    pub fn new(theme: Theme) -> Self {
        let multi = MultiProgress::new();
        // ... bars 構築 ...

        // W7-D: tracing と協調するため logging を MultiProgress 経由で初期化し直す
        crate::logging::init_with_progress(&theme, multi.clone());

        Self { multi, bars, theme }
    }
}
```

**`src/cli/tui.rs` 拡張 (TUI モード用 tracing→AppEvent::Log forwarding):**

```rust
// 起動時に tracing layer を 1 つ追加し、AppEvent::Log に forward
struct TuiTracingLayer {
    tx: mpsc::UnboundedSender<AppEvent>,
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for TuiTracingLayer {
    fn on_event(&self, event: &Event, _ctx: Context<S>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        if let Some(msg) = visitor.message {
            let line = format!("{} {}", event.metadata().level(), msg);
            let _ = self.tx.send(AppEvent::Log(line));
        }
    }
}

// AppEvent に Log バリアント追加
enum AppEvent {
    Key(KeyEvent),
    Tick,
    Pipeline(PipelineUpdate),
    Log(String),  // ← 新規
    WorkerDone(Result<String, String>),
}
```

#### 影響範囲

- `Cargo.toml`: +1 dep
- `src/logging.rs`: +30 行 (init_with_progress + 既存 init_with は維持)
- `src/display.rs`: +5 行 (IndicatifBackend::new で logging 初期化を呼ぶ)
- `src/cli/tui.rs`: +50 行 (TuiTracingLayer + AppEvent::Log)

**API 互換性**: 既存呼出 (`logging::init_with(&theme)`) は無変更で動く。`init_with_progress` は新規 API。

---

### W7-E: 進捗バー視覚改善 (~6h)

#### 動機

Phase 2 設計案 v3 で予定していた **「5 stage × 7 source bar」本格実装** を Phase 3 で完遂。現状 v0.8.1:

- 5 stage 各 1 bar のみ
- writer 内部の article 別進捗は不可視 (3 件並列で書いてもバーは 1 つ)
- publish 内部の note + X 並列も同様

#### 設計

**`src/display.rs` 拡張 — `PipelineBackend` trait に sub-bar API 追加:**

```rust
pub trait PipelineBackend: Send + Sync {
    // 既存 (v0.7.7〜v0.8.1)
    fn stage_start(&self, stage: Stage, msg: &str);
    fn stage_done(&self, stage: Stage, msg: &str);
    fn stage_fail(&self, stage: Stage, err: &str);
    fn finish(&self);

    // W7-E 追加: 子 bar を返す (default impl で no-op、IndicatifBackend と TuiBackend で実装)
    fn source_bar(&self, _stage: Stage, _source: &str) -> Box<dyn SubBar> {
        Box::new(NoopSubBar)
    }
    fn article_bar(&self, _stage: Stage, _slug: &str, _phase: &str) -> Box<dyn SubBar> {
        Box::new(NoopSubBar)
    }
}

pub trait SubBar: Send + Sync {
    fn tick(&self, msg: &str);
    fn done(&self, msg: &str);
    fn fail(&self, err: &str);
}

struct NoopSubBar;
impl SubBar for NoopSubBar {
    fn tick(&self, _: &str) {}
    fn done(&self, _: &str) {}
    fn fail(&self, _: &str) {}
}
```

**IndicatifBackend 実装:**

```rust
impl PipelineBackend for IndicatifBackend {
    fn source_bar(&self, stage: Stage, source: &str) -> Box<dyn SubBar> {
        let pb = self.multi.add(ProgressBar::new_spinner());
        pb.set_style(/* indented 子 style */);
        pb.set_prefix(format!("    └─ {source}"));
        Box::new(IndicatifSubBar { pb })
    }
    // article_bar も同様
}
```

**TuiBackend 実装:**

```rust
// PipelineUpdate に SubStart/SubTick/SubDone/SubFail を追加
pub enum PipelineUpdate {
    StageStart { stage: Stage, msg: String },
    StageDone { stage: Stage, msg: String },
    StageFail { stage: Stage, err: String },
    SubStart { stage: Stage, label: String },
    SubTick { stage: Stage, label: String, msg: String },
    SubDone { stage: Stage, label: String, msg: String },
    SubFail { stage: Stage, label: String, err: String },
    Finished,
}
```

**`src/cli/tui.rs` 拡張 — Pipeline ペインに sub-bar 表示:**

```rust
struct App {
    // 既存
    pipeline: [StageState; 5],
    // W7-E 追加: 各 stage に紐づく sub-bar 群
    sub_bars: HashMap<(Stage, String), SubBarState>,
}
```

**writer/publish 側の wire (quality 連携必要):**

```rust
// src/writer/mod.rs
pub async fn run(
    cfg: &Config,
    trends: &[SelectedTrend],
    out_dir: &Path,
    progress: Option<&PipelineProgress>,  // 設計案 v3 で予定していた追加
) -> Result<Vec<WrittenArticle>> {
    let bars: Vec<_> = trends.iter().map(|t| {
        progress.map(|p| p.article_bar(Stage::Write, &t.slug, "AI"))
    }).collect();
    // ... article ごとに bars[i].tick("LLM 呼び出し中") などを呼ぶ
}
```

#### 影響範囲

- `src/display.rs`: +120 行 (SubBar trait + IndicatifSubBar + TuiSubBar)
- `src/cli/tui.rs`: +80 行 (sub_bars state + render 拡張)
- `src/writer/mod.rs`: +30 行 (article_bar wire) — **quality 領域への介入、要協調**
- `src/publish/mod.rs`: +25 行 (note/X 別 bar wire) — **同上**
- `src/trends/mod.rs`: +20 行 (source 別 bar wire) — **同上**

**API 互換性**: `PipelineBackend` の新メソッドは default impl 付きなので既存実装に影響なし。writer/publish/trends の追加引数は `Option<&PipelineProgress>` で None 許容なので、main.rs::Once などからは `None` で呼べる。daemon::execute_cycle_with_progress からは `Some(&progress)` で渡す。

**quality 連携**: writer/publish 内部で `bar.tick()` を呼ぶのはロジックではないが、ループの構造把握が必要。Phase 2 と同様、ui-macos が引数追加 + tick 呼び出しを行い、quality は内部ロジック触らずに済む形を堅持。

---

### W7-F: エラー詳細 modal (~2h)

#### 動機

現状 v0.8.1:
- `StageFail` の error message は Pipeline ペインに `"failed: <err>"` として 1 行で表示
- 長いエラー (HTTP body / stack trace) は省略される
- Logs ペインにも同じ 1 行のみ

ユーザが debug したいときは log file (`logs/<date>_<time>.log`) を別途開く必要がある。TUI 内で完結させたい。

#### 設計

**`App` 拡張:**

```rust
struct App {
    // 既存 + P4
    help_overlay: bool,

    // W7-F 追加: 最新の StageFail / WorkerDone Err を保持
    error_detail: Option<ErrorDetail>,
    error_overlay: bool,
}

struct ErrorDetail {
    stage: Option<Stage>,
    msg: String,
    timestamp: chrono::DateTime<chrono::Local>,
}
```

**handle_key:**

```rust
KeyCode::Char('e') => {
    // error が記録されていれば overlay トグル
    if app.error_detail.is_some() {
        app.error_overlay = !app.error_overlay;
    } else {
        app.push_log("(エラー履歴なし)".to_string());
    }
}
```

**apply_update:**

```rust
PipelineUpdate::StageFail { stage, err } => {
    app.error_detail = Some(ErrorDetail {
        stage: Some(stage),
        msg: err.clone(),
        timestamp: chrono::Local::now(),
    });
    // 既存の Logs / Pipeline ペイン更新も継続
}
```

**WorkerDone Err 受信時:**

```rust
AppEvent::WorkerDone(Err(err)) => {
    app.error_detail = Some(ErrorDetail {
        stage: None,
        msg: err.clone(),
        timestamp: chrono::Local::now(),
    });
}
```

**render — P4 の `render_help_overlay` を流用してエラー modal:**

```rust
fn render_error_overlay(f: &mut Frame, app: &App) {
    if let Some(detail) = &app.error_detail {
        let modal_w = (f.area().width as f32 * 0.7) as u16;
        let modal_h = (f.area().height as f32 * 0.6) as u16;
        // ... Clear + Block (border ERROR_C) + Paragraph (wrap=true で多行)
        // 内容: timestamp / stage / err.lines() を 1 行ずつ
    }
}
```

**ステータスバー追加:**

```rust
// `e` Error-detail ヒントを追加 (error_detail があれば赤、なければ dim)
```

#### 影響範囲

- `src/cli/tui.rs`: +60 行 (App 拡張 + handle_key + render_error_overlay + status bar 拡張)

**API 互換性**: 完全に display モジュール内のみで完結、外部影響ゼロ。

---

## 3. 段階分割提案

### 推奨: **3 PR に分離 (W7-D / W7-E / W7-F)**

理由:
- W7-D (indicatif-log-bridge): logging.rs / display.rs / cli/tui.rs 触る、CLI/TUI 両モード効果、独立テスト可能
- W7-E (sub-bar): writer/publish/trends に介入する **quality 連携 PR**、最大規模、独立 review 推奨
- W7-F (error modal): cli/tui.rs のみ単独編集、軽量、即体感価値、独立 PR で fast-track 可能

### 分離 PR の組合せ

| PR | 担当ファイル | 行数 | 工数 | 衝突 | 自走可否 |
|----|------------|------|------|------|---------|
| **PR-N (W7-D)** | Cargo.toml / logging.rs / display.rs / cli/tui.rs | +90/-5 | ~3h | ゼロ (全 ui-macos 領域) | 4 条件で自走可 |
| **PR-N+1 (W7-E)** | display.rs / cli/tui.rs / writer/mod.rs / publish/mod.rs / trends/mod.rs | +275/-30 | ~6h | quality 連携 | quality 同時 review が望ましい、自走条件付き |
| **PR-N+2 (W7-F)** | cli/tui.rs のみ | +60/-5 | ~2h | ゼロ | 4 条件で自走可 |

合計 **~11h、3 PR、+425/-40**。

### 一括 1 PR 案

- 上記 3 機能を 1 PR にまとめる: ~11h、+425/-40、review 重い、ロールバック粒度粗い
- W7-D + W7-F のみまとめて 1 PR (~5h、+150/-10): W7-E の quality 連携を分離したい場合に妥当

### 推奨 vs 代替

**推奨: 3 PR 分離**。Phase 1.5/2/2.5 で 4 条件自走承認パターンが確立されているため、PR 数が増えても review 負担は線形以下。W7-E が遅延しても W7-D / W7-F は v0.9.0 リリースに間に合う。

---

## 4. 既存 v0.8.1 との関係

### API 互換性

- **W7-D**: `logging::init_with(&theme)` は無変更維持、`init_with_progress` を新規追加。既存 main.rs / daemon.rs 呼出は変更なし。
- **W7-E**: `PipelineBackend` の追加メソッドは default impl 付き、既存 IndicatifBackend / TuiBackend 拡張のみ。writer/publish/trends に追加引数 `Option<&PipelineProgress>` は None で既存挙動。
- **W7-F**: cli/tui.rs 内のみで完結、外部 API 変更ゼロ。

### worktree 戦略 + pre-push hook 継続

- v0.8.1 で確立した 3 重ガード (worktree 隔離 + branch protection + pre-push hook) を Phase 3 でも継続
- `note-auto-tui` worktree を再利用、ブランチは `ui/v0.9.0-tui-d` / `-tui-e` / `-tui-f` で切替

### v4.1 二重確認の習慣化

- 各 PR の commit/push 前に `git status -sb` + `git branch --show-current` を必ず実行、完了報告に明示

---

## 5. 想定リスク 3 項目 + 対策

### R1: indicatif-log-bridge × Windows ConHost 互換性
- **症状**: 旧 cmd.exe 上で MultiProgress + LogWrapper の cursor 制御が破綻、log と progress bar が二重描画される可能性
- **対策**:
  - W7-D の動作対象を `console::Term::stdout().is_term()` && `wants_emoji()` で gate
  - 非対応環境では既存 `init_with(&theme)` を継続 (tracing は stderr 直書き、indicatif は MultiProgress::println 経由ではない)
  - smoke test に `chcp 850` (旧コードページ) ケース追加

### R2: W7-E の writer/publish 連携で quality と仕様齟齬
- **症状**: `Option<&PipelineProgress>` 引数を ui-macos が writer に追加 → quality が並行で writer ロジック修正中だと衝突 → rebase conflict
- **対策**:
  - W7-E 着手前に commander 経由で quality に「writer/mod.rs / publish/mod.rs シグネチャ拡張予定」を通知、同時編集を回避
  - 万一 conflict 出たら、ui-macos は引数追加のみ反映 (内部ロジック変更は quality に委ねる) で merge
  - Phase 2 設計案 v3 と同じく、ui-macos = 引数追加 + tick 呼び出しのみ、quality = 内部ロジック堅持

### R3: W7-E sub-bar の MultiProgress 描画パフォーマンス
- **症状**: 7 source × 5 stage = 最大 35 個のバーが同時 tick → terminal 描画が CPU バウンドになりレスポンス低下
- **対策**:
  - SubBar は **stage 終了時に finish()** で消えるよう実装 (active 時のみ表示、完了済は折り畳む)
  - tick 周期を `enable_steady_tick(80ms)` から `120ms` に伸ばす (既存 stage bar とずらす)
  - bench で並列タスク数 7 まで tick 周波数を実測、必要なら adaptive throttling

---

## 6. 編集マトリクス (Phase 3 確定時の予定)

| ファイル | 編集者 | 変更内容 |
|---------|--------|---------|
| `src/cli/tui.rs` | ui-macos | App 拡張 / sub-bars state / error modal / AppEvent::Log |
| `src/display.rs` | ui-macos | SubBar trait / IndicatifSubBar / TuiSubBar / PipelineUpdate 拡張 |
| `src/logging.rs` | ui-macos | `init_with_progress(&theme, multi)` 追加 (W7-D) |
| `src/main.rs` | ui-macos (極小、必要時) | TUI 起動時の logging 初期化切替 (W7-D) |
| `src/writer/mod.rs` | **ui-macos が引数追加**、quality は内部ロジック堅持 | `Option<&PipelineProgress>` 引数 + article_bar wire (W7-E) |
| `src/publish/mod.rs` | 同上 | `Option<&PipelineProgress>` 引数 + note/X bar wire (W7-E) |
| `src/trends/mod.rs` | 同上 | `Option<&PipelineProgress>` 引数 + source_bar wire (W7-E) |
| `Cargo.toml` | lowlevel (W7-D で +1 dep) | `indicatif-log-bridge = "0.2"` 追加 |

衝突リスク:
- **W7-D**: ゼロ (全 ui-macos 領域、Cargo.toml は lowlevel が dep 追加)
- **W7-E**: writer/publish/trends に介入、Phase 2 設計案 v3 で承認された方式 (ui-macos = 引数追加、quality = 内部不可侵)。事前通知で rebase conflict 回避
- **W7-F**: ゼロ (cli/tui.rs 単独)

---

## 7. 未確定 5 項目 (要 team-lead 裁定)

1. **3 PR 分割 vs 1 PR 一括**: 3 PR 推奨だが、release timing で 1 PR にまとめる選択肢も
2. **W7-D の `init_with_progress` シグネチャ**: `MultiProgress` を引数で渡すか、`Theme + bool` で内部生成かのどちらが clean か
3. **W7-E の writer/publish への `Option<&PipelineProgress>` 引数追加**: Phase 2 で deferred したが、Phase 3 で正式に投入してよいか (quality 領域への介入度)
4. **W7-E sub-bar の見た目**: 設計案 v3 で書いた `↳ <source> {spinner} {wide_msg}` で OK か、別形式か
5. **W7-F のキー**: `e` で error modal トグルだが、`?` (ヘルプ) と並列で modal 種別が増える。`Esc` で全 modal 閉じる挙動を統一するか

---

## 8. ロードマップ (採用前提)

```
Phase 3-D (~3h / v0.9.0-rc1): W7-D indicatif-log-bridge
   ├─ Cargo.toml: indicatif-log-bridge 追加 (lowlevel に依頼)
   ├─ logging.rs: init_with_progress 追加
   ├─ display.rs: IndicatifBackend::new で MultiProgress を logging に渡す
   ├─ cli/tui.rs: TuiTracingLayer + AppEvent::Log
   └─ smoke: fetch-trends で log と bar が干渉しないことを実機確認

Phase 3-F (~2h / v0.9.0-rc2): W7-F エラー詳細 modal
   ├─ cli/tui.rs: App.error_detail / `e` キー / render_error_overlay
   ├─ status bar に `e` ヒント追加
   └─ smoke: 故意に失敗させて modal 表示・スクロール確認

Phase 3-E (~6h / v0.9.0): W7-E sub-bar 進捗
   ├─ display.rs: SubBar trait + IndicatifSubBar + TuiSubBar
   ├─ writer/publish/trends: Option<&PipelineProgress> 引数 + article_bar wire
   ├─ cli/tui.rs: App.sub_bars + render 拡張 (Pipeline ペイン折り畳み)
   ├─ quality に事前通知 (writer/publish/trends シグネチャ拡張)
   └─ smoke: --top 3 で 3 article × 5 stage = 15 bar 同時表示確認

Phase 3 完了 (v0.9.0): 3 PR merged + smoke + tag
```

---

## 9. 次アクション

1. 本設計案を team-lead 上申
2. 採択判断 (3 機能全採用 / 部分採用 / 別案)
3. PR 順序確定 (W7-D → W7-F → W7-E 推奨)
4. quality に W7-E 連携の事前通知 (commander 経由)
5. 実装 GO 後、worktree `note-auto-tui` を再利用してブランチ `ui/v0.9.0-tui-d` から開始

---

> 実装はそれまで保留。アイドル待機継続。
