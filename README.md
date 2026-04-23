# note-auto

note記事のトレンドドリブン自動生成システム。

## Phase 1 (現状): トレンド収集 + スコアリング

4ソースを並行取得してスコアリング・重複排除し、上位 N 件を JSON 出力。

### ソース

| ソース | 状態 | API キー |
|--------|------|---------|
| X (Grok x_search) | ✅ `XAI_API_KEY` 設定時のみ有効 | `XAI_API_KEY` 環境変数 |
| Google Trends (JP) | ✅ 有効 | 不要 |
| note RSS (trending + hashtag × 3) | ✅ 有効 | 不要 |
| Hacker News (Algolia) | ✅ 有効 | 不要 |
| Reddit | ⏸ 無効 (Cloudflare TLS fingerprinting) | OAuth対応はPhase 2 |

### 使い方

```bash
# 設定
cp config.toml config.local.toml
# 必要なら XAI_API_KEY を環境変数に設定

# トレンド取得
cargo run --release -- fetch-trends --top 3

# 出力: drafts/YYYY-MM-DD/trends.json
```

### スコアリング

1. ソース内で raw_score を 0-1 正規化
2. ソース別ウェイト (x=1.2, note=1.1, google=1.0, hn=0.9, reddit=0.8) 適用
3. タイトル bigram Jaccard 類似度 ≥0.65 を重複として除去
4. composite_score 降順で上位 N 件

### 出力 JSON スキーマ

```json
[
  {
    "source": "google",
    "title": "...",
    "summary": "...",
    "url": "...",
    "raw_score": 10000.0,
    "metrics": { "approx_traffic": "10000+" },
    "fetched_at": "2026-04-23T00:15:26Z",
    "normalized_score": 1.0,
    "composite_score": 1.0
  }
]
```

## ロードマップ

- **Phase 1** ✅ トレンド収集 + スコアリング
- **Phase 2** ⏳ AI執筆パイプライン (Opus 4.7 / Haiku 4.5 / Grok / gpt-image-2.0)
- **Phase 3** ⏳ note 自動投稿 (Playwright) + X 告知 + Slack Webhook + 07:00 JST cron 常駐
