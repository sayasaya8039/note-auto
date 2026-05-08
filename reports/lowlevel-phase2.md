# Phase 2 (v0.8.0) — lowlevel 調査レポート

**担当**: lowlevel
**作成日**: 2026-05-05
**対象 main HEAD**: `4a38ffc` (v0.7.7 リリース後)
**スコープ**: Tier 1 (必達) + Tier 2 (実測ベース判断)
**実装はゼロ・調査のみ**

---

## TL;DR

| 項目 | 評価 | 推奨 |
|------|------|------|
| **#1 buffer_unordered** | ★★★ 必須投入 | `writer::run` の `join_all` を `buffer_unordered(N=3)` に置換、`fetch_all` は据え置き、og_image enrich は閾値変更 |
| **#2 tracing 計測** | ★★★ 必須投入 | 4 stage に `info_span!` 追加（fetch / score+dedup / write / publish）、`Instant` 測定の冗長を削減 |
| **#3 quick-xml SAX** | ★ 却下 | `rss` / `atom_syndication` クレート API を保持する以上、SAX 直書きは逆行。クレート差し替えの大改修無しでは効果なし |
| **#4 simd-json** | ★★ 採用候補 (実測必須) | AI レスポンス 30〜100KB がボーダーゾーン。anthropic/xai のみ局所差し替えで A/B 計測可能 |
| **#5 jemalloc ベンチ** | ★ 保留 | 現 mimalloc は small-medium alloc に最適、note-auto の workload と整合。bench 数字無しに切り替えは非推奨 |

最高 ROI は **#1 + #2** の組み合わせ。両方とも工数 30〜60 分・低リスクで Phase 2 の最初の改善として最適。

---

## Tier 1 — 必達

### 1. buffer_unordered 並列度制御

#### 現状の並列度マップ

| 場所 | 現状 | 並列度 | 並列対象 | 問題 |
|------|------|--------|---------|------|
| `trends::fetch_all` | `join_all(futs)` | 最大 11 (sources 数) | x/google/gnews/note/hn/reddit/konbini/hyakkin/... | source ごとに違う API なので **rate-limit クロス汚染なし**、現状で OK |
| `trends::gnews_rss::enrich_og_images` | `join_all(targets.iter())` + `take(parallelism)` | parallelism (config) | OG image 取得 | `take(N)` は対象数の上限、並列度制御ではない (Phase 1 既出) |
| `writer::run` (★) | `join_all(tasks)` | **記事数 (typically 3〜21)** | write_one() = research+brief+body+4画像 全部 | **★ 最大の問題**: AI 同時呼び出しで rate-limit 直撃 |
| `writer::write_one` 内部 | `tokio::join!(draft, images)` | 2 | draft / images 並列 | 1 記事内、AI 1 系統ずつなので OK |
| `writer::*` 画像生成 | `join_all(futs)` | 4 (固定) | 画像 4 枚並列 | 1 記事内、Pollo / NVIDIA 等別々プロバイダなので OK |
| `publish::publish_all` | `stream::iter().buffered(2)` ✅ | 2 | note 投稿 + X 告知 | **Q4 で既に実装済** |

#### 推奨

##### A. `writer::run` (line 47) — **最優先**
```rust
// 現状: join_all で全記事同時並列
let results = join_all(tasks).await;

// 推奨: buffer_unordered(3) で並列度 3
use futures::stream::{self, StreamExt};
let results: Vec<_> = stream::iter(tasks)
    .buffer_unordered(cfg.writer.parallelism.unwrap_or(3))
    .collect().await;
```

**根拠**:
- 1 記事 = research(Grok) + brief(Haiku) + body(Claude) + 4 画像(Pollo/NVIDIA) ≒ 7 API 呼び出し
- 同時 21 記事だと最大 147 並列 API 呼び出し → **Anthropic / Pollo の rate limit 確実に超過**
- N=3 なら 21 並列で安全圏、ストリーム形式で順次完了 → 早い記事から log 出現
- config 化: `cfg.writer.parallelism: Option<usize>` (default 3)

**工数**: 20 分（実装 + ビルド + smoke test）

##### B. `gnews_rss::enrich_og_images` — 補正
```rust
// 現状: take(N) 後に join_all (実質「N 個拾って全並列」)
let targets: Vec<_> = items.iter().enumerate().filter_map(...).take(parallelism).collect();
let results = join_all(targets.iter().map(...)).await;

// 推奨: buffer_unordered で本来の並列度制御
let results: Vec<_> = stream::iter(items.iter().enumerate().filter_map(...))
    .map(|(idx, url)| async move { ... })
    .buffer_unordered(parallelism)
    .collect().await;
```

**根拠**: 意味論明確化、`take(N)` で「N 個」上限指示と「同時 N 並列」を分離。
**工数**: 10 分

##### C. `trends::fetch_all` — **据え置き推奨**
**根拠**: 各 source は別 API (X / Google News / Reddit Algolia / RSS / Playwright sidecar 等)。同時実行で rate-limit クロス汚染なし。並列度制限はメリット小。

