//! xAI Grok リサーチクライアント
//!
//! トレンドトピックについて Live Search (web + x) で一次情報、統計、反対意見、
//! 代表 X 投稿を集め、ResearchResult として返す。

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::json;

use super::{strip_code_fence, ResearchResult};
use crate::scoring::SelectedTrend;

const ENDPOINT: &str = "https://api.x.ai/v1/chat/completions";

pub struct GrokClient<'a> {
    http: &'a reqwest::Client,
    api_key: String,
    model: String,
}

impl<'a> GrokClient<'a> {
    pub fn new(http: &'a reqwest::Client, api_key: &str, model: &str) -> Self {
        Self {
            http,
            api_key: api_key.to_string(),
            model: model.to_string(),
        }
    }

    pub async fn research(&self, trend: &SelectedTrend) -> Result<ResearchResult> {
        let system = "あなたは一次情報収集を得意とするリサーチャーです。\
与えられたトピックについて Live Search を使い、日本の読者向け note 記事を書くための\
素材を集めます。主張には必ず出典URLを添えてください。出力は JSON のみ。";

        let user = format!(
            "【トピック】{}\n\
【既知の文脈】{}\n\n\
このトピックについて web と X (Twitter) を横断検索し、直近の動向を踏まえて\
以下のスキーマで JSON を返してください (```で囲まない):\n\
{{\n  \
\"summary\": \"200-300文字のトピック全体要約 (日本語)\",\n  \
\"key_facts\": [\"数字や固有名を含む具体的事実1\", \"事実2\", \"事実3\", \"事実4\", \"事実5\"],\n  \
\"citations\": [\"https://...\", \"https://...\"],\n  \
\"counterpoints\": [\"反対意見や留意点1\", \"留意点2\"]\n\
}}",
            trend.item.title,
            trend.item.summary.clone().unwrap_or_default(),
        );

        let body = json!({
            "model": self.model,
            "stream": false,
            "temperature": 0.2,
            "search_parameters": {
                "mode": "on",
                "sources": [{"type": "web"}, {"type": "x"}],
                "return_citations": true
            },
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ]
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
            return Err(anyhow!("Grok API {}: {}", status, txt));
        }

        #[derive(Deserialize)]
        struct Chat { choices: Vec<Choice>, #[serde(default)] citations: Vec<String> }
        #[derive(Deserialize)]
        struct Choice { message: Msg }
        #[derive(Deserialize)]
        struct Msg { content: String }

        let parsed: Chat = resp.json().await?;
        let content = parsed.choices.first()
            .ok_or_else(|| anyhow!("empty grok choices"))?
            .message.content.clone();
        let cleaned = strip_code_fence(&content);

        let mut result: ResearchResult = serde_json::from_str(&cleaned).with_context(|| {
            format!("Grok research JSON parse failed. head: {}", cleaned.chars().take(200).collect::<String>())
        })?;

        // Grok API 側の citations 配列が返ればマージ
        if !parsed.citations.is_empty() {
            for c in parsed.citations {
                if !result.citations.contains(&c) {
                    result.citations.push(c);
                }
            }
        }

        Ok(result)
    }

    /// dry-run 用スタブ
    pub fn stub(trend: &SelectedTrend) -> ResearchResult {
        ResearchResult {
            summary: format!("[dry-run stub] {} のリサーチ結果プレースホルダ", trend.item.title),
            key_facts: vec!["事実A (dry-run)".into(), "事実B (dry-run)".into()],
            citations: trend.item.url.iter().cloned().collect(),
            counterpoints: vec!["留意点 (dry-run)".into()],
        }
    }
}
