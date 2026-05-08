# v0.9.3 (Phase 3.7) 統合スコープ提案

**作成日**: 2026-05-06
**作成者**: commander
**対象 main HEAD**: `04f119a chore: bump version to 0.9.2`
**ベース**: `reports/lowlevel-phase37.md` + `reports/ui-macos-phase37.md`

---

## 統合 TL;DR

**両担当ともに「v0.9.3 はミニ release」を推奨**。明確な必須課題は少なく、Phase 3 のテーマ (security + UX + quality) は v0.9.0/0.9.1/0.9.2 で完成、v0.9.3 は **polish + dead_code 整理 + 最低限の test coverage 底上げ + supply chain 監視追加** に絞る方向。

**改善候補総数**: 36 件 (ui-macos 19 + lowlevel 17、観察 + 重複考慮で 35 件相当)
**ミニ release 採用候補**: 11 件 + CI3 (~5.5h、実績 50% 短縮で ~3h 想定)

---

## 統合採用候補 (Plan A: ミニ release)

### 必須レベル
| ID | 担当 | 工数 | 理由 |
|----|------|------|------|
| **DCX-2** SubTick allow_dead_code 削除 | ui-macos | 5min | WPW1 で実発火経路ができたので必須 |

### 即体感価値
| ID | 担当 | 工数 | 効果 |
|----|------|------|------|
| **CLI-3** clap --help 整形 (help_heading) | ui-macos | 20min | --help 出力が Global/Commands 分離で見やすく |
| **CLI-5** banner terminal 幅追従 | ui-macos | 30min | Phase 1.5 残置の固定 56 cols 解消 |
| **DEP-1** urlencoding crate 削除 | lowlevel | 15min | バイナリ -数 KB、deps -1 |
| **PERF-4** gnews_rss regex LazyLock | lowlevel | 5min | OG enrich の regex 再構築排除 |

### TUI ヘビー user 価値
| ID | 担当 | 工数 | 効果 |
|----|------|------|------|
| **W7-H-1** vim nav (h/j/k/l) | ui-macos | 30min | Tab 補助、ペイン navigation |
| **W7-H-5** Logs PgUp/PgDn scroll | ui-macos | 1h | 履歴遡行可能 (現状 tail 固定) |

### test coverage 底上げ
| ID | 担当 | 工数 | 効果 |
|----|------|------|------|
| **TEST-1** scoring::select_top 5 シナリオ | lowlevel | 30min | コア純粋関数、現状テスト 0 件 |
| **TEST-2** title_similarity 境界 test | lowlevel | 15min | 同上 |

### supply chain
| ID | 担当 | 工数 | 効果 |
|----|------|------|------|
| **CI3** cargo audit を ci.yml に追加 | lowlevel | 15min | CVE 検出の継続自動化 |

### 任意検討
| ID | 担当 | 工数 | 効果 |
|----|------|------|------|
| **CL2 部分採用** | lowlevel | 30min-1h | clippy pedantic 実機実行 → safety 系のみ採用 |

**ミニ release 合計**: ~3.5-5.5h (CL2 を含めるかで変動、過去 50% 短縮実績で実績 ~2-3h 想定)

---

## v1.0.0 大型 phase へ deferred (推奨)

| ID | 担当 | 工数 | 理由 |
|----|------|------|------|
| ERR-2 thiserror 導入 | lowlevel | 4-6h | anyhow との trade-off 検討 + 大型 |
| TEST-4 send_with_retry mock retry | lowlevel | 1.5h | mock 構築 |
| TEST-5 DNS rebinding mock | lowlevel | 2h | mock 構築 |
| TEST-7 e2e dry-run test | lowlevel | 3h | mock 多すぎ |
| PERF-2 bigram index pair | lowlevel | 4h | 大型 scoring refactor |
| DLG-1 dialoguer interactive | ui-macos | 4h | TUI 完成で必要性低、v1.0.0 |
| W7-H-3 theme 拡張 (solarized/dracula) | ui-macos | 2h | 低優先 |
| MAC-* macOS-style 仕上げ 5 件 | ui-macos | 4h | パッケージ実装で v1.0.0 |
| 残 W7-H-2/4/6/7/8 | ui-macos | 5h | 体感小 |

