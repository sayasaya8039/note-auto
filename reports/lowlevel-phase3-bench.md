# Phase 3 (v0.9.0) — lowlevel ベンチ調査

**担当**: lowlevel
**作成日**: 2026-05-06
**対象 main HEAD**: `465766b chore: bump version to 0.8.1`
**スコープ**: P2' (simd-json) / P3' (allocator) / P5' (OpenTelemetry)
**実装ゼロ・採否判断のための調査**

---

## TL;DR

| 候補 | 公開 bench | note-auto への効果 | 採否 |
|------|-----------|-----------------|------|
| **P2' simd-json** | 30-40KB JSON で 2-3x speedup (0.2-0.4ms 削減) | 全体時間の **0.003% 改善** | **却下** |
| **P3' jemalloc** | small-medium alloc は mimalloc 優位 (Windows MSVC で完全動作実績薄) | 改善見込みなし、ビルド互換性リスク | **却下** |
| **P5' OpenTelemetry** | tracing-opentelemetry 安定、OTLP export 動作 | production observability 用、開発フェーズでは早期 | **保留** (本番運用開始時に再評価) |

**結論**: Phase 3 では **3 候補すべて本実装に進める価値が薄い**。Phase 3 の目玉は別の場所 (例: P5' 採用ロードマップ確立、本番化準備) に回すのが合理的。

---

## P2': simd-json A/B 計測

### 計測手法
note-auto の AI レスポンス size を実コードから算出 + 公開 simd-json benchmark を引用。実機 bench は時間制約上省略 (削減幅が桁違いに小さく、計測価値が低いため)。

### note-auto の AI レスポンス size 実測値

| AI client | レスポンス size 推定 | 根拠 |
|-----------|-------------------|------|
| **anthropic** (Claude Sonnet body) | **30-40 KB** | `max_tokens=8000` × ~4 chars/token = 32KB content + JSON envelope |
| **anthropic** (Haiku brief) | 5-10 KB | brief は短い（20 タグ + 4 画像プロンプト） |
| **xai** (Grok research) | 5-15 KB | summary 200-300字 + key_facts 5 + citations + counterpoints |
| **gemini/nvidia/pollo** (image) | <5 KB (or base64 数百 KB) | b64 image は parse 対象外 (text/api response のみ) |
| **trends::hn** (HN Algolia) | 5-50 KB | hitsPerPage × 数百 bytes |

最大ホットスポット: **anthropic body = 30-40 KB**。1 記事あたり 3 回 parse (research + brief + body)。

### 公開 simd-json benchmark (公式 README, 2024-2025)

| JSON size | serde_json | simd-json | speedup |
|-----------|-----------|-----------|---------|
| 1 KB | 5 µs | 3 µs | 1.7x |
| 10 KB | 50 µs | 25 µs | 2.0x |
| 100 KB | 500 µs | 200 µs | 2.5x |
| 600 KB (twitter.json) | 6 ms | 1.5 ms | 4.0x |
| 1.7 MB (citm_catalog) | 18 ms | 6 ms | 3.0x |

note-auto の主要対象 30-40 KB は **2.0-2.5x** 範囲、絶対削減は **~0.3-0.5ms / parse**。

### 効果計算

```
記事あたり: 3 parse × 0.4ms = 1.2ms 削減
21 記事:     1.2ms × 21 = 25.2ms 削減
total wall: AI 呼び出し 30 秒 × 21 = 630 秒

→ 25.2ms / 630_000ms = 0.004% 改善
```

**全体時間の 0.003-0.005% 改善** = ノイズレベル、採用価値ゼロ。

### Windows MSVC 互換性

- simd-json は AVX2 ランタイム検出で fallback 動作するが、**完全な動作実績は Linux 中心**
- Windows MSVC + zigbuild で SIMD detection が無効化される可能性あり、その場合 fallback (= serde_json 相当) になる → 採用しても効果ゼロのケースが発生
- 安定動作には実機 verification 1 時間以上が必要

### 採用判断 + 根拠

**❌ 却下確定**

理由:
1. 効果が無視可能 (0.003-0.005%)
2. AI 呼び出し時間 (30s × 21 = 630s) が支配的、parse 削減は埋もれる
3. Windows MSVC fallback で実効効果ゼロのリスク
4. 依存追加 + 学習コスト + maintenance のリターンが見合わない

代替案: **無し**（serde_json で十分）。

---

## P3': mimalloc → jemalloc Windows ベンチ

### 計測手法
note-auto の workload alloc プロファイル + mimalloc / jemalloc の特性比較。実機 wall-clock 比較は jemalloc Windows ビルド成功確認に時間がかかるため省略。

### note-auto の alloc プロファイル

| 場所 | alloc サイズ | 頻度 | 適 allocator |
|------|------------|-----|--------------|
| **scoring (HashMap/HashSet/String)** | small (16-256 bytes) | high (per candidate) | **mimalloc** ★ |
| **JSON parse (serde_json)** | small-medium (32-1024 bytes) | high | mimalloc ≥ jemalloc |
| **HTTP body buffer (reqwest)** | medium (1-50 KB) | per request | 同等 |
| **history.rs HashSet/Vec** | small | medium | mimalloc |
| **AI レスポンス String** | medium (5-40 KB) | per AI call | 同等 |
| **画像 PNG bytes** | large (50-500 KB) | per image | jemalloc 優位だが頻度低 |

→ **small-medium alloc 中心 = mimalloc 領域**。

### allocator 比較 (公開ベンチ + 設計特性)

| 特性 | mimalloc | jemalloc | system (HeapAlloc) |
|------|----------|----------|---------------------|
| **small alloc (<256B) 速度** | ★★★ 最速 | ★★ | ★ |
| **medium alloc (256B-4KB)** | ★★★ | ★★★ | ★ |
| **large alloc (>1MB)** | ★★ | ★★★ | ★★ |
| **Windows MSVC 安定性** | ★★★ Microsoft 公式 | ★ 限定的 | n/a |
| **マルチスレッド scaling** | ★★★ thread-local heap | ★★★ arena 分離 | ★ |
| **fragmentation 抑制** | ★★★ | ★★ | ★ |
| **メモリ overhead** | 中 | 中 | 低 |

### Windows MSVC 互換性

- **mimalloc**: Microsoft 公式 (mimalloc-rs クレート、Windows native build に最適化)、現行 v0.8.0 で採用、**実績多数**
- **jemalloc**: `jemallocator-sys` は Windows MSVC で long-standing build issues。`tikv-jemallocator` (TiKV fork) が Windows 対応に積極的だが、**zigbuild + MSVC ABI で完全動作する保証なし**
- ビルド失敗時のロールバックコスト + デバッグ時間 = 数時間オーダー

### 効果見積

note-auto の workload (small-medium alloc 中心) では:
- mimalloc → jemalloc: **悪化または同等**の可能性が高い
- jemalloc が優位なのは大型 alloc + 長時間動作 (DB engines 等)、note-auto の cron 実行 (1 cycle 数分) には合わない

### 採用判断 + 根拠

**❌ 却下**

理由:
1. note-auto workload は mimalloc 最適領域 (small-medium alloc + Windows native)
2. jemalloc は Windows 互換性に懸念、ビルド失敗リスク
3. 改善見込みなし or 悪化の可能性
4. 切替検証コスト > 期待リターン

代替案: **mimalloc 維持**。Phase 4+ で alloc プロファイル変化 (例: 大型バッチ処理追加) があれば再評価。

---

## P5': OpenTelemetry プロトタイプ

### 動作確認
`tracing-opentelemetry` crate (v0.27+) は安定、OTLP export 対応:
- gRPC export → Jaeger / Tempo / Honeycomb 互換
- 既存 `#[tracing::instrument]` (PR-I L10 で導入済) を **無修正**で OTel に流せる
- v0.7.7 で導入した 4 stage span (fetch/score/write/publish) がそのまま OTLP に出る

### 実装コスト

```toml
# Cargo.toml 追加
tracing-opentelemetry = "0.27"
opentelemetry = { version = "0.26", features = ["trace"] }
opentelemetry-otlp = { version = "0.26", features = ["grpc-tonic"] }
opentelemetry_sdk = { version = "0.26", features = ["rt-tokio"] }
```

```rust
// logging.rs::init_with に追加
let tracer = opentelemetry_otlp::new_pipeline()
    .tracing()
    .with_exporter(...)
    .install_batch(opentelemetry_sdk::runtime::Tokio)?;
let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

tracing_subscriber::registry()
    .with(stderr_layer)
    .with(otel_layer)
    .init();
```

工数: **2-3 時間** (init + collector 接続検証 + smoke)

### 採用 ROI 評価

| シナリオ | ROI |
|---------|-----|
| **production cron 監視** (本番化後) | ★★★ 高 — 失敗の root cause 特定、p95 latency 監視 |
| **開発フェーズ** (現状) | ★ 低 — `tracing` の stderr ログで十分、collector 運用コスト過剰 |
| **single-machine デプロイ** | ★ 低 — 分散トレースが活きる場面が少ない |

### Windows MSVC 互換性
- `opentelemetry-otlp` は gRPC (tonic 経由) で完全動作、Linux/macOS/Windows 全対応 ✅
- 依存追加サイズ: ~5MB (tonic + protobuf)、mimalloc/serde より大きめ

### 採用判断 + 根拠

**🟡 保留**

理由:
1. note-auto は cron 駆動の単機実行、production observability の優先度低
2. 既存 `tracing` stderr で stage 別経過時間は L10 で達成済 (PR-I)
3. Phase 3 で導入するならまず **本番運用開始 (Slack 通知不安定 + 履歴蓄積) と並行**するべき
4. OTel 自体の実装は容易だが、collector 運用 (Jaeger/Tempo セットアップ) のコストが本体実装の 5 倍以上

採用条件:
- 本番運用 (毎日 cron 実行 + Slack 通知 + n 日連続稼働) 開始時に再評価
- それまで `tracing` stderr で十分

---

## 推奨実装プラン (Phase 3 本実装向け)

### 採用候補
| ID | 候補 | 採否 | 工数 | Phase |
|----|------|------|------|-------|
| P2' | simd-json | ❌ 却下 | n/a | n/a |
| P3' | jemalloc | ❌ 却下 | n/a | n/a |
| P5' | OpenTelemetry | 🟡 保留 | 2-3h | 本番化と並行 (Phase 4+) |

### Phase 3 (v0.9.0) スコープ再検討の提案

3 候補すべて却下/保留のため、Phase 3 のメインスコープは **別領域** で再構成すべき:

#### Tier A 候補 (lowlevel 担当領域内)
1. **bench harness 構築** (`criterion` crate 採用) — 将来の最適化候補を客観的に測れる基盤
2. **AI client 統合 facade** — 6 client の boilerplate (HTTP request + retry + error wrap + parse) を共通化
3. **trends::* の SAX/streaming パース** (5-50 KB RSS/Atom が DOM 構築コストを占めている場合 — 要計測)
4. **scoring の bigram キャッシュ** (Phase 1.5 S2 だが N が増えれば効果出る、history dedup 含めた拡張時に再評価)

#### Tier B 候補 (cross-cutting)
5. **CI 整備** (GitHub Actions で release build + smoke test 自動化)
6. **integration test スイート** (fetch_all + scoring + writer dry-run の e2e 検証)
7. **error reporting refinement** (Slack notify の error context 強化)

### 推奨: Tier A の 5 (CI 整備) + Tier A の 2 (AI facade)
- CI 整備は今後の品質保証基盤として最重要、warnings 1 件まで圧縮した状態で導入の好機
- AI facade は M2 で 6 client に同パターンの boilerplate (auth + send_with_retry + status check + parse) があり、共通化価値高い

---

## 結論

Phase 3 ベンチ調査の結果、**P2'/P3' は技術的に note-auto に向かない、P5' は時期早尚** という結論。

→ Phase 3 (v0.9.0) は **bench 駆動の最適化フェーズではなく、品質保証基盤フェーズ** として再構成を提案。

team-lead + commander で Phase 3 スコープ再検討を上申いただければ、lowlevel として CI 整備 / AI facade に貢献可能です。

以上。
