# v0.9.0 — AI client 統一 + production silent loss 4 件救済 + SSRF 第一防衛線

**Release Date**: 2026-05-06
**Tag**: `v0.9.0`
**Previous**: v0.8.1

## ハイライト

v0.9.0 は **AI infrastructure の統一**、**production silent loss 4 件の根絶**、**SSRF 第一防衛線**、**CI/CD 整備**を一気に完成させた品質保証リリース。

- **AI client 全 retry/backoff coverage**: H1 (Pollo polling) / M1 (X API) / M2 (image DL) で AI 呼び出しの全 path に retry が貫通
- **AiClient trait 統一**: 6 client (anthropic / openai / xai / gemini / nvidia / pollo) を `impl AiClient` で統一、新規 client 追加コストが大幅削減
- **SSRF guard**: `is_safe_image_url` で HTTPS only + IPv4 private/loopback/link-local block、169.254.169.254 cloud metadata 攻撃を遮断
- **CI/CD 整備**: GitHub Actions ci.yml (lint+build) + release.yml (tag → binary artifact) + bench.yml (workflow_dispatch)
- **Criterion bench harness**: `title_similarity_short = 4.99µs` 等の baseline 数値が記録、将来 perf 改善判断の基盤
- **UI/UX 改善**: 自前 MakeWriter による tracing × indicatif 競合解消、エラー詳細 modal (`e` トグル / `Esc` 統一)
- **warnings 0 達成**: v0.7.7 baseline 4 件 → v0.8.1 1 件 → **v0.9.0 0 件**

## 詳細変更

### CI/CD (3 PR)
- **CI1** (#19): `.github/workflows/ci.yml` 追加 — lint (clippy `-D warnings`) + build (linux + windows)
- **CI2** (#20): `.github/workflows/release.yml` 追加 — tag push 時に linux/windows binary を artifact upload
- **AF3 副産物** (#30): `.github/workflows/bench.yml` 追加 — `workflow_dispatch` only、CI コスト最小化

### AI Infrastructure (3 PR)
- **AF1** (#21): `src/ai/client.rs` — `AiClient` trait + `send_json` / `send_text` helper、async_trait + retry/backoff 統合
- **AF2** (#23): 6 client (anthropic / openai / xai / gemini / nvidia / pollo) を `impl AiClient` に移行、error format / retry 一元化
- **AF3** (#30): criterion bench harness、`benches/scoring.rs` (3 fn) + `benches/json_parse.rs` (3 fn)、実機計測済

### Production Silent Loss 救済 (3 PR)
- **H1** (#24): Pollo `poll_until_done` 5 分 polling 中の単一ネットワークエラーで全水泡 → `match+continue` で resilience、画像 0 枚 silent loss + Pollo 課金損失防止
- **M1** (#25): X API announce + post_text の bare `.send()?` を `send_with_retry(max_retries=3)` で wrap、Slack final report の x_status 改善、429/5xx で permanent fail 解消
- **M2** (#27): util.rs に `fetch_bytes_with_retry` helper 追加、pollo / openai / nvidia の secondary image download 3 箇所適用、retry coverage 完成

### Security (1 PR)
- **M3 Fix A 簡易** (#28): writer/mod.rs に `is_safe_image_url` ガード — HTTPS only + IPv4 private/loopback/link-local block。古典 SSRF (169.254.169.254 metadata, 192.168.x.x, 127.0.0.1) を遮断
  - **v0.9.1 で Fix B 熟成予定**: DNS resolve + retail CDN allowlist + TOCTOU 対策

### UI/UX (3 PR)
- **W7-D** (#22): 自前 `MultiProgressWriter` (`MakeWriter` 実装) で `multi.suspend()` 経由の log 出力、indicatif progress bar と tracing event の競合解消。indicatif-log-bridge 不採用判断 (log/tracing 非互換)
- **W7-F** (#26): `ErrorDetail` struct + error overlay (中央 70%×60% modal)、`e` トグル / `Esc` 統一、status bar に `e Error` ヒント
- **W7-D'** (#29): main.rs init refactor — `init_with` 削除 + PipelineProgress 経路は `init_with_progress` 経由、W7-D の wire を完全活性化

### Code Quality
- **silent quality Phase 3 report** (87e4cd7): `reports/quality-phase3.md` (+192 行)、H1 (HIGH) 発見が起点
- **warnings 0 達成**: CI1 で `clippy -D warnings` を強制
- **clippy allow 2 系統明示**: `if_same_then_else` (display.rs) / `type_complexity` (trends/mod.rs) は v0.9.1 で解消候補

### Governance
- **public 化** + branch protection + admin-override-log + 月次レビュー仕組み
- **pre-push hook** で main/master 物理 block、accidental commit 防止
- **通信ルール v4 + v4.1 + 直前確認** 全 13 PR で main 直接 commit ゼロ達成
- **3 重防壁** (pre-push hook + v4.1 + 直前確認) が完璧に機能

## 統計

- **13 PR + 1 silent commit + 1 tag/release** で v0.9.0 完成
- Total: 1 day で集中達成
- lowlevel: 9 PR (CI1/CI2/AF1/AF2/AF3/H1/M1/M2/M3) + chore bump
- ui-macos: 3 PR (W7-D/W7-F/W7-D')
- quality silent: 1 commit (Phase 3 report)
- commander: 1 chore PR (version bump) + 全 PR self-merge

## 互換性

- **配置環境**: developer / cloud VM 両対応 (M3 SSRF guard で cloud metadata 防御)
- **API key**: 既存設定そのまま、変更なし
- **Cargo.toml**: criterion = "0.5" を `[dev-dependencies]` に追加 (bench 用、production binary 影響なし)

## v0.9.1 (Phase 3.5) 予定スコープ

- **W7-E**: writer/publish/trends に `Option<&PipelineProgress>` 引数追加 (~6h)、pipeline progress threading + UI deeper integration
- **M3 Fix B**: DNS resolve + retail CDN allowlist + TOCTOU 対策 (要熟成)
- **quality 復帰時の Q?**: 任意取り込み
- **clippy allow 2 系統解消**: display.rs / trends/mod.rs の整理

## 学習スキル昇格

本リリース中に発見・蓄積した運用パターン:

- `independent-convergent-reports.md` (#019, S 級): 独立収束 = 信頼性 S 級シグナル
- `agent-worktree-isolation-strategy.md` (#018, S 級): worktree 物理隔離戦略
- `untracked-files-hidden-build-success.md` (#016, S 級): tracked code が untracked file 参照する罠
- `stacked-pr-delete-branch-trap.md` (#017, S 級): `--delete-branch` で子 PR auto-close 罠
- `silent-worker-as-quality-reviewer.md` (utility 0.85 v1.0): silent worker は reports/ 経路で重大バグ発見可能

## クレジット

- **lowlevel**: AI infrastructure / CI/CD / production 救済 / security の主担当
- **ui-macos**: UI/UX 主担当、indicatif × tracing 統合 + error modal
- **quality (silent)**: Phase 3 調査レポート、H1 発見の起点
- **team-lead**: governance + Plan A/Y 採用判断 + M3 Fix A 簡易採用 (commander v0.9.1 推奨を覆し)
- **commander**: orchestration + 13 PR 自走マージ + リリース工程

---

🤖 Generated with [Claude Code](https://claude.com/claude-code)
