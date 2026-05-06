//! 共通ユーティリティ

use std::collections::HashSet;
use std::sync::LazyLock;
use unicode_segmentation::UnicodeSegmentation;

/// JSON レスポンスからコードフェンスを剥がす
pub fn strip_code_fence(s: &str) -> String {
    let t = s.trim();
    for prefix in ["```json", "```JSON", "```"] {
        if let Some(rest) = t.strip_prefix(prefix) {
            return rest.trim_end_matches("```").trim().to_string();
        }
    }
    // オブジェクト形式 `{...}` を優先的に抽出 (Grok が前置きを含めて返してくる場合があるため)。
    // 配列だけ抜き出してしまうと `{ "key_facts": [...], "citations": [...] }` のような構造が壊れる。
    if let (Some(l), Some(r)) = (t.find('{'), t.rfind('}')) {
        if l < r {
            return t[l..=r].to_string();
        }
    }
    // 文中に JSON 配列が混じっている場合に最初の `[` から最後の `]` を抽出
    if let (Some(l), Some(r)) = (t.find('['), t.rfind(']')) {
        if l < r {
            return t[l..=r].to_string();
        }
    }
    t.to_string()
}

static RE_HTML: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"<[^>]+>").unwrap());

/// HTML タグを除去してプレーンテキストにする
pub fn strip_html(s: &str) -> String {
    let stripped = RE_HTML.replace_all(s, " ");
    html_escape::decode_html_entities(&stripped)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Jaccard 類似度 (grapheme bigram ベース)
pub fn title_similarity(a: &str, b: &str) -> f64 {
    let ga = bigrams(a);
    let gb = bigrams(b);
    if ga.is_empty() || gb.is_empty() {
        return 0.0;
    }
    let inter = ga.intersection(&gb).count() as f64;
    let union = ga.union(&gb).count() as f64;
    inter / union
}

fn bigrams(s: &str) -> HashSet<String> {
    let lower = s.to_lowercase();
    let gs: Vec<&str> = lower.graphemes(true).collect();
    if gs.len() < 2 {
        return std::iter::once(lower).collect();
    }
    gs.windows(2).map(|w| w.concat()).collect()
}

/// 共通 HTTP クライアント (プロセス全体で 1 インスタンス共有)
///
/// `OnceLock` でプロセス起動時に 1 度だけビルドし、以降は `clone()` で参照を返す。
/// reqwest::Client の clone は内部 Arc なので低コスト。
/// 失敗時 (TLS 初期化エラー等) は `expect` で fail-fast。プロセス起動直後しか呼ばれない。
static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent("note-auto/0.1 (+https://note.com)")
        .timeout(std::time::Duration::from_secs(30))
        .gzip(true)
        .build()
        .expect("failed to build shared http client")
});

static HTTP_CLIENT_LONG: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent("note-auto/0.2")
        .timeout(std::time::Duration::from_secs(180))
        .gzip(true)
        .build()
        .expect("failed to build shared http_client_long")
});

/// 共通 HTTP クライアント (Result 返却 — 旧 API 互換)
///
/// 内部は `OnceLock` 経由で初回のみビルド、以降は clone を返す。
/// 各呼び出し側のコード変更は不要。
pub fn http_client() -> anyhow::Result<reqwest::Client> {
    Ok(HTTP_CLIENT.clone())
}

/// AI 用 HTTP クライアント (タイムアウト長め、Result 返却 — 旧 API 互換)
pub fn http_client_long() -> anyhow::Result<reqwest::Client> {
    Ok(HTTP_CLIENT_LONG.clone())
}

