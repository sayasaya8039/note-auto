# B1: 学習スキル rules 昇格判断レポート

> **担当**: commander
> **日付**: 2026-05-06
> **対象**: utility_score 0.90+ の 4 件
> **目的**: `~/.claude/skills/learned/` から `~/.claude/rules/` への昇格判断

---

## 結論

**4 件すべて昇格推奨**。すべて他プロジェクトで即適用可能、Learned #001-#013 と同等の汎用性を持つ。

| # | スキル名 | utility | 昇格推奨 | rules 化形式案 |
|---|---------|---------|---------|---------------|
| 1 | untracked-files-hidden-build-success | 0.92 | ★★★ 強推奨 | `Learned #014` |
| 2 | stacked-pr-delete-branch-trap | 0.90 | ★★★ 強推奨 | `Learned #015` |
| 3 | agent-worktree-isolation-strategy | 0.90 | ★★★ 強推奨 | `Learned #016` |
| 4 | independent-convergent-reports | 0.90 | ★★ 推奨 | `Learned #017` |

---

## 評価軸

各スキルを以下の軸で 5 点満点評価:

- **再利用性**: 他プロジェクトで適用可能か (1=note-auto 専用 / 5=普遍)
- **頻度**: 該当状況の発生頻度 (1=稀 / 5=日常)
- **被害規模**: 適用しなかった時の損害 (1=小 / 5=大)
- **検出容易性**: 一次情報で検出可能か (1=困難 / 5=明確)
- **対応コスト**: 適用時の作業負担 (1=重 / 5=軽)

---

## 評価詳細

### 1. untracked-files-hidden-build-success (0.92)

| 軸 | 評価 |
|----|------|
| 再利用性 | 5/5 - Rust/Go/TS/任意のモジュールベース言語で適用可能 |
| 頻度 | 4/5 - 開発フェーズ初期 + WIP 退避時に頻発 |
| 被害規模 | 5/5 - main ビルド破壊で全チーム停止リスク |
| 検出容易性 | 4/5 - WIP 退避後の build 検証ステップで即検出可能 |
| 対応コスト | 4/5 - hot-fix で復旧、wip ブランチから cherry-pick |

**総合**: 22/25 → 強推奨

**rules #014 形式案**:
```markdown
# 未追跡ファイル前提のビルド成功は隠れた破綻（Learned #014 — S昇格）

> tracked code が untracked file を参照していると、working tree でビルド成功するが stash/clean/checkout で破綻する。

## ルール
WIP 退避前に必ず:
1. 退避後ブランチで `cargo build` 再実行
2. エラーが出たら退避を中断、untracked と tracked の依存解消
3. 特に `pub mod xxx;` 宣言と実装ファイルの分離を警戒

## 適用条件
- Rust/Go/TS 等のモジュールベース言語
- WIP / 未コミット変更を別ブランチに退避する作業
- マルチエージェント開発で working tree 共有

## 実績
- note-auto v0.7.6: PR-A マージ後 main ビルド不能、wip/preexisting から 3 ファイル復元で復旧（PR-C #4）
- 2026-05-05 incident、10 分診断で復旧
```

---

### 2. stacked-pr-delete-branch-trap (0.90)

| 軸 | 評価 |
|----|------|
| 再利用性 | 5/5 - GitHub PR を使う全プロジェクト |
| 頻度 | 3/5 - stacked PR 運用時に発生 |
| 被害規模 | 3/5 - PR 番号消費 + 履歴喪失 |
| 検出容易性 | 5/5 - `gh pr view --json state` で即判明 |
| 対応コスト | 4/5 - 新規 PR 作成 + body 復元 5 分 |

**総合**: 20/25 → 強推奨

**rules #015 形式案**:
```markdown
# stacked PR + --delete-branch の自動クローズ罠（Learned #015 — S昇格）

> 親 PR を `--delete-branch` でマージすると依存する子 PR が auto-close、reopen 不可。

## ルール
stacked PR を運用する場合、親マージ前に:
1. 子 PR の base を新しいターゲット (例: main) に切り替える
2. または親マージで `--delete-branch` を使わず、後で手動削除
3. もしくは再作成覚悟で進める（gh pr view --jq '.body' で本文復元）

## 適用条件
- GitHub PR の stacked workflow
- ブランチ改名 (master→main) を伴う作業

## 実績
- note-auto v0.7.6: PR #1 master 削除で auto-close → PR #2 で再作成
- note-auto v0.7.6: PR #3 perf/v0.7.6 削除で auto-close → PR #5 で再作成
- 2 件発生、計 ~10 分の手戻り
```

---

### 3. agent-worktree-isolation-strategy (0.90)

| 軸 | 評価 |
|----|------|
| 再利用性 | 5/5 - マルチエージェント協働の全プロジェクト |
| 頻度 | 5/5 - 並列タスクが日常 |
| 被害規模 | 4/5 - WIP 衝突 / dirty 誤検出 / build 不整合の予防 |
| 検出容易性 | 5/5 - `git worktree list` で明示 |
| 対応コスト | 5/5 - 1 回 setup で永続効果 |

