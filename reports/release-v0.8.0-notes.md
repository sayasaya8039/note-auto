## 🔧 Bug fix / Reliability
- **writer::run の 147 並列問題解決** (L9) — `buffer_unordered(N=cfg.writer.concurrency, default 3)` で Anthropic/Pollo rate-limit 直撃を回避（21 記事 × 7 API → 同時 3 記事）
- **ai/mod.rs::http_client expect → fallback** (H1) — Q2 漏れの panic=abort 不安全を `unwrap_or_else + tracing::warn` で解消

## 🚀 Performance / Observability
- **tracing::info_span! 4 stage** (L10) — `fetch` / `score` / `write` / `publish` のレイテンシが log で見えるように
- **logging.rs FmtSpan::CLOSE** (L12) — span CLOSE 時の elapsed 自動出力（実機: `INFO fetch: close time.busy=12.3s` 等）
- **gnews_rss og_images take→buffer_unordered 正規化** (L11) — 並列度 N の意味を仕様通りに

## 🎨 UI / UX — macOS-style TUI 全実装
- **`note-auto tui` 新サブコマンド** (W7) — `--features tui` opt-in で有効化
  - 3 ペインレイアウト: 左 Sidebar (7 カテゴリ Finder 風) / 右上 5 Stage 進捗 / 右下 Logs
  - macOS Big Sur dark パレット: accent `#007AFF` / success `#30D158` / error `#FF453A`
  - キーバインド: `j/k` ↑↓ / `Enter` 実行 / `Tab` ペイン切替 / `q` 終了
  - panic-safe な raw_mode + AlternateScreen 復元
- **TuiBackend + IndicatifBackend (PipelineBackend trait)** (W7-A) — CLI/TUI 切替、API 互換 100% 維持
- **`note-auto tui --attach-daemon`** (W7-C) — `.note-auto.lock` で daemon 排他制御、cron 進捗監視
- **`--ascii` フラグ TUI 反映** (W7-C) — TUI 内でも ASCII fallback 動作
- **History 表示** (W7-C) — Logs ペインに最近 5 件
- **`--features tui` opt-in** — TUI 不要環境（CI/server）はビルドサイズ最小化、ratatui 追加 +2〜3MB

## 📊 Stats
- v0.7.7 → v0.8.0: writer 並列度 **147 → 3**（rate-limit 直撃回避）
- warnings: **4 → 4** (baseline 維持、新規追加なし)
- 5 機能 PR merged (PR-I / PR-K-A / PR-I' / PR-K-B / PR-K-C)
- 1 silent quality 直接 commit (Phase 2 調査レポート)
- 1 release commit (version bump)

## ⚠ Compat
- 既存 `note-auto run/once/daemon/fetch-trends/write/publish` は無変更
- bat shim 互換維持
- API シグネチャ `print_*` / `Stage` enum / `PipelineProgress` ファクトリ全 unchanged
- `--features tui` 無効時はバイナリサイズ +0KB、ratatui 不要

## 🔭 Next: v0.8.1 (Phase 2.5)
- **Q1**: quality silent worker の Phase 1 残存 CRITICAL バグ
- **M1**: daemon::execute_cycle の History::load() 2 回呼び出し整理
- **M2**: AI クライアント retry/backoff 統一実装（指数バックオフ + jitter）
- **M3**: xai.rs Grok citations 空問題の本格対応
- main public 化 + branch protection 検討（コミット履歴監査後）

## 🏆 Phase 2 Learning
本フェーズで `~/.claude/skills/learned/` 1 件追加 + 3 件 utility_score bump:
- **`independent-convergent-reports.md`** (utility 0.85, v1.0) ← NEW
- `untracked-files-hidden-build-success` 0.90 → **0.92** (v1.2)
- `stacked-pr-delete-branch-trap` 0.85 → **0.90** (v1.1)
- `agent-worktree-isolation-strategy` 0.85 → **0.90** (v1.1)

→ Phase 1〜2 で計 **5 件**の学習スキル蓄積、3 件が rules 昇格候補レベル。