/// transient HTTP エラー (408/425/429/500/502/503/504) と connect/timeout エラーに対し
/// 指数バックオフ + jitter でリトライする汎用ヘルパ (M2)。
///
/// - `factory`: `RequestBuilder::send()` 相当を返す Fn（毎試行で新規作成）
/// - `max_retries`: 失敗時の追加試行回数 (全試行 = max_retries + 1)
/// - 永続エラー (4xx の他、5xx の中で transient 以外) は即座に Response を返す
///   → caller 側が `status().is_success()` で判定して err 化する流れを踏襲
/// - 全リトライ失敗時は最後の Response (or Err) を返す
///
/// バックオフ: 500ms × 2^attempt + jitter (0..base/4)、最大 30s
///
/// Anthropic / Grok / Gemini / Nvidia / Pollo / OpenAI 全 AI client から呼ぶ想定。
pub async fn send_with_retry<F, Fut>(
    factory: F,
    max_retries: usize,
    label: &str,
) -> std::result::Result<reqwest::Response, reqwest::Error>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = std::result::Result<reqwest::Response, reqwest::Error>>,
{
    let mut attempt: usize = 0;
    loop {
        let result = factory().await;
        match result {
            Ok(resp) => {
                let code = resp.status().as_u16();
                let transient = matches!(code, 408 | 425 | 429 | 500 | 502 | 503 | 504);
                if transient && attempt < max_retries {
                    let delay = backoff_ms(attempt);
                    tracing::warn!(
                        label, attempt = attempt + 1, max = max_retries + 1,
                        status = code, delay_ms = delay,
                        "transient HTTP error, retrying"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    attempt += 1;
                    continue;
                }
                return Ok(resp);
            }
            Err(e) => {
                let recoverable = e.is_connect() || e.is_timeout();
                if recoverable && attempt < max_retries {
                    let delay = backoff_ms(attempt);
                    tracing::warn!(
                        label, attempt = attempt + 1, max = max_retries + 1,
                        error = %e, delay_ms = delay,
                        "connect/timeout error, retrying"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    attempt += 1;
                    continue;
                }
                return Err(e);
            }
        }
    }
}

/// HTTP GET でバイナリを取得し、`Vec<u8>` を返す。M2 (image silent loss 防止)。
///
/// `send_with_retry` で transient + connect/timeout を 3 回まで retry、
/// 非 2xx status は err 化、bytes() 取得失敗もエラー化。
///
/// 用途:
/// - `pollo.rs` の最終 PNG download (poll 完了後の videoUrl fetch)
/// - `openai.rs` の URL response (b64_json でなく url 形式の場合)
/// - `nvidia.rs::fetch_url` (data フィールドが url の場合)
///
/// quality M2 報告: 旧実装は `?` 連鎖で network blip により bytes 取得失敗 → 全 article fail。
/// 本 helper は retry + 詳細 error message で resilience 強化。
pub async fn fetch_bytes_with_retry(
    http: &reqwest::Client,
    url: &str,
    label: &str,
) -> anyhow::Result<Vec<u8>> {
    let resp = send_with_retry(
        || http.get(url).send(),
        3,
        label,
    )
    .await
    .map_err(|e| anyhow::anyhow!("{} fetch send failed: {}", label, e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        return Err(anyhow::anyhow!("{} fetch HTTP {}", label, status));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| anyhow::anyhow!("{} fetch bytes failed: {}", label, e))?;
    Ok(bytes.to_vec())
}

/// M3-B (Fix B 熟成): hostname を DNS resolve し、得られた全 IP に対して
/// loopback / private / link-local をチェックする。1 つでも内部 IP が
/// 含まれていれば `false` を返す (**DNS rebinding 対策の第一歩**)。
///
/// IPv4 + IPv6 両対応:
/// - IPv4: loopback / unspecified / private (10/8, 172.16/12, 192.168/16) / link-local (169.254/16) を block
/// - IPv6: loopback (::1) / unspecified (::) / ULA (fc00::/7) / link-local (fe80::/10) を block
///
/// DNS 解決失敗 (host が unreachable) → false
/// 空 iterator (resolve したが結果ゼロ) → false
///
/// ## TOCTOU 注意
/// 本実装は「DNS 解決時点の IP が safe か」のみ保証する。
/// 真の TOCTOU 対策 (resolve 結果を request 時に固定する) には
/// `reqwest::Client::resolve` override が必要で、v0.9.2+ で熟成予定。
pub async fn resolve_and_check_safe_ip(host: &str) -> bool {
    use std::net::IpAddr;
    use tokio::net::lookup_host;

    // tokio::net::lookup_host は "host:port" 形式を要求
    let target = format!("{host}:443");
    let addrs = match lookup_host(&target).await {
        Ok(it) => it,
        Err(_) => return false,
    };

    let mut any = false;
    for addr in addrs {
        any = true;
        let ip = addr.ip();
        if ip.is_loopback() || ip.is_unspecified() {
            return false;
        }
        match ip {
            IpAddr::V4(v4) => {
                if v4.is_private() || v4.is_link_local() {
                    return false;
                }
            }
            IpAddr::V6(v6) => {
                let seg = v6.segments();
                // ULA (fc00::/7): 最上位 7 bit が 1111110
                if (seg[0] & 0xfe00) == 0xfc00 {
                    return false;
                }
                // link-local (fe80::/10): 最上位 10 bit が 1111111010
                if (seg[0] & 0xffc0) == 0xfe80 {
                    return false;
                }
            }
        }
    }

    any
}

/// 指数バックオフ + jitter (1/4 of base) ms。最大 30s で頭打ち。
fn backoff_ms(attempt: usize) -> u64 {
    use rand::Rng;
    let base = 500u64.saturating_mul(1u64 << attempt.min(6));
    let base = base.min(30_000);
    let jitter_max = (base / 4).max(1);
    let jitter = rand::thread_rng().gen_range(0..jitter_max);
    base.saturating_add(jitter)
}

/// YAML 値のエスケープ (インジェクション防止)
pub fn yaml_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "")
}
