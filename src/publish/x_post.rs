//! X (Twitter) API v2 で告知ツイートを投稿 (OAuth 1.0a user context)

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use hmac::{Hmac, Mac};
use rand::Rng;
use serde_json::json;
use sha1::Sha1;

use crate::config::Config;
use crate::writer::WrittenArticle;

type HmacSha1 = Hmac<Sha1>;

const ENDPOINT: &str = "https://api.x.com/2/tweets";

pub async fn announce(
    cfg: &Config,
    article: &WrittenArticle,
    note_url: Option<&str>,
) -> Result<Option<String>> {
    if !cfg.publish.x_announce {
        return Ok(None);
    }
    if cfg.publish.dry_run {
        tracing::info!(slug = %article.slug, "[dry-run] X告知スキップ");
        return Ok(None);
    }

    let (key, secret, token, token_secret) = match (
        cfg.publish.x_api_key.as_deref(),
        cfg.publish.x_api_secret.as_deref(),
        cfg.publish.x_access_token.as_deref(),
        cfg.publish.x_access_secret.as_deref(),
    ) {
        (Some(k), Some(s), Some(t), Some(ts)) => (k, s, t, ts),
        _ => {
            tracing::info!("X OAuth 認証情報未設定 — X告知スキップ");
            return Ok(None);
        }
    };

    let text = build_tweet_text(article, note_url);
    let body = json!({ "text": text });

    let auth = build_oauth1_header(
        "POST", ENDPOINT, key, secret, token, token_secret, &[],
    )?;

    let client = crate::util::http_client()?;

    // M1 (quality): X API は 429 (rate limit) や 502/503 で permanent fail していた。
    //               util::send_with_retry で transient HTTP + connect/timeout を retry。
    //               OAuth 1.0a の nonce/timestamp は build_oauth1_header で生成済、
    //               短時間 retry (~3.5s 以内) なら nonce 重複問題は実用上発生しない。
    let resp = crate::util::send_with_retry(
        || client
            .post(ENDPOINT)
            .header("Authorization", auth.as_str())
            .header("Content-Type", "application/json")
            .json(&body)
            .send(),
        3,
        "x_api",
    ).await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let txt = resp.text().await.unwrap_or_default();
        return Err(anyhow!("X API {}: {}", status, txt));
    }

    #[derive(serde::Deserialize)]
    struct Resp { data: Data }
    #[derive(serde::Deserialize)]
    struct Data { id: String }

    let parsed: Resp = resp.json().await?;
    Ok(Some(format!("https://x.com/i/web/status/{}", parsed.data.id)))
}

/// 任意テキストを tweet 投稿 (OAuth 疎通確認・CLI テスト用)
pub async fn post_text(cfg: &Config, text: &str) -> Result<String> {
    let (key, secret, token, token_secret) = match (
        cfg.publish.x_api_key.as_deref(),
        cfg.publish.x_api_secret.as_deref(),
        cfg.publish.x_access_token.as_deref(),
        cfg.publish.x_access_secret.as_deref(),
    ) {
        (Some(k), Some(s), Some(t), Some(ts)) => (k, s, t, ts),
        _ => return Err(anyhow!("X OAuth 認証情報が .env に揃っていません")),
    };

    let body = json!({ "text": text });
    let auth = build_oauth1_header("POST", ENDPOINT, key, secret, token, token_secret, &[])?;
    let client = crate::util::http_client()?;

    // M1 (quality): X API 429 / 5xx を transient retry で吸収。
    //               詳細は post_announce 側コメント参照。
    let resp = crate::util::send_with_retry(
        || client
            .post(ENDPOINT)
            .header("Authorization", auth.as_str())
            .header("Content-Type", "application/json")
            .json(&body)
            .send(),
        3,
        "x_api",
    ).await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let txt = resp.text().await.unwrap_or_default();
        return Err(anyhow!("X API {}: {}", status, txt));
    }

    #[derive(serde::Deserialize)]
    struct Resp { data: Data }
    #[derive(serde::Deserialize)]
    struct Data { id: String }

    let parsed: Resp = resp.json().await?;
    Ok(format!("https://x.com/i/web/status/{}", parsed.data.id))
}

fn build_tweet_text(article: &WrittenArticle, note_url: Option<&str>) -> String {
    // X は 280 文字 (日本語は大半が2weight扱いなので実質100-140文字目安)
    let tags_line = article.tags.iter()
        .take(3)
        .map(|t| format!("#{}", t.replace(' ', "")))
        .collect::<Vec<_>>()
        .join(" ");

    let base_url = note_url.unwrap_or("");
    let title = truncate_chars(&article.title, 80);

    if base_url.is_empty() {
        format!("📝 新記事公開\n\n{title}\n\n{tags_line}")
    } else {
        format!("📝 新記事公開\n\n{title}\n\n{base_url}\n\n{tags_line}")
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max { s.to_string() } else {
        let mut t: String = chars[..max].iter().collect();
        t.push('…');
        t
    }
}

/// OAuth1.0a signed Authorization header
fn build_oauth1_header(
    method: &str,
    url: &str,
    consumer_key: &str,
    consumer_secret: &str,
    access_token: &str,
    access_secret: &str,
    extra_params: &[(&str, &str)],
) -> Result<String> {
    let nonce: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();
    let timestamp = chrono::Utc::now().timestamp().to_string();

    let mut params: Vec<(String, String)> = vec![
        ("oauth_consumer_key".into(), consumer_key.into()),
        ("oauth_nonce".into(), nonce.clone()),
        ("oauth_signature_method".into(), "HMAC-SHA1".into()),
        ("oauth_timestamp".into(), timestamp.clone()),
        ("oauth_token".into(), access_token.into()),
        ("oauth_version".into(), "1.0".into()),
    ];
    for (k, v) in extra_params {
        params.push((k.to_string(), v.to_string()));
    }

    // parameter string (percent-encoded, sorted)
    let mut encoded: Vec<(String, String)> = params.iter()
        .map(|(k, v)| (pe(k), pe(v)))
        .collect();
    encoded.sort();
    let param_string = encoded.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");

    // signature base string
    let base_string = format!("{}&{}&{}", method, pe(url), pe(&param_string));
    let signing_key = format!("{}&{}", pe(consumer_secret), pe(access_secret));

    let mut mac = HmacSha1::new_from_slice(signing_key.as_bytes())
        .map_err(|e| anyhow!("hmac key error: {e}"))?;
    mac.update(base_string.as_bytes());
    let sig = STANDARD.encode(mac.finalize().into_bytes());

    // Authorization header
    let mut header_parts: Vec<(String, String)> = vec![
        ("oauth_consumer_key".into(), consumer_key.into()),
        ("oauth_nonce".into(), nonce),
        ("oauth_signature".into(), sig),
        ("oauth_signature_method".into(), "HMAC-SHA1".into()),
        ("oauth_timestamp".into(), timestamp),
        ("oauth_token".into(), access_token.into()),
        ("oauth_version".into(), "1.0".into()),
    ];
    header_parts.sort();

    let mut header = String::from("OAuth ");
    header.push_str(&header_parts.iter()
        .map(|(k, v)| format!("{}=\"{}\"", pe(k), pe(v)))
        .collect::<Vec<_>>()
        .join(", "));
    Ok(header)
}

/// OAuth percent-encoding (RFC 3986 unreserved: A-Z a-z 0-9 - . _ ~)
fn pe(s: &str) -> String {
    const UNRESERVED: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.as_bytes() {
        if UNRESERVED.contains(b) {
            out.push(*b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}
