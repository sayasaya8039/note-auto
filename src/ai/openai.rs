//! OpenAI Images API (gpt-image-1) によるアイキャッチ画像生成
//!
//! gpt-image-1 は base64 で画像を返す。PNG として保存可能。

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use serde_json::json;

use super::ImageAsset;

const ENDPOINT: &str = "https://api.openai.com/v1/images/generations";

pub struct OpenAiImageClient<'a> {
    http: &'a reqwest::Client,
    api_key: String,
    model: String,
    size: String,
}

impl<'a> OpenAiImageClient<'a> {
    pub fn new(http: &'a reqwest::Client, api_key: &str, model: &str, size: &str) -> Self {
        Self {
            http,
            api_key: api_key.to_string(),
            model: model.to_string(),
            size: size.to_string(),
        }
    }

    pub async fn generate_prompt(&self, prompt: &str) -> Result<ImageAsset> {
        let prompt = prompt.to_string();
        let body = json!({
            "model": self.model,
            "prompt": prompt,
            "size": self.size,
            "n": 1,
        });

        let resp = self.http
            .post(ENDPOINT)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(anyhow!("OpenAI Images {}: {}", status, txt));
        }

        #[derive(Deserialize)]
        struct Resp { data: Vec<ImgData> }
        #[derive(Deserialize)]
        struct ImgData {
            #[serde(default)]
            b64_json: Option<String>,
            #[serde(default)]
            url: Option<String>,
        }

        let parsed: Resp = resp.json().await?;
        let first = parsed.data.into_iter().next().ok_or_else(|| anyhow!("empty image data"))?;

        let bytes = if let Some(b64) = first.b64_json {
            STANDARD.decode(b64).context("b64 decode")?
        } else if let Some(u) = first.url {
            self.http.get(&u).send().await?.error_for_status()?.bytes().await?.to_vec()
        } else {
            return Err(anyhow!("image response had neither b64_json nor url"));
        };

        Ok(ImageAsset { prompt, png_bytes: bytes })
    }
}
