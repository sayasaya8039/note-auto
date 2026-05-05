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

use super::client::AiClient;
use super::ImageAsset;

const BASE: &str = "https://pollo.ai/api/platform";

pub struct PolloClient<'a> {
    http: &'a reqwest::Client,
    api_key: String,
    model_name: String,
    aspect_ratio: String,
    quality: String,
}

impl AiClient for PolloClient<'_> {
    fn label(&self) -> &'static str {
        "pollo_submit"
    }

    /// AF2: submit (text2image POST) のみ trait 化。
    /// polling (GET /generation/{id}) は別フロー (poll_until_done) で別途処理し、
    /// quality H1 (polling unretried) は別 PR で対応予定 (本 AF2 のスコープ外)。
    fn build_request(&self, body: &serde_json::Value) -> reqwest::RequestBuilder {
        let submit_url = format!("{BASE}/generation/text2image");
        self.http
            .post(submit_url)
            .header("x-api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(body)
    }
}

impl<'a> PolloClient<'a> {
    pub fn new(http: &'a reqwest::Client, api_key: &str, model_name: &str, aspect_ratio: &str) -> Self {
        Self {
            http,
            api_key: api_key.to_string(),
            model_name: model_name.to_string(),
            aspect_ratio: aspect_ratio.to_string(),
            quality: "high".to_string(),
        }
    }

    pub fn with_quality(mut self, q: &str) -> Self {
        self.quality = q.to_string();
        self
    }

    /// 任意プロンプトで画像を1枚生成
    pub async fn generate_with_prompt(&self, prompt: &str) -> Result<ImageAsset> {
        // 1. submit
        let submit = json!({
            "generationInput": {
                "modelName": self.model_name,
                "prompt": prompt,
                "aspectRatio": self.aspect_ratio,
                "quality": self.quality,
                "numOutputs": 1,
            }
        });

        #[derive(Deserialize)]
        struct SubmitResp { data: TaskRef }
        #[derive(Deserialize)]
        struct TaskRef { id: u64, #[allow(dead_code)] status: String }

        // AF2: AiClient + client::send_json 経由で M2 retry + status check + parse 統合
        let submit_resp: SubmitResp = super::client::send_json(self, &submit).await
            .context("Pollo submit failed")?;
        let task_id = submit_resp.data.id;
        tracing::info!(task_id, model = %self.model_name, quality = %self.quality, "pollo task submitted");

        // 2. poll
        let url = format!("{BASE}/generation/{task_id}");
        let png_url = self.poll_until_done(&url, 60, Duration::from_secs(5)).await?;

        // 3. download
        // M2: bare .send().await? を crate::util::fetch_bytes_with_retry で置換、
        //     network blip による image silent loss を防ぐ。
        let png_bytes = crate::util::fetch_bytes_with_retry(
            self.http,
            &png_url,
            "pollo_image_dl",
        ).await?;

        Ok(ImageAsset { prompt: prompt.to_string(), png_bytes })
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

            // H1 (quality 報告): 旧実装は `?` で network blip 1 回で task 全 fail →
            //                   pollo 課金済 + inline_image_paths 0 件の silent loss。
            // 修正: send_with_retry で transient + connect/timeout を 2 回まで retry、
            //       それでも失敗なら continue で polling 続行。
            //       (polling 自体が「待機 + 再試行」なので、1 iter 失敗 = 全 task abort
            //        という挙動は本来不要だった。)
            //
            // Independently identified by quality (H1) and team-lead (Plan A).
            let resp = match crate::util::send_with_retry(
                || self.http.get(url).header("x-api-key", &self.api_key).send(),
                2,
                "pollo_poll",
            ).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(error = %e, iter = i, "pollo poll send failed, continuing");
                    continue;
                }
            };

            if !resp.status().is_success() {
                tracing::debug!(status = %resp.status(), iter = i, "poll non-200");
                continue;
            }
            // H1: JSON parse 失敗時も `?` で全 abort せず continue
            //     (一時的な malformed response や接続切れに対する resilience)
            let poll: PollResp = match resp.json().await {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(error = %e, iter = i, "pollo poll JSON parse failed, continuing");
                    continue;
                }
            };
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
                other => tracing::warn!(status = %other, "unknown pollo status"),
            }
        }
        Err(anyhow!("pollo poll timeout after {} iterations", max_iter))
    }
}
