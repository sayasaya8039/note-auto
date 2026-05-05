//! NVIDIA Build API: black-forest-labs/flux.2-klein-4b
//!
//! 軽量・高速な text-to-image。本文挿入用 (low quality) 画像に使用する。
//! エンドポイント: https://ai.api.nvidia.com/v1/genai/black-forest-labs/flux.2-klein-4b
//!
//! リクエスト: { prompt, width, height, seed, steps }
//! レスポンス: base64 PNG を含む JSON (`image` / `artifacts[].base64` / `images[]` 等に対応)

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use serde_json::{json, Value};

use super::client::AiClient;
use super::ImageAsset;

const ENDPOINT: &str = "https://ai.api.nvidia.com/v1/genai/black-forest-labs/flux.2-klein-4b";

pub struct NvidiaFluxClient<'a> {
    http: &'a reqwest::Client,
    api_key: String,
    width: u32,
    height: u32,
    steps: u32,
}

impl AiClient for NvidiaFluxClient<'_> {
    fn label(&self) -> &'static str {
        "nvidia_flux"
    }

    fn build_request(&self, body: &serde_json::Value) -> reqwest::RequestBuilder {
        self.http
            .post(ENDPOINT)
            .bearer_auth(&self.api_key)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(body)
    }
}

impl<'a> NvidiaFluxClient<'a> {
    pub fn new(http: &'a reqwest::Client, api_key: &str) -> Self {
        Self {
            http,
            api_key: api_key.to_string(),
            width: 1024,
            height: 1024,
            steps: 4,
        }
    }

    #[allow(dead_code)]
    pub fn with_size(mut self, w: u32, h: u32) -> Self {
        self.width = w;
        self.height = h;
        self
    }

    pub async fn generate_prompt(&self, prompt: &str) -> Result<ImageAsset> {
        let body = json!({
            "prompt": prompt,
            "width": self.width,
            "height": self.height,
            "seed": 0,
            "steps": self.steps,
        });

        // AF2: AiClient + client::send_json 経由で M2 retry + status check + parse 統合
        let v: Value = super::client::send_json(self, &body).await
            .context("NVIDIA flux.2 request failed")?;
        let bytes = extract_png_bytes(&v, self.http).await?;

        Ok(ImageAsset {
            prompt: prompt.to_string(),
            png_bytes: bytes,
        })
    }
}

/// NVIDIA のレスポンスから PNG バイト列を抽出する。
/// 候補フィールド: `image` / `images[0]` / `artifacts[0].base64` / `data[0].b64_json` / `data[0].url`
async fn extract_png_bytes(v: &Value, http: &reqwest::Client) -> Result<Vec<u8>> {
    if let Some(s) = v.get("image").and_then(|x| x.as_str()) {
        return decode_b64_or_data_url(s);
    }
    if let Some(arr) = v.get("images").and_then(|x| x.as_array()) {
        if let Some(first) = arr.first() {
            if let Some(s) = first.as_str() {
                return decode_b64_or_data_url(s);
            }
            if let Some(s) = first.get("base64").and_then(|x| x.as_str()) {
                return decode_b64_or_data_url(s);
            }
            if let Some(s) = first.get("b64_json").and_then(|x| x.as_str()) {
                return decode_b64_or_data_url(s);
            }
            if let Some(u) = first.get("url").and_then(|x| x.as_str()) {
                return fetch_url(http, u).await;
            }
        }
    }
    if let Some(arr) = v.get("artifacts").and_then(|x| x.as_array()) {
        if let Some(first) = arr.first() {
            if let Some(s) = first.get("base64").and_then(|x| x.as_str()) {
                return decode_b64_or_data_url(s);
            }
            if let Some(s) = first.get("b64_json").and_then(|x| x.as_str()) {
                return decode_b64_or_data_url(s);
            }
        }
    }
    if let Some(arr) = v.get("data").and_then(|x| x.as_array()) {
        #[derive(Deserialize)]
        struct D { #[serde(default)] b64_json: Option<String>, #[serde(default)] url: Option<String> }
        if let Some(first) = arr.first() {
            if let Ok(d) = serde_json::from_value::<D>(first.clone()) {
                if let Some(s) = d.b64_json {
                    return decode_b64_or_data_url(&s);
                }
                if let Some(u) = d.url {
                    return fetch_url(http, &u).await;
                }
            }
        }
    }
    Err(anyhow!("NVIDIA flux.2 response missing image payload: {}", v))
}

fn decode_b64_or_data_url(s: &str) -> Result<Vec<u8>> {
    let raw = if let Some(idx) = s.find("base64,") {
        &s[idx + "base64,".len()..]
    } else {
        s
    };
    STANDARD.decode(raw.trim()).context("NVIDIA base64 decode")
}

async fn fetch_url(http: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    // M2: bare .send().await? を crate::util::fetch_bytes_with_retry で置換
    crate::util::fetch_bytes_with_retry(http, url, "nvidia_image_dl").await
}
