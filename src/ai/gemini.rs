//! Google Gemini Image API: Nano Banana 2 (gemini-3.1-flash-image-preview)
//!
//! 本文挿入画像 (low quality) 用。Nano Banana 2 は速度と品質を両立しており、
//! NVIDIA flux.2-klein-4b の代替として inline 画像で使用する。
//!
//! エンドポイント: https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent
//! 認証: x-goog-api-key ヘッダー
//! リクエスト: { contents: [{parts: [{text: prompt}]}], generationConfig: { responseModalities: ["IMAGE"] } }
//! レスポンス: candidates[0].content.parts[].inlineData.data (base64 PNG/JPEG)

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};

use super::ImageAsset;

const ENDPOINT_TEMPLATE: &str =
    "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent";

pub struct GeminiImageClient<'a> {
    http: &'a reqwest::Client,
    api_key: String,
    model: String,
    aspect_ratio: String,
    image_size: String,
    thinking_level: String,
}

impl<'a> GeminiImageClient<'a> {
    pub fn new(http: &'a reqwest::Client, api_key: &str, model: &str) -> Self {
        // 既定: アスペクト比 16:9 / 解像度 1K / Thinking High モード (Gemini 3.1 Deep Think)
        // 注意: responseModalities に "TEXT" を含めると imageSize が 1K に固定されるバグあり。
        // ["IMAGE"] のみで送ること。
        // .env から読んだキーに CR/LF が混入しているケースを防ぐため trim する。
        Self {
            http,
            api_key: api_key.trim().to_string(),
            model: model.to_string(),
            aspect_ratio: "16:9".to_string(),
            image_size: "1K".to_string(),
            thinking_level: "high".to_string(),
        }
    }

    #[allow(dead_code)]
    pub fn with_aspect_ratio(mut self, ar: &str) -> Self {
        self.aspect_ratio = ar.to_string();
        self
    }

    #[allow(dead_code)]
    pub fn with_image_size(mut self, sz: &str) -> Self {
        self.image_size = sz.to_string();
        self
    }

    #[allow(dead_code)]
    pub fn with_thinking_level(mut self, lvl: &str) -> Self {
        self.thinking_level = lvl.to_string();
        self
    }

    pub async fn generate_prompt(&self, prompt: &str) -> Result<ImageAsset> {
        let endpoint = ENDPOINT_TEMPLATE.replace("{model}", &self.model);
        let body = json!({
            "contents": [{
                "parts": [{ "text": prompt }]
            }],
            "generationConfig": {
                "responseModalities": ["IMAGE"],
                "imageConfig": {
                    "aspectRatio": self.aspect_ratio,
                    "imageSize": self.image_size,
                },
                "thinkingConfig": {
                    "thinkingLevel": self.thinking_level,
                }
            }
        });

        // M2: transient + connect/timeout を 3 回まで指数バックオフで retry
        let resp = crate::util::send_with_retry(
            || self.http
                .post(&endpoint)
                .header("x-goog-api-key", &self.api_key)
                .header("Content-Type", "application/json")
                .json(&body)
                .send(),
            3,
            "gemini_image",
        ).await?;

        let status = resp.status();
        if !status.is_success() {
            let txt = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Gemini image {} ({}): {}", self.model, status, txt));
        }

        let v: Value = resp.json().await.context("Gemini response parse")?;
        let bytes = extract_inline_image(&v).ok_or_else(|| {
            anyhow!(
                "Gemini response missing inlineData. body head: {}",
                v.to_string().chars().take(400).collect::<String>()
            )
        })?;

        Ok(ImageAsset {
            prompt: prompt.to_string(),
            png_bytes: bytes,
        })
    }
}

/// candidates[0].content.parts[] を走査し、最初の inlineData.data を base64 デコード。
fn extract_inline_image(v: &Value) -> Option<Vec<u8>> {
    let parts = v
        .get("candidates")?
        .as_array()?
        .first()?
        .get("content")?
        .get("parts")?
        .as_array()?;
    for p in parts {
        // camelCase ("inlineData") と snake_case ("inline_data") 双方を許容
        let inline = p.get("inlineData").or_else(|| p.get("inline_data"))?;
        if let Some(s) = inline.get("data").and_then(|x| x.as_str()) {
            if let Ok(bytes) = STANDARD.decode(s.trim()) {
                return Some(bytes);
            }
        }
    }
    None
}
