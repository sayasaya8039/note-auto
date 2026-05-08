# Phase 1.5 (v0.7.7) — lowlevel 担当 scoring.rs 最適化調査

**担当**: lowlevel
**対象**: `src/scoring.rs` (96 行) + 補助 `src/util.rs::title_similarity`
**スコープ**: 調査のみ (実装は別タスクで GO 後)
**作成日**: 2026-05-05

---

## TL;DR

scoring.rs はホットパスではない（候補数 N = `top * 4` ≈ 12〜40 件、全体 I/O 数百 ms〜数秒に対して数 μs オーダー）。
**生 perf より「機能性 / 決定論性 / 警告解消」に振った 3 案** を推奨。

| # | 候補 | 種別 | 推定工数 | リスク | ROI 評価 |
|---|------|------|---------|--------|---------|
| **S1** | `ScoringConfig::dedup_threshold` の参照漏れ修正 | 機能 | 5 分 | 低 | **★★★ 高** |
| **S2** | bigram 事前計算キャッシュ (per-title HashSet を 1 回だけ生成) | perf | 20 分 | 低 | ★ 低〜中 |
| **S3** | ラウンドロビン補充ループの O(N²) → O(N) (HashSet で title 重複チェック高速化) | perf | 10 分 | 低 | ★ 低 |

実装優先度は **S1 > S3 > S2**。S1 だけは Phase 1.5 に必ず入れる価値あり。

---

## 現状の解析

### 処理フロー（scoring.rs::select_top）

```
items (Vec<TrendItem>, 数十〜数百件)
    │
    ▼
1. by_source: HashMap<String, Vec<TrendItem>> へグルーピング
    │
    ▼
2. ソース内 max 正規化 + ScoringConfig::source_weights による weight 適用
    │ → normalized: Vec<SelectedTrend>
    ▼
3. composite_score 降順ソート (Vec::sort_by + total_cmp)
    │
    ▼
4. queues: BTreeMap<String, VecDeque<SelectedTrend>> をソース別に構築
    │
    ▼
5. ラウンドロビン (max_rounds = top + |sources|): 各 source から
    │  bigram Jaccard >= 0.65 で重複しない候補を 1 件ずつ選定
    │
    ▼
6. 補充ループ: 不足分は normalized 全体をスキャンして埋める
    │
    ▼
7. selected: Vec<SelectedTrend>
```

### 性能特性

- N = 候補数（典型 30〜50、上限 200 想定）
- 重複判定は **bigram Jaccard 類似度 >= 0.65** (`util::title_similarity`)
- ラウンドロビン段階で `selected.iter().any(|s| title_similarity(...) >= 0.65)` を per-candidate 実行 → **O(N²) 比較 × O(L) bigram 構築コスト** ≒ N=50 で 2,500 回の HashSet 構築・intersection 計算
- 絶対時間: 1 回あたり ~数 μs、総計 数 ms 以下。fetch_all (数百 ms〜数秒) に完全に埋もれる。

### 機能上の不整合

- `ScoringConfig::dedup_threshold: f64` が **使われていない** (cargo build 時に warning: `field is never read`)
- 現状 0.65 がハードコード (scoring.rs:71, 86)
- v0.7.6 ビルドの 6 警告のうち 1 件はこれが原因

---

## 推奨案 詳細

### S1. `dedup_threshold` 参照漏れ修正 ★★★

**問題**: `ScoringConfig::dedup_threshold` は config TOML から読み取られているのに `select_top` 内で使われていない。0.65 ハードコード。

**修正案**:

```rust
// src/scoring.rs::select_top 冒頭
let scoring_cfg = crate::config::ScoringConfig::default();  // または cfg を引数で受け取る
let dedup_th = scoring_cfg.dedup_threshold;

// scoring.rs:71, 86 の `>= 0.65` を `>= dedup_th` に置換
```

**ROI 評価**:
- ★★★ 高: ① 既存 warning 解消、② config 機能の実機能化、③ 実装コスト 5 分、④ 副作用ほぼゼロ
- 既に `scoring::select_top(items, top)` のシグネチャ変更が必要 → cfg 引数追加で main.rs / daemon.rs 呼び出し 2 箇所修正発生
- **シグネチャ変更は破壊的** なので Phase 1.5 quality 主担当の config 触り (Q1/Q2 等) と整合確認必須

### S2. bigram 事前計算キャッシュ ★

**問題**: ラウンドロビン段階で 1 候補ごとに `title_similarity(s, cand)` を呼ぶ → `util::bigrams()` が両側で毎回 HashSet<String> を再構築する。N=50 で約 2,500 回の重複構築。

**修正案**:

```rust
// scoring.rs::select_top 内、ラウンドロビン前に 1 度だけ
let bigram_cache: HashMap<usize, HashSet<String>> = normalized.iter()
    .enumerate()
    .map(|(i, t)| (i, util::bigrams(&t.item.title)))
    .collect();

// title_similarity の代わりに inline で
fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() || b.is_empty() { return 0.0; }
    let inter = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    inter / union
}
```