**総合**: 24/25 → 強推奨

**rules #016 形式案**:
```markdown
# Agent Worktree Isolation 戦略（Learned #016 — S昇格）

> マルチエージェント並列開発で `git worktree add` で物理隔離、WIP 衝突・dirty 誤検出・build 不整合を構造的に防ぐ。

## ルール
エージェント数 ≥ 2 で並列タスクある場合:
1. 各エージェント専用の worktree を作成
2. base 共通、head 独立
3. commander は main worktree 維持、各エージェントが独立 build / dirty check
4. rebase は各 worktree で独立実施

## 構造
```
/path/to/repo                ← commander の main
/path/to/repo-agent-A        ← agent A 専用
/path/to/repo-agent-B        ← agent B 専用
```

## 適用条件
- マルチエージェント / マルチユーザー協働
- 並列度 2 以上、各タスクが独立ブランチ可能

## 実績
- note-auto v0.7.7 PR-A/B/C 並列実装で衝突ゼロ
- note-auto v0.8.0 PR-K-A/B/C 連続 PR で worktree 戦略実証
- ui-macos が `note-auto-tui` worktree で物理隔離 → quality silent direct commit の影響を 100% 回避
```

---

### 4. independent-convergent-reports (0.90)

| 軸 | 評価 |
|----|------|
| 再利用性 | 4/5 - 複数エージェント / 複数レビュアー体制で適用 |
| 頻度 | 3/5 - 大型機能調査時に発生 |
| 被害規模 | 3/5 - 信頼性判断ミスで Phase 1 やり直しリスク |
| 検出容易性 | 4/5 - reports 比較 / commit message diff で検出 |
| 対応コスト | 5/5 - 認識するだけで意思決定加速 |

**総合**: 19/25 → 推奨

**rules #017 形式案**:
```markdown
# Independent Convergent Reports — 重複は信頼性 S 級（Learned #017 — S昇格）

> 複数エージェントが独立して同じ問題を特定したら、それは確実に存在する重大問題。冗長ではなく確実性の保証。

## ルール
独立収束を発見したら:
1. その問題は「議論の余地なし」として最優先実装
2. 単独報告との重み付けを変える（独立 2 件 = S 級）
3. team-lead エスカレで「commander 推奨採用」と添える

## 検出
- 異なるエージェントの調査レポートで同ファイル + 同機能 + 同リスク報告
- `diff reports/<a>-phase*.md reports/<b>-phase*.md` で照合

## 適用条件
- マルチエージェント並列調査体制
- 各エージェントが独立した視点・領域を持つ

## 実績
- note-auto v0.8.0 Phase 2: lowlevel L9 (writer 147 並列) ↔ quality H2 (writer 並列化リスク) 独立収束
- note-auto v0.8.1 Phase 2.5: lowlevel M2 (5 client retry) ↔ quality openai 削除 (Warning 3/4) 収束的補完
- 2 件で実証、信頼性 S 級判定で意思決定速度向上
```

---

## 昇格手順案

### Step 1: rules ディレクトリへ複製
```bash
cp ~/.claude/skills/learned/<skill>.md ~/.claude/rules/<skill>.md
# または rules 形式で書き換えて新規作成
```

### Step 2: 番号付与
既存 Learned #001-#013 の続きで:
- #014: untracked-files-hidden-build-success
- #015: stacked-pr-delete-branch-trap
- #016: agent-worktree-isolation-strategy
- #017: independent-convergent-reports

### Step 3: フォーマット統一
既存 rules (`~/.claude/rules/`) のスタイルに合わせ:
- 短い rule 説明（1-3 行）
- ルール本体（do/don't）
- 適用条件
- 実績

→ 詳細は learned/ に残し、rules/ では essence のみ。

### Step 4: 学習スキルからのリンク
昇格後、learned/ には:
```yaml
promoted_to: rules/<skill>.md
status: archived
```
を追記、参照リンクで結びつける。

---

## team-lead への判断要請

1. **4 件昇格 GO** で良いか?
2. 昇格番号 (#014-#017) は仮、既存 #013 までの実情に合わせて番号調整必要なら指示ください
3. 昇格作業の実行者: commander 自身で OK?

承認後 commander が `~/.claude/rules/` への昇格作業を実施します（30 分以内）。

---

## 残存スキル (utility 0.80-0.85)

昇格しないが価値ある 3 件:
- `dirty-tree-attribution-rule` (0.80) - 1 回の note-auto 適用のみ、追加実証で 0.90+ なら昇格検討
- `gradual-branch-protection-rollout` (0.85) - note-auto 1 サイクルで実証、別プロジェクトでの再適用で 0.90+ 期待
- `accidental-main-direct-commit-recovery` (0.80) - lowlevel の 2 度目で実証中、再発防止策が機能すれば utility 上昇

これらは v0.9.0 / Phase 4 で再評価予定。
