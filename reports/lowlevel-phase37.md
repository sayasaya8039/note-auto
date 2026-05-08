# v0.9.3 (Phase 3.7) lowlevel 調査レポート

**担当**: lowlevel
**作成日**: 2026-05-06
**対象 main HEAD**: `04f119a chore: bump version to 0.9.2`
**スコープ**: 自由スコープ (5 軸: CL2 / 依存 / perf / error / test)
**実装ゼロ・候補列挙**

---

## サマリ

| 項目 | 件数 |
|------|------|
| 改善候補 | **17 件** |
| ミニ release 推奨候補 | **5 件 (~2h)** |
| v1.0.0 大型 phase 候補 | 7 件 |
| 却下 | 5 件 |

**TL;DR**: v0.9.3 は **CL2 (clippy pedantic 部分採用) + urlencoding crate 削除 + scoring/util.rs unit test 追加** の 3 軸で `~2h` のミニ release 推奨。thiserror 移行 / 全 client mock test / SIMD 検討は v1.0.0 以降に deferred。

---

## 1. CL2 — clippy strict 適用

実機 `cargo clippy --release -- -W clippy::pedantic -W clippy::nursery` 未実行 (時間制約)。コードベース観察ベースで予想される警告分類:

| ID | 観点 | 場所(推定) | 内容 | 優先度 | 工数 | ROI |
|----|------|-----------|------|-------|------|-----|
| CL2-1 | needless_pass_by_value | ai/* / writer/* | `String` 受けを `&str` に | 中 | 30min | 微小 perf + 慣用句 |
| CL2-2 | redundant_clone | ai/anthropic.rs / xai.rs | `.clone()` 不要箇所 | 低 | 10min | 微小 |
| CL2-3 | must_use_candidate | pub fn 多数 | `#[must_use]` 推奨 | 低 | 15min | 警告防止 |
| CL2-4 | missing_errors_doc | pub Result 返却 fn | doc 欠落 | 中 | 30min | doc 整備 |
| CL2-5 | missing_panics_doc | unwrap/expect 含む pub fn | doc 欠落 | 中 | 15min | doc 整備 |
| CL2-6 | module_name_repetitions | ai::AiClient | 慣用句的命名 | 低 | n/a | **却下推奨** (Rust idiom) |
| CL2-7 | cast_possible_truncation | scoring.rs (f64 cast) | safety check 不足 | 中 | 20min | safety |
| CL2-8 | unnecessary_wraps | helper fn | `Some(x)` のみ返す | 低 | 15min | 微小 |

**実機判定推奨**: 30 分の `cargo clippy --release -- -W clippy::pedantic` 走行で正確な件数 + 場所が出る。本レポートは予想ベース。

**推奨**: CL2 は **実機実行で発生件数判定** → 50 件以上なら部分採用 (CL2-7 cast_possible_truncation のような safety 系のみ)、20 件以下なら全件採用。

---

## 2. 依存整理

### 現状 Cargo.toml (~30 deps)

| crate | 用途 | 削除可能性 |
|-------|------|-----------|
| **urlencoding** | gnews_rss.rs:27 のみ | **🟢 削除候補** (`url::form_urlencoded` で代用) |
| `slug` | note slug 生成 | 維持 (専門用途) |
| `tokio-cron-scheduler` | daemon mode | 維持 |
| `hmac` + `sha1` | x_post.rs OAuth 1.0a | 維持 |
| `chrono` + `chrono-tz` | 時刻処理 | 維持 |
| `atom_syndication` + `rss` | reddit (atom) + 他 (rss) | 両方維持 |
| `html-escape` | util.rs::strip_html | 維持 |
| `unicode-segmentation` | bigram | 維持 |
| `regex` | gnews_rss / util.rs::RE_HTML | 維持 |
| `dotenvy` | .env 読込 | 維持 |
| `base64` | image b64 decode | 維持 |
| `async-trait` | AiClient trait | 維持 |

### 候補

| ID | 内容 | 優先度 | 工数 | ROI |
|----|------|-------|------|-----|
| **DEP-1** | **`urlencoding` crate 削除 → `url::form_urlencoded`** | **★ 採用推奨** | 15min | バイナリ -数 KB、deps 1 件削減 |
| DEP-2 | `cargo tree --duplicates` で重複 dep 確認 | 中 | 10min | semver-major 重複の発見 |
| DEP-3 | `cargo udeps` で未使用 dep 検出 (nightly) | 中 | 30min | 自動検出、要 nightly toolchain |
| DEP-4 | `cargo audit` で security 脆弱性スキャン | 高 | 5min | CVE 検出、CI 統合検討 |

**推奨**: **DEP-1** をミニ release (CL2 と同 PR で同梱可)、**DEP-4** を CI に統合 (cargo-audit-action) で継続的監視。

### Supply chain 観点
- **risky deps なし**: 全 crate が広く使われている主要 crate
- `slug` (個人 maintainer) のみ若干心配だが代替なし、現状容認

---

## 3. 微小 perf

### AF3 bench で計測済 (v0.9.0)
- `title_similarity_short`: ~5 µs
- 他 (`title_similarity_long` / `bigrams_construction` / json_parse 系 3 件): 未計測 → workflow_dispatch で実行可能

### 新規候補

| ID | 内容 | 優先度 | 工数 | ROI |
|----|------|-------|------|-----|
| PERF-1 | `serde_json::from_str` → `from_slice` | 低 | 各 client +1 line | 微小 (1 copy 削減) |
| PERF-2 | bigram の `String` → `(usize, usize)` index pair | 低 | scoring 全書直し | 5-10% scoring 高速化、N 小で全体寄与微小 |
| PERF-3 | `download_source_images` の per-URL client cache (同 host で再利用) | 低 | util.rs +20 lines | 数 ms 削減、3 件/サイクルで埋もれる |
| PERF-4 | `regex::Regex::new` の `LazyLock` 化 (gnews_rss::extract_og_image) | 中 | 5min | OG enrich N 件で µs オーダー削減 |

**Phase 2 で却下した P2/P3** (simd-json / jemalloc) は **再評価不要** (workload に対して無効と確定済)。

**推奨**: **PERF-4** のみ採用候補 (低工数、明確効果)。他は v1.0.0 phase で実機 bench 後に判定。

---

## 4. error handling 改善

### 現状
- 全体に `anyhow::Result` + `.with_context(...)` で context 補強済
- ただし以下の場所で context 薄い箇所あり:
  - `ai/*::call` → `{label} API {status}: {body}` のみ、URL や retry 経歴なし
  - `trends/*::fetch` → source 名のみ、URL や HTTP status 不明
  - `writer/mod.rs::run` 内の各 await `?` 連鎖でブロブ的 error

### 候補

| ID | 内容 | 優先度 | 工数 | ROI |
|----|------|-------|------|-----|
| ERR-1 | `with_context` を全主要 `?` 直前に追加 | 低 | 1h | debug 容易化、verbose |
| ERR-2 | **thiserror 導入** (構造化 error) | 低 | **大型 (4-6h)** | 型安全性、再利用性。**v1.0.0 deferred 推奨** |
| ERR-3 | Slack notify の error context 強化 | 中 | 30min | production debug 容易化 |
| ERR-4 | `tracing::error!` の field 充実化 | 中 | 30min | 構造化ログで原因特定容易 |

**推奨**: **ERR-3 + ERR-4** をミニ release 候補 (production 運用 quality 直結)。**ERR-2 thiserror** は anyhow との trade-off 検討 + 大型工数で v1.0.0 phase。

---

## 5. テスト追加

### 現状カバレッジ
| ファイル | 既存 unit test |
|---------|---------------|
| util.rs | 5 件 (M3-C 由来: allowlist + IP unsafe) |
| display.rs | 11 件 (ui-macos 由来) |
| scoring.rs | **0 件** |
| ai/* | **0 件** |
| writer/* | **0 件** |
| publish/* | **0 件** |
| trends/* | **0 件** |

→ **コア logic (scoring / ai / writer / publish / trends) のテストゼロ**

### 候補

| ID | 内容 | 優先度 | 工数 | ROI |
|----|------|-------|------|-----|
| **TEST-1** | **scoring.rs::select_top の 5 シナリオ test** (empty / single / dedup / round-robin / fallback) | **★ 高 採用推奨** | 30min | 純粋関数、mock 不要、即書ける |
| **TEST-2** | **util.rs::title_similarity の境界 test** (空 / 同一 / 完全異 / unicode) | **★ 高 採用推奨** | 15min | 同上 |
| TEST-3 | util.rs::strip_html / strip_code_fence の境界 test | 中 | 20min | regex 周りの保証 |
| TEST-4 | send_with_retry の retry 動作 mock test (hyper-test 等) | 中 | 1.5h | production silent loss 予防 |
| TEST-5 | check_and_pin_image_client の DNS rebinding mock | 低 | 大型 (2h) | M3-C 動作保証、mock 構築重い |
| TEST-6 | AiClient trait の build_request シグネチャ test | 低 | 30min | refactor 安全性 |
| TEST-7 | daemon::execute_cycle の e2e dry-run test | 低 | 大型 (3h) | mock 多すぎ、CI 時間 |

**推奨**: **TEST-1 + TEST-2** をミニ release 候補 (合計 45 分、純粋関数で書きやすい、現状ゼロからの脱却が大きい)。TEST-4 (mock retry) は v1.0.0 phase で。

---

## 推奨アクション

### v0.9.3 ミニ release (`~2h`)
1. **DEP-1** urlencoding 削除 (15min)
2. **CL2 部分採用** — 実機 clippy pedantic 走行 → safety 系のみ採用 (30min 〜 1h)
3. **PERF-4** gnews_rss::extract_og_image 正規表現 LazyLock 化 (5min)
4. **TEST-1** scoring::select_top unit test (30min)
5. **TEST-2** title_similarity 境界 test (15min)

合計 ~1.5-2.5h、PR は 2-3 本に分割推奨:
- PR-A: DEP-1 + PERF-4 (mechanical changes、衝突ゼロ)
- PR-B: CL2 部分採用 (実機 clippy 後判断)
- PR-C: TEST-1 + TEST-2 (純粋関数 unit test)

### v1.0.0 大型 phase 候補 (deferred)

| ID | 内容 | 工数 |
|----|------|------|
| ERR-2 | thiserror 導入 (構造化 error) | 4-6h |
| TEST-4 | send_with_retry mock retry test | 1.5h |
| TEST-5 | check_and_pin_image_client DNS rebinding mock | 2h |
| TEST-7 | daemon::execute_cycle e2e dry-run test | 3h |
| PERF-2 | bigram index pair 化 (大型 scoring refactor) | 4h |

### 却下

- **CL2-6** module_name_repetitions: Rust idiom (e.g. `ai::AiClient`)、`#[allow(clippy::module_name_repetitions)]` workspace 設定が妥当
- **simd-json / jemalloc 再評価**: Phase 3 既に確定却下
- **lib 化** (src/lib.rs 作成 → bench 直接利用): 大規模 refactor、effort vs benefit 不見合い

### CI 強化候補 (任意)

| ID | 内容 | 工数 |
|----|------|------|
| CI3 | `cargo audit` を ci.yml に追加 (security 脆弱性 CI 検出) | 15min |
| CI4 | `cargo deny` で license / dup check (cargo-audit との trade-off) | 30min |
| CI5 | code coverage (`cargo llvm-cov`) を bench.yml と同レベルの workflow_dispatch で起動 | 30min |

CI3 は v0.9.3 で同梱推奨 (~15min、production safety 直結)。

---

## 結論

**推奨実装プラン**: v0.9.3 を **DEP-1 + PERF-4 + TEST-1 + TEST-2 + CI3** の **5 件ミニ release** で締めくくる。**CL2 (clippy pedantic)** は実機計測ベースで部分採用判断、**ERR-2 thiserror** + **TEST-4/5/7** は v1.0.0 大型 phase へ deferred。

→ note-auto は v0.9.0-0.9.3 で **production-ready security + observability + quality** を達成、v1.0.0 で **maintainability + testability** の最終段階に進む段階的成熟が見えた状態。

以上、30 分以内提出。
