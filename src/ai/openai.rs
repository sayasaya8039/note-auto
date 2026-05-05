//! OpenAI Images API (gpt-image-1) によるアイキャッチ画像生成
//!
//! gpt-image-1 は base64 で画像を返す。PNG として保存可能。

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use serde_json::json;

use super::client::AiClient;
use super::ImageAsset;

const ENDPOINT: &str = "https://api.openai.com/v1/images/generations";

pub struct OpenAiImageClient<'a> {
    http: &'a reqwest::Client,
    api_key: String,
    model: String,
    size: String,
}

impl AiClient for OpenAiImageClient<'_> {
    fn label(&self) -> &'static str {
        "openai_image"
    }

    fn build_request(&self, body: &serde_json::Value) -> reqwest::RequestBuilder {
        self.http
            .post(ENDPOINT)
            .bearer_auth(&self.api_key)
            .json(body)
    }
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

        #[derive(Deserialize)]
        struct Resp { data: Vec<ImgData> }
        #[derive(Deserialize)]
        struct ImgData {
            #[serde(default)]
            b64_json: Option<String>,
            #[serde(default)]
            url: Option<String>,
        }

        // AF2: AiClient + client::send_json 経由で M2 retry + status check + parse 統合
        let parsed: Resp = super::client::send_json(self, &body).await?;
        let first = parsed.data.into_iter().next().ok_or_else(|| anyhow!("empty image data"))?;

        let bytes = if let Some(b64) = first.b64_json {
            STANDARD.decode(b64).context("b64 decode")?
        } else if let Some(u) = first.url {
            // M2: bare .send().await? を crate::util::fetch_bytes_with_retry で置換
            crate::util::fetch_bytes_with_retry(self.http, &u, "openai_image_dl").await?
        } else {
            return Err(anyhow!("image response had neither b64_json nor url"));
        };

        Ok(ImageAsset { prompt, png_bytes: bytes })
    }
}
