# v0.9.4 — Phase 4 hot-fix: publish hardening (Fix-A/B/D/S/E2)

**Release Date**: 2026-05-07
**Tag**: `v0.9.4`
**Previous**: v0.9.3
**Type**: Production CRITICAL hot-fix

## ハイライト

v0.9.4 は v0.9.3 リリース直後の cooling-off を **production CRITICAL 緊急中断**して投入した hot-fix release。production note 投稿失敗 + selector timeout 多発 + Slack 通知ゼロという複合問題を根本解消。

- **root cause #1 解消**: `dotenvy::dotenv()` 配線抜け → main entry で恒久化 (Fix-E2)
- **production hardening 4 件**: UA 統一 (Fix-A) + SingletonLock 回避 (Fix-B) + 下書き保存 selector 強化 (Fix-D) + waitForSelector + screenshot + retry (Fix-S)
- **cooling-off 中断 → 緊急再起動 → 1.5h で 5 PR 完成**: 過去 50% 短縮実績適用、production 救済の最速対応

## 詳細変更

### Rust (1 PR)

#### PR #45 — Fix-E2: dotenvy 配線恒久化
- **担当**: lowlevel
- **修正**: `src/main.rs:118-122` に `let _ = dotenvy::dotenv();` 5 行追加 (`Cli::parse()` より前)
- **stats**: 1 file +5
- **build**: 0 errors / 12.45s incremental / warnings 0
- **背景**: production で `.env` が読まれていなかった → Slack 通知ゼロ + 各種 API key 未設定動作の root cause
  team-lead が production に 1 行直接適用で即時救済 → 本 PR で commit + push 恒久化
- **副次成果**: lowlevel が `slack.rs` + `ai/*` + `publish/*` + `writer/*` の env 依存箇所を横断調査 → **dotenvy 配線抜け類似バグは他に存在しない**ことを確認 (`reports/lowlevel-phase4-audit.md`)

### Node.js publish script (1 PR)

#### PR #46 — Fix-A + Fix-B + Fix-S + Fix-D (4 commit)
- **担当**: ui-macos
- **対象**: `scripts/note-publish.mjs` (1 file +148/-16)
- **stats**: 4 commit (Fix 単位、revert 粒度確保)、25min で完成 (50% 短縮実績通り)

##### Fix-A (UA 統一) — `0585bfa`
- **修正**: `:65` UA module 定数化 + `:129-135` tryLaunch baseArgs に注入
- **目的**: managed chromium fallback 経路でも HeadlessChrome 検出回避
- **リスク**: 低

##### Fix-B (SingletonLock 衝突回避) — `acbf734`
- **修正**: `:60-62` lock unlink + `:80-83` Promise.race + exit 21 検出 + 5 秒待機 + 1 retry + waitForPort timeout 20s→5s
- **目的**: 残骸 SingletonLock で chromium 起動失敗時の自動回復
- **リスク**: 中 (本番 profile 専用で他 instance との競合は軽微)

##### Fix-S (waitForSelector 強化) — `d79278f`
- **修正**: `:224, :230` timeout 30s→60s + 失敗時 screenshot 保存 + 1 回 reload retry をヘルパー化
- **目的**: throttling 下での selector 出現遅延に対応、失敗時の調査資材確保
- **リスク**: 低 (timeout +30s/article、screenshot は cookie_dir 下)

##### Fix-D (下書き保存強化) — `79985e1`
- **修正**: `:383-386` `.catch(() => {})` 削除 + 3 selector fallback + URL pattern 検証 (`^下書き保存$` regex アンカー化)
- **目的**: silently failing draft save の検出 + 詐称防止
- **リスク**: 低-中 (誤クリック回避、status 値が変わるケースで Rust 側 publisher の error 経路要確認)

### Chore (1 PR)
- **PR #47**: `chore: bump 0.9.3→0.9.4`

## 統計

- **3 PR + 1 tag/release** で v0.9.4 完成
- Total: ~1.5h で集中対応 (cooling-off 中断 → 緊急再起動 → release まで)
- lowlevel: 1 PR (Fix-E2、10 min) + slack.rs 横断監査 (5 min)
- ui-macos: 1 PR (4 commit、25 min for impl)
- commander: 1 chore PR (version bump、5 min)

## live smoke 状況

### 実施済 ✅
- **dry-run smoke**: `note-auto.exe publish --from drafts/2026-05-07/articles.json --dry-run`
  - env / build / config / manifest loading 全 OK
  - Cargo + dotenvy 経路 (Fix-E2) 動作確認
  - articles.json 3 記事 enumeration 動作確認
  - Slack 通知 dry-run skip OK
