//! Pollo AI text-to-image クライアント (openai-gpt-image-2-0 等を経由)
//!
//! フロー:
//!   1. POST /generation/text2image         → task {id, status: "waiting"}
//!   2. GET  /generation/{id} (ポーリング)    → status が "succeed" になるまで待機
//!   3. videoList[0].videoUrlNoWatermark の PNG をダウンロード

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;

use super::{ArticleBrief, ImageAsset};

const BASE: &str = "https://pollo.ai/api/platform";

pub struct PolloClient<'a> {
    http: &'a reqwest::Client,
    api_key: String,
    model_name: String,
    aspect_ratio: String,
}

impl<'a> PolloClient<'a> {
    pub fn new(http: &'a reqwest::Client, api_key: &str, model_name: &str, aspect_ratio: &str) -> Self {
        Self {
            http,
            api_key: api_key.to_string(),
            model_name: model_name.to_string(),
            aspect_ratio: aspect_ratio.to_string(),
        }
    }

    pub async fn generate(&self, brief: &ArticleBrief) -> Result<ImageAsset> {
        let prompt = build_prompt(brief);

        // 1. submit
        let submit = json!({
            "generationInput": {
                "modelName": self.model_name,
                "prompt": prompt,
                "aspectRatio": self.aspect_ratio,
                "numOutputs": 1,
            }
        });

        let resp = self.http
            .post(format!("{BASE}/generation/text2image"))
            .header("x-api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&submit)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Pollo submit {}: {}", status, txt));
        }

        #[derive(Deserialize)]
        struct SubmitResp { data: TaskRef }
        #[derive(Deserialize)]
        struct TaskRef { id: u64, #[allow(dead_code)] status: String }

        let submit_resp: SubmitResp = resp.json().await.context("parse submit response")?;
        let task_id = submit_resp.data.id;
        tracing::info!(task_id, model = %self.model_name, "pollo task submitted");

        // 2. poll (generation は gpt-image-2 で 10-60秒程度)
        let url = format!("{BASE}/generation/{task_id}");
        let png_url = self.poll_until_done(&url, 60, Duration::from_secs(5)).await?;

        // 3. download
        let png_bytes = self.http
            .get(&png_url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?
            .to_vec();

        Ok(ImageAsset { prompt, png_bytes })
    }

    async fn poll_until_done(&self, url: &str, max_iter: usize, interval: Duration) -> Result<String> {
        #[derive(Deserialize)]
        struct PollResp { data: TaskRecord }
        #[derive(Deserialize)]
        struct TaskRecord {
            status: String,
            #[serde(rename = "failMsg")]
            fail_msg: Option<String>,
            #[serde(rename = "videoList", default)]
            video_list: Vec<VideoItem>,
        }
        #[derive(Deserialize)]
        struct VideoItem {
            #[serde(rename = "videoUrlNoWatermark")]
            video_url_no_watermark: Option<String>,
            #[serde(rename = "videoUrl")]
            video_url: Option<String>,
        }

        for i in 1..=max_iter {
            tokio::time::sleep(interval).await;
            let resp = self.http
                .get(url)
                .header("x-api-key", &self.api_key)
                .send()
                .await?;
            if !resp.status().is_success() {
                let s = resp.status();
                tracing::debug!(status = %s, iter = i, "poll non-200");
                continue;
            }
            let poll: PollResp = resp.json().await.context("parse poll response")?;
            match poll.data.status.as_str() {
                "succeed" => {
                    let first = poll.data.video_list.first()
                        .ok_or_else(|| anyhow!("succeed but empty videoList"))?;
                    let png = first.video_url_no_watermark.clone()
                        .or_else(|| first.video_url.clone())
                        .ok_or_else(|| anyhow!("no videoUrl in result"))?;
                    return Ok(png);
                }
                "failed" => {
                    return Err(anyhow!("pollo task failed: {}",
                        poll.data.fail_msg.unwrap_or_else(|| "unknown".into())));
                }
                "waiting" | "processing" => {
                    tracing::debug!(iter = i, status = %poll.data.status, "polling");
                }
                other => {
                    tracing::warn!(status = %other, "unknown pollo status");
                }
            }
        }
        Err(anyhow!("pollo poll timeout after {} iterations", max_iter))
    }
}

fn build_prompt(brief: &ArticleBrief) -> String {
    format!(
        "Hero banner illustration for a Japanese note article titled \"{}\". \
Category: {}. Keywords: {}. \
Modern, clean editorial style. Strong composition, abstract metaphor in the center. \
No text, no letters, no characters of any language. \
Soft palette with one accent color, cinematic lighting, 16:9 aspect.",
        brief.title,
        brief.category,
        brief.tags.join(", ")
    )
}
