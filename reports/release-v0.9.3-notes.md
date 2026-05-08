# v0.9.3 — Phase 3.7: polish + supply chain + test coverage 底上げ

**Release Date**: 2026-05-06
**Tag**: `v0.9.3`
**Previous**: v0.9.2

## ハイライト

v0.9.3 は v0.9.2 リリース直後の **Phase 3.7 短期集中** + 発見ベース調査により、polish + supply chain 監視 + test coverage 底上げ + dead_code 整理を達成。**Phase 3 完全完遂の最終リリース**。

- **dead_code 整理**: DCX-2 で SubTick の `#[allow(dead_code)]` 自然解消、3 段階透明 deferred パターン完結
- **CLI/UX polish**: CLI-3 (--help heading 分離) + CLI-5 (banner terminal 幅追従)
- **TUI ヘビー user 価値**: W7-H-1 vim nav (h/l) + W7-H-5 Logs PgUp/PgDn scroll
- **依存整理 + perf**: DEP-1 (urlencoding 撤去) + PERF-4 (regex LazyLock 化)
- **test coverage 底上げ**: TEST-1 (scoring 5 シナリオ) + TEST-2 (title_similarity 6 境界)、test 16 → 27 (+11、+69%)
- **supply chain 監視**: CI3 で cargo audit ジョブ追加 (warning-only、Phase 3.7 段階)

## 詳細変更

### lowlevel batch (2 PR)

#### PR #40 — DEP-1 + PERF-4 (chore + perf)
- **DEP-1**: `urlencoding` crate を `Cargo.toml` から削除、gnews_rss.rs:24-28 の `urlencoding::encode` を `url::form_urlencoded::byte_serialize` で代用
- **PERF-4**: `gnews_rss::extract_og_image` の `Regex::new` を `static OG_IMAGE_RE: LazyLock<Regex>` 化、OG enrich N 件で µs オーダー削減
- 3 files +17/-19、deps -1 件、バイナリ -数 KB

#### PR #42 — TEST-1 + TEST-2 + CI3
- **TEST-1**: `scoring::select_top` の 5 シナリオ unit test (empty / truncation / dedup / round-robin / fallback)
- **TEST-2**: `util::title_similarity` の 6 境界 test (identical / different / empty / partial / unicode / case-insensitive)
- **CI3**: `.github/workflows/ci.yml` に `cargo audit` ジョブ追加 (continue-on-error で warning-only)
- 3 files +187/-?、test pass: 16 → 27 (+11、+69%)
- TEST-1 で初回 failure → "Story A"/"Story B" 類似度 ~0.71 (dedup_threshold 0.65 超過) を発見、scoring の dedup 動作が意図通り強力であることを実証

### ui-macos batch (2 PR)

#### PR #41 — DCX-2 + CLI-3 + CLI-5
- **DCX-2**: `PipelineUpdate::SubTick` の `#[allow(dead_code)]` 削除、WPW1 で `bar.tick()` 経由 (3 段階透明 deferred 完結)
- **CLI-3**: clap `help_heading` で Global Options / Display Options 分離
- **CLI-5**: banner cosmetic を `console::Term::size_checked()` で terminal 幅追従、上限 120 / 下限 50 / fallback 60、stages 行末尾 padding で右枠揃え
- 2 files +32/-10

#### PR #43 — W7-H-1 + W7-H-5
- **W7-H-1**: vim nav `h/←` (Sidebar 直行) + `l/→` (cycle_focus)、既存 Tab/j/k 互換
- **W7-H-5**: `App.logs_scroll` 追加、`PgUp/PgDn` (5 行/page) + `Home/End` (最古/最新)、Logs focus 時のみ反応、`push_log` 内で scroll=0 リセット (tail 動作維持)、title バーに `(N | M↑)` バッジ
- help overlay 拡張で新キー 5 件を一覧化 (modal 16→22 行)
- 1 file +89/-6

