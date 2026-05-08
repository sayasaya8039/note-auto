# v0.9.3 (Phase 3.7) ui-macos 調査レポート

> **Status**: 調査確定待ち（commander 統合 → team-lead 上申）
> **Owner**: ui-macos teammate
> **Target version**: v0.9.3
> **Scope policy**: 発見ベース、自由スコープから優先度判定

---

## サマリ

- **改善候補**: 19 件 (4 軸合計)
- **フル実装時の総工数**: ~21h
- **ミニ release 推奨候補**: 5 件 (~3.5h、Phase 3.7 採用候補)
- **v1.0.0 deferred 推奨**: 6 件 (大型または ROI 待ち)
- **不採用**: 8 件 (現状で十分 / 優先度低)

**所感**: v0.7.6〜v0.9.2 で UI/CLI/TUI が成熟、**明確な必須課題は少ない**。v0.9.3 は **「ミニ release で polish 5 件のみ」** を推奨。dialoguer は v1.0.0 大型 phase 候補。

---

## 1. W7-H 候補 (TUI polish)

| ID | 観点 | 内容 | 優先度 | 工数 | ROI |
|----|------|------|--------|------|-----|
| **W7-H-1** | vim nav | `h/j/k/l` で全方向ペイン navigation (`Tab` の補助) | 中 | 30min | 中 |
| W7-H-2 | F1 help 拡張 | `?` overlay と F1 を共有、ビギナー向け hint | 低 | 30min | 低 |
| W7-H-3 | theme 拡張 | `--theme=solarized-dark / dracula` の代替パレット (Aqua 以外も) | 低 | 2h | 低 |
| W7-H-4 | spinner variation | 5 stage 別に異なる spinner pattern (Braille / dots / arrows) | 低 | 1h | 極小 |
| **W7-H-5** | scroll behavior | Logs ペインに `PgUp/PgDn` で履歴スクロール (現状 tail 固定) | 中 | 1h | 中 |
| W7-H-6 | color pulse | Active stage のアクセント色を 1Hz でパルス (subtle 強調) | 低 | 45min | 低 |
| W7-H-7 | ペイン分割比率調整 | `+/-` で sidebar 幅をリアルタイム調整 | 低 | 45min | 低 |
| W7-H-8 | 長文 wrap | sub-bar msg が長い場合の自動省略 + tooltip | 中 | 1h | 低 |

---

## 2. macOS-style 仕上げ

| ID | 観点 | 内容 | 優先度 | 工数 | ROI |
|----|------|------|--------|------|-----|
| MAC-1 | SF Pro 代替検証 | Linux/Windows で Cascadia Code/JetBrains Mono の見え方確認、README で推奨フォント明記 | 低 | 30min | 低 |
| **MAC-2** | accent 色拡張 | 現在 ACCENT 1 色 → secondary accent (gradient or hover state) 追加 | 低 | 1h | 中 |
| MAC-3 | focus visual 強化 | 現在 border 色変化のみ → 角丸の強調 + dim 化 で focus/blur 区別 | 低 | 1h | 低 |
| MAC-4 | spacing/padding 調整 | 8pt grid をより厳密に、status bar / title 余白の微調整 | 低 | 30min | 極小 |
| MAC-5 | micro-interaction | sub-bar 完了時の checkmark フェードイン演出 | 低 | 1.5h | 低 |

---

## 3. CLI 改善

| ID | 観点 | 内容 | 優先度 | 工数 | ROI |
|----|------|------|--------|------|-----|
| CLI-1 | terminal 幅対応 | 80 cols 環境で stage label が崩れる現象、`fmt::layer` の wrap 強化 | 中 | 30min | 中 |
| CLI-2 | error UX | anyhow chain の表示が縦長、`with_context` 連鎖の見やすさ改善 | 低 | 1h | 中 |
| **CLI-3** | --help 整形 | clap の `help_heading` で `Global Options` / `Commands` を分離 | 低 | 20min | 中 |
| CLI-4 | bash piped 検証 | tee/redirect 時の出力比較、ANSI 残留の有無を smoke test 化 | 低 | 30min | 低 |
| **CLI-5** | banner cosmetic | 固定 56 cols 制約 → terminal 幅追従 (Phase 1.5 から残置) | 低 | 30min | 中 |

---

## 4. dialoguer 検討 (Phase 1 deferred 再評価)

| ID | 観点 | 内容 | 優先度 | 工数 | ROI |
|----|------|------|--------|------|-----|
| DLG-1 | interactive 採用判断 | `note-auto run` で fzf 風カテゴリ multi-select | 低 | 4h | 低 |

