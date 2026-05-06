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

/// M3-C (v0.9.2 TOCTOU 完全対策 + allowlist): URL を SSRF 防御チェックし、
/// safe と判定された場合は **resolve pin した secure reqwest::Client** を返す。
///
/// ## 防御層 (defense-in-depth 完成版)
/// - L1 (M3 Fix A): scheme==https + IPv4 リテラル private/loopback/link-local block
/// - L2 (M3-B): DNS resolve 後の全 IP に対する safety check
/// - L3 (M3-C): allowlist 適用 (空時は M3-B 動作 fallback)
/// - L4 (M3-C): resolve pin で TOCTOU race 排除 — `ClientBuilder::resolve(host, addr)` で固定
/// - L5 (M3-C): cross-domain redirect block — `redirect::Policy::custom` で同 hostname のみ follow
///
/// ## 戻り値
/// - `Some(client)`: URL は safe、返された client で fetch すれば TOCTOU race なし
/// - `None`: URL は unsafe (scheme / allowlist / IP / resolve いずれかで deny)
pub async fn check_and_pin_image_client(
    url_str: &str,
    allowlist: &[String],
) -> Option<reqwest::Client> {
    use std::net::{IpAddr, SocketAddr};
    use tokio::net::lookup_host;

    let parsed = url::Url::parse(url_str).ok()?;
    if parsed.scheme() != "https" {
        return None;
    }
    let host = parsed.host_str()?.to_string();
    let port = parsed.port_or_known_default().unwrap_or(443);

    // L3: allowlist が指定されていれば match 必須 (空なら skip)
    if !allowlist.is_empty() && !matches_allowlist(&host, allowlist) {
        return None;
    }

    // L1+L2: IP リテラル or DNS resolve + IP safety check + 1 IP を pin 用に決定
    let pinned_ip: IpAddr = if let Ok(ip) = host.parse::<IpAddr>() {
        if is_unsafe_ip(ip) {
            return None;
        }
        ip
    } else {
        let target = format!("{host}:{port}");
        let addrs = lookup_host(&target).await.ok()?;
        let mut chosen: Option<IpAddr> = None;
        for addr in addrs {
            let ip = addr.ip();
            if is_unsafe_ip(ip) {
                return None; // 1 つでも unsafe なら全否定 (DNS rebinding 抑止)
            }
            if chosen.is_none() {
                chosen = Some(ip);
            }
        }
        chosen?
    };

    // L4 + L5: resolve pin した secure client を build
    let socket = SocketAddr::new(pinned_ip, port);
    let host_for_redirect = host.clone();
    reqwest::Client::builder()
        .user_agent("note-auto/0.9 (secure-image)")
        .timeout(std::time::Duration::from_secs(30))
        .gzip(true)
        .resolve(&host, socket)
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            // L5: cross-domain redirect block。同 host のみ follow を許可。
            // (resolve pin は構築時の host のみに効くため、cross-domain redirect は
            //  別 host = 別 IP となり TOCTOU race の温床になる)
            let next_host = attempt.url().host_str().unwrap_or("");
            if next_host == host_for_redirect {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .ok()
}

/// 内部 IP (loopback / unspecified / private / link-local / ULA) 判定 (M3-B/M3-C 共通)。
fn is_unsafe_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    if ip.is_loopback() || ip.is_unspecified() {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            let seg = v6.segments();
            // ULA (fc00::/7)
            if (seg[0] & 0xfe00) == 0xfc00 {
                return true;
            }
            // link-local (fe80::/10)
            if (seg[0] & 0xffc0) == 0xfe80 {
                return true;
            }
            false
        }
    }
}

/// host が allowlist のいずれかにマッチするか判定 (M3-C)。
/// - exact match (`"example.com"` ↔ `host == "example.com"`)
/// - wildcard suffix (`"*.cdn.example.com"` ↔ `host` が `.cdn.example.com` で終わる、
///   または `cdn.example.com` 完全一致でも true)
fn matches_allowlist(host: &str, allowlist: &[String]) -> bool {
    for pattern in allowlist {
        if let Some(suffix) = pattern.strip_prefix("*.") {
            let needle = format!(".{suffix}");
            if host.ends_with(&needle) || host == suffix {
                return true;
            }
        } else if host == pattern.as_str() {
            return true;
        }
    }
    false
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_exact_match() {
        let list = vec!["konbini.com".to_string()];
        assert!(matches_allowlist("konbini.com", &list));
        assert!(!matches_allowlist("evil.com", &list));
        assert!(!matches_allowlist("sub.konbini.com", &list));
    }

    #[test]
    fn allowlist_wildcard_match() {
        let list = vec!["*.cdn.example.com".to_string()];
        assert!(matches_allowlist("img.cdn.example.com", &list));
        assert!(matches_allowlist("a.b.cdn.example.com", &list));
        assert!(matches_allowlist("cdn.example.com", &list));
        assert!(!matches_allowlist("cdn.example.org", &list));
        assert!(!matches_allowlist("evil-cdn.example.com", &list));
    }

    #[test]
    fn allowlist_empty_returns_false() {
        let list: Vec<String> = vec![];
        assert!(!matches_allowlist("anything.com", &list));
    }

    #[test]
    fn unsafe_ip_v4() {
        use std::net::{IpAddr, Ipv4Addr};
        assert!(is_unsafe_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
        assert!(is_unsafe_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(is_unsafe_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
        assert!(is_unsafe_ip(IpAddr::V4(Ipv4Addr::new(169, 254, 0, 1))));
        assert!(!is_unsafe_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
    }

    #[test]
    fn unsafe_ip_v6() {
        use std::net::{IpAddr, Ipv6Addr};
        assert!(is_unsafe_ip(IpAddr::V6(Ipv6Addr::new(
            0xfc00, 0, 0, 0, 0, 0, 0, 1
        ))));
        assert!(is_unsafe_ip(IpAddr::V6(Ipv6Addr::new(
            0xfe80, 0, 0, 0, 0, 0, 0, 1
        ))));
        assert!(!is_unsafe_ip(IpAddr::V6(Ipv6Addr::new(
            0x2606, 0x4700, 0, 0, 0, 0, 0, 1
        ))));
    }
}