### Chore (1 PR)
- **chore: bump 0.9.2→0.9.3** (#44): Cargo.toml / Cargo.lock version sync

### 担当外観察 (相互レポート)
ui-macos 調査が以下を観察 (Phase 3.7 不採用、v1.0.0 候補):
- **lowlevel**: `--features tui` 時の binary +3-5MB、`cargo bloat` 精査余地
- **quality**: `tracing::info_span!` field の冗長性、tag/category 追加余地

## 統計

- **5 PR + 1 tag/release** で v0.9.3 完成
- Total: ~1.5h で集中達成 (lowlevel 1h + ui-macos 1.5h + commander chore + tag)
- lowlevel: 2 PR (DEP+PERF + TEST+CI3)
- ui-macos: 2 PR (DCX+CLI + W7-H-1/5)
- commander: 1 chore PR (version bump)
- 並行戦略: lowlevel/ui-macos worktree 完全独立、衝突ゼロ

## 品質メトリクス推移

| 指標 | v0.9.0 | v0.9.1 | v0.9.2 | **v0.9.3 (現在)** |
|------|--------|--------|--------|-------------------|
| warnings | 0 | 0 | 0 | **0** |
| clippy allow | 2 系統 | 0 | 0 | **0** |
| AI retry coverage | 6/6 | 6/6 | 6/6 | **6/6** |
| SSRF defense layer | L1 | L2 | L5 (5 層) | **L5 (5 層)** |
| TUI 機能 | wire only | sub-bar event | 階層表示 + phase wire | **+ vim nav + scroll** |
| **test pass** | **16** | **16** | **16** | **27 (+11、+69%)** |
| **CI/CD jobs** | **3 (lint/build×2)** | **3** | **3** | **4 (+ cargo audit)** |
| **依存数** | **31** | **31** | **31** | **30 (-urlencoding)** |
| dead_code allow | 18 | 18 | 17 (SubTick 解消経路) | **17 (SubTick 完結)** |

## CL2 — v1.0.0 deferred

clippy strict (`-W clippy::pedantic -W clippy::nursery`) の safety 系部分採用は **v1.0.0 cooling-off 期間で熟成**:
- rtk wrapper 経由出力が空形式で個別 lint 特定困難
- raw cargo + log file output で個別精査体制を整える必要
- ROI vs 判別コストで commander 判断、v1.0.0 大型 phase の一部として採用予定

## 互換性

- **依存**: `urlencoding` crate 削除、`url::form_urlencoded` で代用、public API 影響なし
- **CLI**: --help 出力構造変更 (heading 追加)、内容は同等
- **TUI**: 新規キーバインド (h/l/PgUp/PgDn/Home/End)、既存キー互換
- **API key / 設定ファイル**: 既存設定そのまま、変更なし

## v1.0.0 (cooling-off 後) 予定スコープ

cooling-off 期間 (1〜2 週間)、commander が以下を実施:
- 月次レビュー継続
- 実運用反応蓄積 (`reports/v0.9.x-feedback.md`)
- silent quality 復帰機会の観察

cooling-off 後、user 判断で v1.0.0 着手:
- ERR-2 thiserror 導入 (4-6h)
- TEST-4/5/7 mock test 群 (6.5h)
- PERF-2 bigram index pair refactor (4h)
- DLG-1 dialoguer interactive 検討
- W7-H-3 theme 拡張 + macOS-style 仕上げパッケージ
- CL2 部分採用 (raw cargo + log file output 経由)
- cargo bloat 精査 + tracing 構造化強化 (担当外観察項目)

## クレジット

- **lowlevel**: DEP-1 + PERF-4 + TEST-1 + TEST-2 + CI3 主担当、TEST-1 で dedup 動作の堅牢性を実証
- **ui-macos**: DCX-2 + CLI-3 + CLI-5 + W7-H-1 + W7-H-5 主担当、内化したパターン 5 件 (3 段階 deferred / vim nav 統合 / scroll-on-push reset / status bar 同期 / modal 動的拡張) を v1.0.0 で再活用予定
- **quality (silent)**: 自由スコープ継続中 (招待応答待ち)
- **team-lead**: Plan A 採用判断 + CL2 部分採用基準明示 + v1.0.0 cooling-off 設計
- **commander**: orchestration + 5 PR 自走マージ + リリース工程 + CL2 deferred 一次判断

## v0.9.0+v0.9.1+v0.9.2+v0.9.3 累計 (Phase 3 全期間総括)

| カテゴリ | v0.9.0 | v0.9.1 | v0.9.2 | v0.9.3 | 累計 |
|---------|--------|--------|--------|--------|------|
| CI/CD | 3 | - | - | 1 (CI3) | 4 |
| AI infrastructure | 3 | - | - | - | 3 |
| production 救済 | 3 | - | - | - | 3 |
| security | 1 (L1) | 1 (L2) | 1 (L3+L4+L5) | - | **3 PR、5 層完成** |
| UI/UX | 3 | 1 | 2 | 2 | 8 |
| code quality | - | 1 | - | 1 (DCX-2) | 2 |
| dep/perf | - | - | - | 1 (DEP+PERF) | 1 |
| test | - | - | - | 1 (TEST-1+2) | 1 |
| chore | 1 | 1 | 1 | 1 | 4 |
| silent commit | 1 | - | - | - | 1 |
| **PR 総数** | **14** | **4** | **4** | **5** | **27** |

### 1.5 日で 27 PR 達成 (累計、新記録)
- 通信ルール v4 / v4.1 / v4.2 / pre-push hook / STATE プレフィックス全 PR 厳守
- main 直接 commit ゼロ達成、accidental commit ゼロ
- **4 重防壁完璧機能**
- 5 回の時系列交差を全て 1 ターン合流で解決
- ui-macos 効率: 見込み 2.5h → 実績 ~1h (新たな 50%+ 短縮)

### 学習スキル累計 8 件 (Phase 3 で 5 件追加)
- rules 昇格 4 件 (#016-019)
- skills/learned 4 件 + 1 新規 (silent-worker-as-quality-reviewer 0.85)

### governance 進行
- Phase 3 完了: public + branch protection + admin-override-log + 月次レビュー仕組み
- 月次レビュー 2026-05 完成
- pre-push hook 導入後 accidental 0% 維持
- v1.0.0 cooling-off 期間で governance Phase 4/5 検討

---

🤖 Generated with [Claude Code](https://claude.com/claude-code)
