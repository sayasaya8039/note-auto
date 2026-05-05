# Phase 2 quality 調査レポート

> 調査対象: v0.7.7 (aa4de61) 全ソース
> 調査者: quality (silent worker)
> 日付: 2026-05-06

---

## CRITICAL (0件)

CRITICAL バグなし。

---

## HIGH (2件)

### H1: src/ai/mod.rs:58 — Q2 で見逃した expect() (panic=abort 不安全)

現状コード:

    pub fn http_client() -> reqwest::Client {
        crate::util::http_client_long().expect("reqwest client")  // expect 残存
    }

Q2 スコープ (publish/slack.rs, publish/x_post.rs, writer/mod.rs) は修正済みだが
ai/mod.rs の http_client() が漏れた。LazyLock 初期化失敗時に panic=abort でプロセス即死。

修正案 A (infallible fallback):

    pub fn http_client() -> reqwest::Client {
        crate::util::http_client_long()
            .unwrap_or_else(|_| reqwest::Client::new())
    }

修正案 B: write_one 冒頭を Result 化:

    let http = crate::util::http_client_long()?;

---

### H2: writer::run() — join_all で全記事を完全並列化 → API レート制限リスク

現状コード:

    let tasks = trends.iter().enumerate().map(|(i, t)| { ... });
    let results = join_all(tasks).await;  // 全 N 記事を同時実行

3 記事なら Anthropic に同時 6+ req 発火。publish_all は Q4 で buffered(2) 修正済みだが
writer 側は未対応。HTTP 429 発生時に全記事 fail のリスク。

修正案:

    let results: Vec<_> = stream::iter(owned)
        .map(|(i, trend)| { ... })
        .buffered(2)   // Anthropic rate limit 対策
        .collect()
        .await;

---

## MEDIUM (3件)

### M1: src/daemon.rs:95,161 — History::load() を 1 サイクルで 2 回呼ぶ

L95: dedup 用、L161: append 用と 2 回ファイル読み込み。
2 回目 load 時に history.json が変化すると dedup と追記先が乖離する。
修正: 1 度 load した mut history を dedup + append 両方に使う。

---

### M2: AI クライアント全般 — retry / backoff 未実装

anthropic / xai / gemini / nvidia / openai / pollo の全クライアントに
transient エラー時の retry がない。HTTP 429/502/503 で記事即 fail。
特に Grok research 失敗で記事全体が失われる。

修正案: util.rs に指数バックオフヘルパーを追加:

    pub async fn retry_with_backoff(max: u32, f: impl Fn() -> impl Future) -> Result {
        let mut wait = Duration::from_secs(2);
        for i in 0..max {
            match f().await {
                Ok(v) => return Ok(v),
                Err(e) if i + 1 < max => {
                    tokio::time::sleep(wait).await;
                    wait = wait.saturating_mul(2).min(Duration::from_secs(60));
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }

---

### M3: src/ai/xai.rs:63 — Grok citations 常に空の懸念

コメント: search_parameters は 2026-04 deprecated。citations は空で返る可能性あり。
citations 空 → 記事末尾「参考リンク」がゼロ → SEO/信頼性低下。

修正案 (暫定): citations が空なら trend.item.url を補完:

    if result.citations.is_empty() {
        if let Some(url) = &trend.item.url {
            result.citations.push(url.clone());
        }
    }

---

## v0.7.7 残 warnings 精査

| Warning | 判定 | 推奨アクション |
|---------|------|--------------|
| publish/mod.rs:16 unused import slack::post_progress | 未配線 — daemon.rs が slack モジュールを直接使い re-export をバイパス | daemon.rs の use から slack を除き post_progress を追加 |
| ai/mod.rs:52 field prompt is never read | Dead field (デバッグ用途) | embed_images 内で tracing::debug! ログに活用か _prompt にリネーム |
| ai/openai.rs:31 method generate is never used | Dead code — 削除 | 全 provider が generate_prompt(str) に統一、高レベル API は不要 |
| ai/openai.rs:82 function build_prompt is never used | Dead code — 削除 | 同上。ArticleBrief import も除去 |

Warning 1 修正例 (daemon.rs):

    // Before: use crate::publish::{..., slack, ...};
    //         slack::post_progress(cfg, ...).await;
    // After:  use crate::publish::{..., post_progress, ...};
    //         post_progress(cfg, ...).await;

Warning 3+4 修正例 (openai.rs):

    // Before: use super::{ArticleBrief, ImageAsset};
    //         pub async fn generate(&self, brief: &ArticleBrief) -> Result<ImageAsset> { ... }
    //         fn build_prompt(brief: &ArticleBrief) -> String { ... }
    // After:  use super::ImageAsset;
    //         // generate() と build_prompt() を完全削除

---

## 改善候補 TOP 5

| 優先 | 対象 | 改善 | 期待効果 |
|------|------|------|---------|
| 1 | ai/mod.rs | expect() 除去 (Q2 miss) | panic=abort 完全排除 |
| 2 | writer::run() | join_all → buffered(2) | Anthropic rate limit 回避 |
| 3 | AI クライアント全般 | retry_with_backoff(3) 追加 | 429/503 transient 耐性 |
| 4 | daemon.rs | History 1 回 load で dedup+append 共有 | ファイル IO 半減・一貫性 |
| 5 | ai/openai.rs | Dead code 削除 + Warning 1 配線修正 | 警告ゼロ |

---

## セキュリティ概要

| 項目 | リスク | 状況 |
|------|--------|------|
| API キーハードコード | NONE | 全キーを環境変数/dotenvy から取得 |
| API キーのログ漏れ | LOW | Bearer ヘッダはリクエスト本体に含まれない |
| playwright stdout がエラー文字列に埋め込まれる | LOW | ローカルログのみ、外部漏洩なし |
| SSRF (download_source_images) | LOW | starts_with(http) のみ。ローカル実行のため実害なし |
| config.toml injection | NONE | Command::new + args 経由 (shell 非介入) |

---

## パニックカウント (v0.7.7 全ソース)

| ファイル | expect | 備考 |
|---------|--------|------|
| util.rs | 2 | LazyLock init — infallible |
| ai/mod.rs | 1 | H1: http_client() — 要修正 |
| その他全ファイル | 0 | Q2 修正済み |
| 合計 | 3 | 実問題は ai/mod.rs の 1 件 |

---

## 総合サマリ

| Severity | Count | Status |
|----------|-------|--------|
| CRITICAL | 0 | pass |
| HIGH | 2 | warn |
| MEDIUM | 3 | info |
| LOW/警告 | 4 | note |

Verdict: v0.8.0 着手前に H1 (ai/mod.rs expect 除去) 優先対処推奨。
H2 (writer buffered) は並行して着手可。WARNING 3+4 dead code 削除はビルド警告ゼロのため Phase 2 序盤で。
