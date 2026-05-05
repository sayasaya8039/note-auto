//! xAI Grok リサーチクライアント
//!
//! トレンドトピックについて Live Search (web + x) で一次情報、統計、反対意見、
//! 代表 X 投稿を集め、ResearchResult として返す。

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::json;

use super::{date_policy_block, strip_code_fence, today_jst_label, ResearchResult};
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
        let today = today_jst_label();
        let system = format!(
            "あなたは一次情報収集を得意とするリサーチャーです。\
与えられたトピックについて Live Search を使い、日本の読者向け note 記事を書くための\
素材を集めます。主張には必ず出典URLを添えてください。出力は JSON のみ。\n\n\
{}\n\
【key_facts 内の日付ルール】\n\
- 発表日・発売日・イベント日など具体的日付を書くときは、必ず一次情報で裏が取れたものだけを載せる。\n\
- 推測・うろ覚えの日付は禁止。曖昧なら『2026年春』『近日』のように曖昧表現のままにする。\n\
- key_facts に日付を入れる場合は可能な限り citations の URL と紐づけること。",
            date_policy_block(&today)
        );

        let user = format!(
            "【今日の日付】{}\n\
【トレンド取得時刻】{}\n\
【トピック】{}\n\
【既知の文脈】{}\n\n\
このトピックについて web と X (Twitter) を横断検索し、直近の動向を踏まえて\
以下のスキーマで JSON を返してください (```で囲まない):\n\
{{\n  \
\"summary\": \"200-300文字のトピック全体要約 (日本語)。日付は裏取り済みのものだけ。\",\n  \
\"key_facts\": [\"数字や固有名を含む具体的事実1 (日付は裏取り済みのみ)\", \"事実2\", \"事実3\", \"事実4\", \"事実5\"],\n  \
\"citations\": [\"https://...\", \"https://...\"],\n  \
\"counterpoints\": [\"反対意見や留意点1\", \"留意点2\"]\n\
}}",
            today,
            trend.item.fetched_at.format("%Y-%m-%d %H:%M UTC"),
            trend.item.title,
            trend.item.summary.clone().unwrap_or_default(),
        );

        // NOTE: xAI search_parameters は 2026-04 deprecated。Agent Tools API 移行待ち。
        // 当面は Grok のモデル内知識で research を代替する。citations は空で返る可能性あり。
        let body = json!({
            "model": self.model,
            "stream": false,
            "temperature": 0.3,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ]
        });

        // M2: transient (429/502/503) と connect/timeout を 3 回まで指数バックオフで retry
        let resp = crate::util::send_with_retry(
            || self.http
                .post(ENDPOINT)
                .bearer_auth(&self.api_key)
                .json(&body)
                .send(),
            3,
            "xai_grok",
        ).await?;

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

        let mut result = parse_research_lenient(&cleaned).with_context(|| {
            format!(
                "Grok research JSON parse failed. head: {}",
                cleaned.chars().take(400).collect::<String>()
            )
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

/// Grok が返す JSON が ResearchResult のスキーマに完全一致しなくても拾えるように
/// serde_json::Value 経由で寛容に組み立てる。
///
/// 主な救済ケース:
///  - 最上位が配列だけ来る (key_facts のみ返る)
///  - フィールドの型違い (例: counterpoints が文字列で来る)
///  - フィールド欠損
fn parse_research_lenient(s: &str) -> Result<ResearchResult> {
    let v: serde_json::Value = serde_json::from_str(s.trim())
        .map_err(|e| anyhow!("not valid JSON: {}", e))?;

    // 配列のみ返ってきた場合は key_facts として扱う
    if let serde_json::Value::Array(arr) = &v {
        return Ok(ResearchResult {
            summary: String::new(),
            key_facts: arr.iter().map(value_to_string).collect(),
            citations: Vec::new(),
            counterpoints: Vec::new(),
        });
    }

    let obj = v.as_object().ok_or_else(|| anyhow!("top-level is not object/array"))?;

    Ok(ResearchResult {
        summary: obj.get("summary").map(value_to_string).unwrap_or_default(),
        key_facts: obj.get("key_facts").map(value_to_string_vec).unwrap_or_default(),
        citations: obj.get("citations").map(value_to_string_vec).unwrap_or_default(),
        counterpoints: obj.get("counterpoints").map(value_to_string_vec).unwrap_or_default(),
    })
}

fn value_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn value_to_string_vec(v: &serde_json::Value) -> Vec<String> {
    match v {
        serde_json::Value::Array(arr) => arr.iter().map(value_to_string).collect(),
        serde_json::Value::String(s) if !s.is_empty() => vec![s.clone()],
        serde_json::Value::Null => Vec::new(),
        other => vec![other.to_string()],
    }
}