#### 全体工数
A + B 合計 **30 分**。

---

### 2. tracing レイテンシ計測

#### 現状

| stage | 計測 | 場所 |
|-------|------|------|
| fetch_all | なし | `info!(source = src, count = items.len(), "fetched")` のみ (per-source) |
| score+dedup | なし | `info!(selected, skipped_dup, "trends selected")` のみ |
| writer::run | なし | per-article `info!(slug, chars, "article written")` のみ |
| publish_all | なし | per-article のみ |
| 全体 | `Instant::now()` + `start.elapsed()` | `daemon::execute_cycle` / `main::Publish` で粗い計測 |

→ **stage 別の所要時間が log にも tracing にも出ていない**。改善前後の効果測定不能。

#### 推奨

##### A. `tracing::info_span!` で全 stage 自動計測
```rust
// daemon::execute_cycle 内
let span_fetch = tracing::info_span!("fetch_all", sources = source_count);
let items = fetch_all(cfg).instrument(span_fetch).await?;
// → 自動的に "elapsed_ms" が log に出る (RUST_LOG=info,note_auto=debug 時)
```

または `#[tracing::instrument]` を `fetch_all`, `select_top`, `writer::run`, `publish_all` に attach。

##### B. enrich_og_images / write_one / per-source 計測（Tier 1 後の Tier 1.5 候補）
- per-article 計測 → どの記事が遅いか可視化
- per-source 計測 → どの API が遅延の主原因か分離

#### 計測点 4 箇所（Tier 1 必達）
1. `trends::fetch_all` — fields: source_count, total_items
2. `scoring::select_top + history dedup` — fields: input_count, selected_count, skipped_dup
3. `writer::run` — fields: trend_count, written_count, total_chars
4. `publish::publish_all` — fields: article_count, success_count

#### 実装イメージ
- `tracing-subscriber` の `with_filter` を `info` で出力する設定（既存 `init_with` で対応済）
- `tracing::Instrument` trait を `use` して `.instrument(span)` を await チェーンに挟む
- log 例:
  ```
  [INFO note_auto] fetch_all: source_count=11, total_items=87, elapsed=4.2s
  [INFO note_auto] select_top: input=87, selected=3, skipped=2, elapsed=12ms
  [INFO note_auto] writer::run: trends=3, written=3, chars=15234, elapsed=412s
  [INFO note_auto] publish_all: articles=3, success=3, elapsed=23s
  ```

#### 工数
**30 分** (4 stage + Cargo.toml に `tracing-subscriber` の `time` feature 確認 + smoke test)

---

## Tier 2 — 実測ベース判断

### 3. quick-xml SAX モード移行

#### 現状
- `rss = "2"` クレート: `Channel::read_from(&[u8])` で DOM 構築。内部 `quick-xml` で SAX → DOM 変換
- `atom_syndication = "0.12"` クレート: `Feed::read_from(&[u8])` で DOM 構築。同上
- 直接利用箇所: `trends/google.rs`, `trends/google_news.rs`, `trends/note_rss.rs`, `trends/gnews_rss.rs`, `trends/reddit.rs`

#### 計測（推定）
- RSS payload size: 5〜100 KB
- パース時間: 数 ms オーダー（DOM 構築含む）
- I/O (HTTP) 時間: 数百 ms
- → **パース時間は I/O の 1〜5%**、SAX で削れる絶対時間は微小

#### 評価
- **クレート API を保持** → SAX 化は内部実装変更で恩恵を享受できない
- クレート差し替え（`quick-xml` 直接使用 + 自前 SAX ハンドラ）→ google.rs/note_rss.rs/gnews_rss.rs/reddit.rs の **4 ファイル全書き換え** = 大改修
- 改善見込み: I/O 時間が支配的 → 体感差ゼロ

#### 採用可否
**却下**。Tier 2 で計測する価値も低い。

---

### 4. simd-json 検討

#### 現状
| AI client | レスポンス size 推定 | parse 場所 |
|-----------|-------------------|-----------|
| anthropic.rs (Claude Sonnet body) | **30〜100 KB** | `serde_json::from_str` |
| xai.rs (Grok research) | 5〜50 KB | `serde_json::from_str` (sometimes via `strip_code_fence`) |
| openai.rs / gemini.rs / nvidia.rs (画像生成) | <5 KB | `serde_json::from_str` |
| pollo.rs | <5 KB | 同上 |
| trends/hn.rs (HN Algolia) | 数 KB | `.json::<Resp>()` |

#### 計測（推定）
- serde_json: 30〜100 KB JSON で **0.5〜2 ms**
- simd-json: 同上で **0.1〜0.5 ms** (3〜5x speedup)
- 1 記事あたり parse 回数: ~3 回 (research + brief + body)
- 全体削減見込み: ~2〜5 ms × 3 = **~10 ms / 記事**