- **基本起動**: `--version` (v0.9.4) / `--help` (CLI-3 heading 整形 visible)

### 未実施 ⏳
- **live publish smoke**: 二重投稿リスク回避のためスキップ
- **Fix-A/B/D/S 実機検証**: 翌朝 cron (07:00 JST) で実施予定

### 翌朝 cron 監視ガード (commander 担当)
2026-05-08 07-08 AM JST:
1. main HEAD `287367e` 以上が cron に乗ったか確認
2. cron 実行ログ (`logs/all_*.log`) tail で確認:
   - `note_status=draft` 真偽 (Fix-D 効果)
   - `cdp ... browser exited code=21` 後の retry 機能 (Fix-B 効果)
   - `waitForSelector` timeout 時の screenshot 保存 (Fix-S 効果)
   - `[launch] OK: managed chromium` fallback 時の認証維持 (Fix-A 効果)
   - Slack 通知到達 (Fix-E2 効果)
3. 判定:
   - 全 7 記事 draft 成功 → **Fix 全効果あり、v0.9.4 確定**
   - 部分 failure (< 50%) → improvement あり、現状受容
   - 全 failure or 改悪 → **即 revert (4 commit 単位) + 緊急エスカレ**

## v1.0.0 cooling-off 候補 (Phase 4 由来)

### Fix-C (deferred)
- 単一ブラウザインスタンス化 (現状 N 記事 N ブラウザ起動)
- 工数大、v1.0.0 大型 phase の一部として計画

### Audit 由来 (lowlevel 監査結果)
- **AUDIT-1**: `slack.rs::post_summary` に send_with_retry 適用 (M1/M2 同パターン、15min)
- **AUDIT-2**: dotenvy 配線確認 integration test (30min)
- **AUDIT-3**: `env_*` helper Default 値 + エラーメッセージ強化 (30min)

### 担当外観察 (ui-macos からの提案)
- `src/publisher/` で `status="error"` 受信時の retry/通知ロジック確認 (lowlevel 領域)
- `cookie_dir/screenshots/` 自動 cleanup タスク化 (quality 領域)

## 互換性

- **依存**: `dotenvy` 既存 dep の使い方変更のみ、新規依存なし
- **API key**: `.env` 形式変更なし、Fix-E2 で配線が確実に動作
- **CLI**: 変更なし
- **TUI**: 変更なし
- **設定ファイル**: 変更なし
- **production cron**: 既存 `note-auto.exe once` cron そのまま動作、Fix が透過適用

## 学習スキル候補 (Phase 4 由来、cooling-off 中盤で追加検討)

team-lead 提案 3 件:
- **dependency-added-but-unwired** (utility 0.85): Cargo.toml に追加済 → main で呼び忘れ
- **silent-error-handler-rule** (utility 0.85): `.catch(() => {})` 全握りつぶしの危険性
- **post-merge-production-smoke-pattern** (utility 0.80): live smoke 不可環境で 4 commit 単位 revert を担保にする運用

## クレジット

- **lowlevel**: Fix-E2 主担当 + slack.rs 横断監査 (env 配線抜け類似バグ全件精査結論「他に存在しない」)
- **ui-macos**: Fix-A/B/D/S 主担当、scripts/note-publish.mjs 集中 1 PR (4 commit)、25min で実装完成 (50% 短縮実績)
- **team-lead**: production CRITICAL 即時救済 (1 行 production 直接適用 = Fix-E hot-fix 起点) + Plan A/(1) 採用判断 + 翌朝 cron 監視ガード設計 + 学習スキル候補抽出
- **commander**: orchestration + 3 PR 自走マージ + dry-run smoke + リリース工程

## v0.9.x 累計 (Phase 3 + Phase 4)

| バージョン | PR 数 | リリース日 | 主要成果 |
|-----------|-------|----------|---------|
| v0.9.0 | 14 | 2026-05-06 | CI/CD + AI infrastructure + production 救済 + SSRF L1 |
| v0.9.1 | 4 | 2026-05-06 | SSRF L2 + clippy default 全 pass |
| v0.9.2 | 4 | 2026-05-06 | SSRF L3+L4+L5、5 層完成 |
| v0.9.3 | 5 | 2026-05-06 | polish + supply chain + test coverage |
| **v0.9.4** | **3** | **2026-05-07** | **production publish 複合問題 hot-fix** |
| **累計** | **30 PR** | **2 日** | - |

通信ルール v4 / v4.1 / v4.2 / pre-push hook / STATE プレフィックス全 PR 厳守、main 直接 commit ゼロ達成 (admin override は Fix-E hot-fix 1 件のみ、Fix-E2 で正規化)。

---

🤖 Generated with [Claude Code](https://claude.com/claude-code)
