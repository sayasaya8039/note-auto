//! AF1 (Phase 3 v0.9.0): AI クライアント統合 facade
//!
//! 6 AI client (anthropic / openai / xai / gemini / nvidia / pollo) で重複していた
//! HTTP request + retry + status check + JSON parse + error 整形のボイラープレートを
//! 共通化する。本ファイルは **trait 定義 + 共通 helper 関数** のみ提供し、
//! 既存 client への適用は AF2 で順次行う (AF1 では既存コード無修正)。
//!
//! # 設計
//!
//! ## `AiClient` trait
//! 各 client が実装する最小契約:
//! - `label()` — API 識別子 (logging / error message 用)
//! - `build_request(body)` — auth header + endpoint + body を持つ `RequestBuilder` 構築
//!
//! ## `send_json` helper (free function)
//! `AiClient` を引数に取り、以下を 1 関数に統合:
//! 1. `util::send_with_retry` で transient エラー指数バックオフ retry (M2 既存機構を活用)
//! 2. ステータスコード非 2xx → `{label} API {status}: {body}` 形式の error
//! 3. `resp.json::<R>().await` で deserialize
//!
//! ## AF2 での効果
//! 各 client の HTTP 呼び出し block (現状 12-15 行) が 1-2 行に圧縮される想定:
//! ```ignore
//! // 旧 (例: anthropic.rs::call)
//! let resp = util::send_with_retry(
//!     || self.http.post(ENDPOINT)
//!         .header("x-api-key", &self.api_key)
//!         .header("anthropic-version", API_VERSION)
//!         .json(&body)
//!         .send(),
//!     3, "anthropic",
//! ).await?;
//! if !resp.status().is_success() {
//!     let status = resp.status();
//!     let txt = resp.text().await.unwrap_or_default();
//!     return Err(anyhow!("Anthropic API {}: {}", status, txt));
//! }
//! let parsed: Resp = resp.json().await?;
//!
//! // 新
//! let parsed: Resp = client::send_json(self, &body).await?;
//! ```

use anyhow::{anyhow, Result};
use serde::de::DeserializeOwned;

/// AI client 共通 trait — endpoint + auth + body 構築のみを抽象化。
///
/// `Send + Sync` 制約は `send_json` 内で `tokio` の async closure 経由で
/// retry を回すために必要。各 impl は `&self` でステートレスに reqwest::Client を
/// clone or borrow できる前提。
pub trait AiClient: Send + Sync {
    /// API 識別子。`tracing` ログと error message に使う。
    /// 例: `"anthropic"`, `"xai_grok"`, `"openai_image"`
    fn label(&self) -> &'static str;

    /// 認証 header + endpoint + body を持つ `RequestBuilder` を構築する。
    /// `send_json` から retry のたびに呼ばれるため、毎回新しい builder を返すこと。
    fn build_request(&self, body: &serde_json::Value) -> reqwest::RequestBuilder;
}

/// `AiClient` を介して JSON POST を送信、retry / status check / parse を統合実行する。
///
/// # 流れ
/// 1. `client.build_request(body)` で毎試行 builder 構築
/// 2. `util::send_with_retry(factory, 3, label)` で transient (408/429/5xx) + connect/timeout を retry
/// 3. ステータス非 2xx → `{label} API {status}: {body}` の anyhow::Error
/// 4. `resp.json::<R>()` で deserialize
///
/// # ジェネリック型
/// - `C`: `AiClient` 実装 (典型: AnthropicClient, GrokClient 等)
/// - `R`: deserialize 先 (典型: 各 client 内の `Resp` 構造体)
#[allow(dead_code)] // AF2 で 6 client の HTTP 呼び出し統合時に活用、現状未使用
pub async fn send_json<C, R>(client: &C, body: &serde_json::Value) -> Result<R>
where
    C: AiClient,
    R: DeserializeOwned,
{
    let label = client.label();
    let resp = crate::util::send_with_retry(
        || client.build_request(body).send(),
        3,
        label,
    )
    .await
    .map_err(|e| anyhow!("{} send failed: {}", label, e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let txt = resp.text().await.unwrap_or_default();
        return Err(anyhow!("{} API {}: {}", label, status, txt));
    }

    let parsed: R = resp
        .json()
        .await
        .map_err(|e| anyhow!("{} response JSON parse failed: {}", label, e))?;
    Ok(parsed)
}

/// `AiClient` を介して JSON POST 後、レスポンスを **生テキスト**で受ける variant。
///
/// 用途: AI からの応答が JSON ではなく markdown / プレーンテキストの場合
/// (例: anthropic の content blocks を独自に処理する場合)。
/// retry + status check は同一、parse 段階のみ scratch。
#[allow(dead_code)] // AF2 で活用予定、現状未使用
pub async fn send_text<C>(client: &C, body: &serde_json::Value) -> Result<String>
where
    C: AiClient,
{
    let label = client.label();
    let resp = crate::util::send_with_retry(
        || client.build_request(body).send(),
        3,
        label,
    )
    .await
    .map_err(|e| anyhow!("{} send failed: {}", label, e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let txt = resp.text().await.unwrap_or_default();
        return Err(anyhow!("{} API {}: {}", label, status, txt));
    }

    let text = resp
        .text()
        .await
        .map_err(|e| anyhow!("{} response text failed: {}", label, e))?;
    Ok(text)
}
