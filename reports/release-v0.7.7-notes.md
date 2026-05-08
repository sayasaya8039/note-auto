## 🔧 Bug fix
- **`ScoringConfig::dedup_threshold` properly wired** (was hardcoded 0.65) — TOML 設定値が反映されるように
- **panic=abort safety**: `unwrap()` を `?-propagation` に変換 (writer/publish 4 sites)

## 🚀 Performance
- **scoring** fill loop O(N²) → O(N) via HashSet
- **history** dedup O(1) lookup + atomic temp+rename write (crash-tolerant)
- **publish_all** parallelized via `stream::iter().buffered(2)`

## 🎨 UI / UX — 5 deps wire-up 完遂
- **owo-colors**: `Palette` via `owo_colors::Rgb`（W3）
- **console**: `Theme::detect` TTY 判定を `console::Term` ベースへ（W4）
- **supports-color**: color level 検出を `supports_color::on()` で共通化（W5）
- **comfy-table**: scoring 結果サマリ + publish 結果テーブルを rounded ボーダー表示（W2）
- **indicatif**: writer/publish パイプラインを `MultiProgress` で 5-stage 進捗可視化（W1）
- **`#![allow(dead_code)]` 削除**、個別 annotation 化（W6）

## 📊 Stats
- warnings: **6 (v0.7.6) → 4 (v0.7.7)**
- 4 PRs merged: PR-D (#6 lowlevel) / PR-E (#7 ui-macos Phase A) / PR-F (#8 ui-macos Phase B) / PR-G (#9 ui-macos Phase C)
- 1 silent quality direct commit (Q2/Q3/Q4 = `aa4de61`)

## ⚠ Compat
- API 互換 100%（既存 `print_*` シグネチャ全維持）
- bat 引数仕様据え置き、外部スケジューラ影響なし
- 旧 cmd.exe (Raster Fonts) 互換は `--ascii` フラグで継続提供

## 🔭 Next: v0.8.0 (Phase 2)
- `buffer_unordered` で並列度制御（trends::fetch_all）
- `quick-xml` SAX モード移行
- `tracing` でレイテンシ計測
- `simd-json` 検討
- `mimalloc` → `jemalloc` 比較ベンチ
- main branch protection rule 設定
- ratatui TUI 検討（v0.7.6 で deferred）

## 🏆 Phase 1.5 Learning
本フェーズで `~/.claude/skills/learned/` に 4 件の学習スキル蓄積:
- `untracked-files-hidden-build-success` (utility 0.90)
- `stacked-pr-delete-branch-trap` (utility 0.85)
- `dirty-tree-attribution-rule` (utility 0.80)
- `agent-worktree-isolation-strategy` (utility 0.85)