**deferred 合計**: ~30h、v1.0.0 大型 phase でパッケージ実装

---

## 却下

| ID | 担当 | 理由 |
|----|------|------|
| CL2-6 module_name_repetitions | lowlevel | Rust idiom、`#[allow]` workspace 設定で OK |
| simd-json / jemalloc 再評価 | lowlevel | Phase 3 既に確定却下 |
| lib 化 (src/lib.rs) | lowlevel | 大規模 refactor、ROI 低 |
| MAC-1/4 / CLI-2/4 等 8 件 | ui-macos | 現状で十分 |

---

## 担当外観察 (相互レポート)

ui-macos が報告した lowlevel/quality 領域の改善余地:
- **lowlevel**: `--features tui` 時の binary +3-5MB、`cargo bloat` 精査余地 (v1.0.0 候補)
- **quality**: `tracing::info_span!` field の冗長性、tag/category 追加余地 (v1.0.0 候補)

→ ui-macos の観察は v1.0.0 phase で活用、本リリースでは取り入れず。

---

## 提案

### 採用パターン

#### Plan A (commander 推奨): 11 件 + CL2 任意 採用
- 計 ~5.5h、実績 ~3h
- 必須 1 + 即体感 4 + TUI 価値 2 + test 2 + CI 1 + 任意 CL2
- 並行戦略: lowlevel/ui-macos 各 worktree、衝突ゼロ
- v0.9.3 を **小ぶりなれど価値ある polish release** で締める

#### Plan B: 必須のみ (DCX-2 + DEP-1 + PERF-4 + CI3)
- 計 ~40min
- 最小 release
- v0.9.3 を **メンテリリース**として早期公開

#### Plan C: skip → v1.0.0 直行
- 現状 v0.9.2 を最終 0.9.x として、v1.0.0 を大型 phase で計画
- ミニ release のオーバーヘッドを節約

### 並行戦略 (Plan A 採用時)
- **lowlevel worktree (`note-auto-phase37-lowlevel`)**: DEP-1 + PERF-4 + TEST-1 + TEST-2 + CI3 (+ CL2 任意) → 1-2 PR で完結
- **ui-macos worktree (`note-auto-phase37-ui`)**: DCX-2 + CLI-3 + CLI-5 + W7-H-1 + W7-H-5 → 1-2 PR で完結
- 衝突予測: 完全独立 (util/scoring/ci.yml/Cargo.toml vs cli/tui.rs/display.rs)
- 過去 v0.9.0/0.9.1/0.9.2 の実績で 50% 短縮見込み

---

## commander 推奨: **Plan A**

理由:
1. v0.9.3 を **「小さくとも価値ある polish」** として完結、Phase 3 の流れを綺麗に締める
2. dead_code 整理 + supply chain 監視 + test coverage 底上げで **基礎体力強化**
3. CLI/TUI polish で **end user UX 向上** (CLI-3/5 + W7-H-1/5)
4. ~3h の実績工数、両担当の負担小、ROI 高
5. v1.0.0 大型 phase の準備として基礎が整う

ただし team-lead が Plan B (最小) or Plan C (skip) を選択する場合も合理的。

---

## 判断仰ぎ

team-lead に以下を上申:

1. **Plan A / Plan B / Plan C / 別案** の選択
2. Plan A 採用時の **CL2 部分採用 vs 全採用 vs 不採用** の方針
3. v1.0.0 大型 phase の **時期目安** (v0.9.3 完了直後 vs 1〜2 週間後)
4. `silent-worker-as-quality-reviewer` 0.85 → 0.90 bump 機会の評価 (本 phase で適用？)

GO/STOP/差戻しでご指示お願いします。