**所感**:
- v0.7.6 で bat shim 集約 + v0.8.0 で TUI 完成 → **CLI 対話性の必要性は低下**
- dialoguer 入れるなら TUI 方向の延長で十分 (W7-G の sub-bar UX が既に高い)
- 設定 TOML での宣言的指定 (`configs/<cat>.toml`) で運用が固まっており、対話モードの必要性低
- → **v1.0.0 大型 phase 候補、v0.9.3 では deferred** 推奨

---

## 5. 既存 dead_code の整理 (隠れたタスク)

| ID | 観点 | 内容 | 優先度 | 工数 | ROI |
|----|------|------|--------|------|-----|
| DCX-1 | display.rs unused 削除 | `#[allow(dead_code)]` 18 件のうち、実 wire 機会のないものを削除 (Glyphs.warn / Palette.WARNING 等の Phase 2 wire 予定が未消費) | 低 | 1h | 中 |
| **DCX-2** | SubTick allow 解消 | WPW1 で実発火経路ができたので display.rs の `PipelineUpdate::SubTick` `#[allow(dead_code)]` を削除 | **高** | 5min | 高 |

DCX-2 は **WPW1 後の自然な fix**、本 phase で必須レベル。

---

## 推奨アクション

### v0.9.3 ミニ release (~3.5h、5 件採用)

```
1. DCX-2 (5min)    — SubTick allow 削除 (WPW1 自然解消)         ★MUST
2. CLI-3 (20min)   — clap --help 整形                          ★中
3. CLI-5 (30min)   — banner 幅追従                            ★中
4. W7-H-1 (30min)  — vim nav (h/j/k/l)                         ★中
5. W7-H-5 (1h)     — Logs ペイン PgUp/PgDn スクロール           ★中
合計 ~2h35min (バッファ込み 3.5h)
```

5 件すべて 1 PR にまとめても OK (ファイル衝突小)、または 2-3 PR 分離も可。

### v1.0.0 deferred 候補 (大型/ROI 待ち)

- DLG-1 (4h) — dialoguer interactive
- W7-H-3 (2h) — テーマ拡張
- MAC-2 (1h) + MAC-3 (1h) + MAC-5 (1.5h) — macOS polish パッケージ
- DCX-1 (1h) — dead_code 完全整理

### 不採用 (現状で十分)

- W7-H-2 / W7-H-4 / W7-H-6 / W7-H-7 / W7-H-8 — 体感価値小
- MAC-1 / MAC-4 — 必要なら README 1 行追記で済む
- CLI-2 / CLI-4 — 既存挙動で実害なし

---

## 6. v0.9.3 release 形式の判断

### 案 A: ミニ release (commander 推奨)
- 上記 5 件 (~3.5h) を 1〜2 PR で
- v0.9.3 タグ + Release
- v1.0.0 大型 phase に向けた基盤整備

### 案 B: skip → v1.0.0 直行
- v0.9.3 を見送り、v0.9.2 で UI 完成と認め v1.0.0 (大型 phase: dialoguer / macOS polish パッケージ) へ直行
- ROI 判断: ミニ release の運用コスト (タグ + Release Note 作成) が改善内容に見合うかどうか

### 私の推奨

**案 A: ミニ release (5 件、~3.5h)**

理由:
- DCX-2 は WPW1 自然解消で必須レベル、これだけでもリリース価値あり
- CLI-3 / CLI-5 は --help / banner の見栄え改善、ユーザ目線で即体感
- W7-H-1 / W7-H-5 は TUI ヘビーユーザの操作性向上
- 過去 Phase の 50% 短縮実績から実績 ~2h で完成見込み、commander マージ作業含めて 3-4h で v0.9.3 完結可能

---

## 7. 担当外への観察 (参考情報)

調査中に気づいた、他チーム領域の改善余地:

- **lowlevel 領域**: `--features tui` 時の binary size +3-5MB、`cargo bloat` 等で精査価値あり (lowlevel 判断)
- **quality 領域**: `tracing::info_span!` の field 数が field-by-field で再計算されるので tag/category 追加余地あり (quality silent 判断)

これらは ui-macos スコープ外、commander 経由で各チームに通知の参考。

---

## 次アクション

1. 本レポートを team-lead 上申
2. team-lead 判断 (案 A / 案 B / 別案)
3. 案 A 採用なら 5 件のうち更に絞るか全採用かを commander 判断
4. 着手 GO 後、worktree 作成 + 5 件を 1〜2 PR で完結 (~3-4h 見込み)

---

> 改善候補は 19 件発見、必須レベル 1 件 + 中優先 4 件で v0.9.3 ミニ release が現実的。
> 大型タスク不在で v1.0.0 候補も明確化、Phase 3.7 で UI 改修サイクル一段落の良い節目。