**ROI 評価**:
- ★ 低〜中: 計算量は ~50 % 削減（構築コストが半分になる程度）だが、絶対時間は数 ms → 数百 μs
- 全体 I/O に埋もれるため **体感差ゼロ**
- ただし `util::bigrams` を `pub` 化する必要あり（現状 private）→ util.rs の API 変更
- util.rs は quality 主担当領域なので Phase 1.5 で衝突リスクあり

**結論**: 純粋 perf としては効果なし、quality との衝突を考えると **後回し or 却下**。

### S3. 補充ループ O(N²) → O(N) ★

**問題**: scoring.rs:84 の補充ループで `selected.iter().any(|s| s.item.title == cand.item.title)` を per-candidate 実行 → O(N²)。

**修正案**:

```rust
// 補充ループ前に
let mut chosen_titles: HashSet<&str> = selected.iter()
    .map(|s| s.item.title.as_str()).collect();

for cand in normalized {
    if chosen_titles.contains(cand.item.title.as_str()) { continue; }
    // ...
    if !dup {
        chosen_titles.insert(cand.item.title.as_str());  // borrow 注意、要 clone
        selected.push(cand);
        // ...
    }
}
```

**ROI 評価**:
- ★ 低: N=50 で 50 → 50 比較程度、絶対時間 ナノ秒オーダー
- 借用関係の調整 (`String` clone or `Rc<str>`) でわずかにコード複雑化
- **学術的には O(N) になるが体感差ゼロ**

**結論**: コードクリーン化の意味では悪くないが、急務ではない。

---

## 却下した案

| 案 | 却下理由 |
|----|---------|
| **rayon 並列化** | N が極小 (12〜40)。rayon の起動コスト > 利得。`select_top` は async にもなっておらず並列化メリット薄い。 |
| **HashMap → BTreeMap 統一 (決定論性)** | `by_source` は HashMap、`queues` は BTreeMap で混在。HashMap の iteration 順は不定だが、`normalized` の最終ソートで救われている。同点時の順序は不安定だが機能上の不整合無し。テスト追加時のみ価値ありで Phase 1.5 で必須ではない。 |
| **bigram の SmallVec / index 化** | grapheme cluster の安全な扱いを優先（絵文字対応）。`String` HashSet は重いが、N が極小で利得無し。大改修コストに見合わない。 |
| **scoring の SIMD/GPU/Zig FFI 化** | ローカル ML 不在のため原理的に効果なし（Phase 1 でも既に却下済）。 |

---

## 推奨実装プラン (Phase 1.5)

### Plan A (最小リスク・最大 ROI)

**S1 のみ実装**。
- `select_top(items, top, &cfg.scoring)` シグネチャ変更
- main.rs / daemon.rs の呼び出し 2 箇所修正
- `dedup_threshold` 参照、warning 解消、機能実装

**期待**: ビルド警告 6 → 5、機能実装漏れ解消。所要 ~10 分。

### Plan B (Plan A + クリーン化)

**S1 + S3 実装**。
- S3 の HashSet 化はおまけ。
- 体感差ゼロだがコードがクリーン。

**期待**: Plan A + N²→N、所要 ~20 分。

### 非推奨

S2 単独実装。util.rs API 変更を伴うため quality との衝突リスクが利得を超える。

---

## quality / ui-macos との衝突リスク

| 候補 | 触るファイル | quality 衝突 | ui-macos 衝突 |
|------|-------------|-------------|--------------|
| S1 | scoring.rs / config.rs / main.rs / daemon.rs | **config.rs 衝突あり** | main.rs (display 呼び出し追加) と編集ゾーン重複の可能性 |
| S2 | scoring.rs / util.rs | **util.rs 衝突あり** | なし |
| S3 | scoring.rs | なし | なし |

**S1 を投入する場合は config.rs 編集を quality と調整必須**。
S3 単独なら scoring.rs しか触らないため衝突ゼロ。

---

## L6 (Cargo.toml deps バージョン更新) 状況

現状 Cargo.toml の主要 UI 依存:
- `indicatif = "0.17"` (現行最新は 0.17.x、bump 不要)
- `console = "0.15"` (0.15.x、bump 不要)
- `owo-colors = "4"` (4.x、bump 不要)
- `comfy-table = "7"` (7.x、bump 不要)
- `supports-color = "3"` (3.x、bump 不要)

**全て semver メジャーが妥当**。ui-macos の wire-up で機能不足が出た場合のみ上申を待ちます。

---

## 次のアクション

- 本レポートを commander に送付
- S1 / S3 実装は team-lead + commander 承認後に PR-D として着手
- quality との config.rs 編集調整は commander 経由で要確認

以上。