→ AI 呼び出し 1 回が数十秒なので、**全体への寄与は無視可能**。

#### 採用条件
- **anthropic.rs / xai.rs だけ局所差し替え** で A/B 計測（10 分）
- Windows MSVC + zigbuild 環境で simd-json が安定動作するか確認必須（過去に AVX2 detection で問題報告あり）
- 数値で 5% 以上の改善が出れば採用

#### 採用可否
**実測必須・条件付き採用候補**。Tier 1 完了後に bench 数字で判定。

#### 工数
- 局所差し替え + bench: 30 分
- 全面採用: 1 時間

---

### 5. mimalloc → jemalloc ベンチ

#### 現状
- v0.7.6 で mimalloc を Windows global allocator に採用
- workload: HashMap/HashSet (scoring/history) + JSON parse (AI レスポンス) + String allocation (bigram/title) → **small-medium alloc 中心**

#### 比較
| allocator | 強み | note-auto 適合度 |
|-----------|------|-----------------|
| **mimalloc** (現行) | small-medium alloc, multi-thread, low fragmentation | ★★★ 最適 |
| jemalloc | medium-large alloc, arena 分離, scaling | ★★ note-auto は medium-large alloc が少ない |
| system (HeapAlloc) | n/a | ★ 既に置き換え済 |

#### 評価
- mimalloc 維持が定石。jemalloc 試行は **数値で 5% 以上の改善が出た場合のみ** 採用
- Windows MSVC + zigbuild で `jemallocator` クレートが動作するか要確認 (Linux/macOS では実績多)
- bench 計測しない限り判断不能 → 優先度低

#### 採用可否
**保留**。Phase 2 後半で時間に余裕あれば bench 検討。

---

## 推奨実装プラン (Phase 2 実装フェーズ向け)

### 優先順位

| # | 項目 | 優先度 | 工数 | 依存 |
|---|------|--------|------|------|
| 1 | **#1A** writer::run の buffer_unordered 化 | ★★★ | 20 min | なし |
| 2 | **#2** tracing info_span 計測 4 箇所 | ★★★ | 30 min | なし |
| 3 | **#1B** gnews_rss enrich_og_images の正規化 | ★★ | 10 min | なし |
| 4 | (計測実行) **#2 で得たログ** から #4 / #5 の必要性判断 | ★★ | smoke run 1 回 | #2 完了後 |
| 5 | **#4** simd-json 局所差し替え (anthropic/xai) | ★ | 30 min | #2 計測結果 |
| 6 | **#5** jemalloc bench | ★ | 60 min | #2 計測結果 + 余裕 |

### 並列実装可能性
- **#1A + #2** は完全独立 → 同時並列で 1 つの PR に統合可
- **#1B** は writer 領域 (#1A) と同 PR にまとめても、別 PR でも OK
- **#4 / #5** は #2 計測結果ベース、Phase 2 後半着手

### Phase 2 第 1 弾 PR (PR-I 仮称) 推奨スコープ
- L9 `writer::run` buffer_unordered (config-driven N=3 default)
- L10 4 stage に `tracing::info_span!` 計測
- L11 `gnews_rss::enrich_og_images` の `take` → `buffer_unordered`
- 触るファイル: `Cargo.toml` (futures features 確認) / `src/writer/mod.rs` / `src/trends/mod.rs` / `src/trends/gnews_rss.rs` / `src/daemon.rs` / `src/main.rs` / `src/config.rs` (parallelism field)

### 衝突リスク
- ui-macos: `display.rs / main.rs / daemon.rs` 編集可能性 → **計測 span 追加は main/daemon の数行のみ、行レベルで分離可能**
- quality: `writer/publish/history` 編集可能性 → **#1A は writer/mod.rs の line 47 周辺だけ、Q1-Q4 修正と物理分離可**

### 期待効果（v0.7.7 比）
- **rate-limit 安全化**: 同時 21 記事 × 7 API → 同時 3 記事 × 7 API (~70 % 削減)
- **観測性向上**: stage 別所要時間が log に出る → Phase 2 の他改善 (simd-json / jemalloc) の効果測定基盤
- **コード品質**: `take(N)` 並列誤読修正、意図明確化

---

## 補足: Cargo.toml deps 見直し（Phase 2 候補）

| 依存 | 現行 | 検討 |
|------|------|------|
| `futures = "0.3"` | OK | `stream` feature 既に default、変更不要 |
| `tokio` (8 features) | OK | `tracing` 連携要否次第で `tracing` feature 検討 |
| `tracing-subscriber` | env-filter+fmt | OK (`time` feature は default) |
| `simd-json` (新規) | — | Phase 2 後半で実測後 |
| `jemallocator` (新規) | — | Phase 2 後半で実測後 |

---

## 次のアクション

- 本レポートを commander に送付
- Phase 2 第 1 弾 PR 着手は team-lead + commander 承認後
- #1A + #2 + #1B を 1 PR で実装するか 2 分割するかは commander 判断

以上。30 分以内提出完了。
