## 🔧 Bug fix / Reliability

- **AI クライアント retry/backoff 統一実装** (M2, 4ba4ddb)
  - `src/util.rs::send_with_retry` を新設、指数バックオフ + jitter
  - 全 6 clients (anthropic / gemini / nvidia / openai / pollo / xai) に適用
  - HTTP 429 / 502 / 503 で記事即 fail を防止
- **daemon History single-load** (M1, eba2567)
  - `execute_cycle` 内で `History::load()` を 1 度に集約
  - dedup と append が同一インスタンスで一貫
- **Grok citations fallback** (M3, cad92ad)
  - `xai::research` で citations 空時に `trend.item.url` を補完
  - 記事末尾「参考リンク」がゼロになる問題を回避

## 🧹 Code quality

- **Warning 1**: `publish/mod.rs` の slack import 整理 (eba2567)
- **Warning 3**: `ai/openai.rs::generate` dead code 削除 (eba2567)
- **Warning 4**: `ai/openai.rs::build_prompt` dead code 削除 (eba2567)
- **warnings**: **4 (v0.8.0) → 1 (v0.8.1)**、-3（75% 削減）

## 🎨 UI / UX — TUI refinement (P4)

- **`?` ヘルプオーバーレイ** (c0688f4)
  - 中央 56×16 cols モーダル、`BorderType::Rounded` + Aqua アクセント
  - キーバインド一覧を視覚的に整理
- **`Ctrl-L` Logs クリア**
  - bash `clear` / less `K` と同セマンティクス
  - `(Logs cleared)` マーカー残置
- **各 stage の elapsed 表示** (L12 連動の価値強化)
  - `format_elapsed()` ns/µs/ms/s 適応切替
  - 例: `"23 件取得 (482ms)"`、Phase 3 ボトルネック特定の足がかり
- **Status bar 拡張**: `?Help` / `Ctrl-L ClearLog` ヒント追加

## 🏗️ Infrastructure (P1 + 学習)

- **main public 化** + **branch protection 有効化**
  - https://github.com/sayasaya8039/note-auto OSS 公開
  - main 直 push 物理ブロック、enforce_admins=false で admin 緊急 hot-fix 維持
- **pre-push hook 推奨**: `reports/git-hooks-suggestion.md`
  - 個人ローカルで main/master 直 push を物理ブロック
  - tag push は通過（hook 改良で stdin の remote_ref を検査）
- **運用ルール体系化**:
  - v3: 通信 (受領応答 / 完了報告)
  - v4: PR 経由必須 + admin override 記録
  - v4.1: branch 確認 (`git status -sb` + `git branch --show-current`)
- **`reports/admin-override-log.md`** で admin 直 push を全記録、月次レビュー

## ⚠ Compat

- 既存 `note-auto run/once/daemon/fetch-trends/write/publish/tui` は無変更
- API シグネチャ全 unchanged（retry は内部追加）
- bat shim 互換維持
- `--features tui` 無効時はバイナリ +0KB

## 📊 Stats

- warnings: **4 → 1**
- AI 全 6 クライアントに retry/backoff
- TUI 拡張 3 機能 (`?` / `Ctrl-L` / elapsed)
- public 化 + branch protection で governance 一段階成熟

## 🔭 Next: Phase 3 (v0.9.0 候補)

- **P2**: simd-json A/B 計測 (anthropic/xai レスポンス、reports/ 蓄積予定)
- **P3**: jemalloc ベンチ計測 (Windows 実測、reports/ 蓄積予定)
- **P5**: OpenTelemetry エクスポート プロトタイプ (低優先)
- **TUI 拡張**: `c` リロード / `s` statistics / indicatif_log_bridge (P4 残候補)
- **enforce_admins=true 切替判断** (hot-fix 需要を実測してから)

## 🏆 Phase 2.5 Learning

本フェーズで **2 件**の学習スキル追加 + **1 件**の utility bump:

- **NEW**: `accidental-main-direct-commit-recovery.md` (utility 0.80, v1.0)
- `independent-convergent-reports.md` 0.88 → **0.90** (v1.2、lowlevel M2 ↔ quality openai 削除の収束で実証)

→ Phase 1〜2.5 累計 **7 件**、4 件が utility 0.90+ で rules 昇格候補レベル。

## 📝 Admin override log

v0.8.1 中に発生した admin override 経路 commits:
- `4ba4ddb feat(ai): unified retry/backoff (M2)` — lowlevel accidental main commit、追認
- `eba2567 refactor(quality): M1 + Warning 1/3/4` — quality silent worker
- `0a36b9f fix(ai): apply M2 to openai.rs (PR-M scope-up)` — lowlevel ad-hoc admin override

全件 `reports/admin-override-log.md` に記録、team-lead 追認済。
