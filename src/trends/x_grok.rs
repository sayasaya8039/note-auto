//! xAI Grok の Responses API + `x_search` Agent Tool で X の **現時刻バズ TOP 3** を取得する。
//!
//! 旧実装は `/v1/chat/completions` + `search_parameters` を使っていたが、
//! 2026-04 に search_parameters が deprecated (410 Gone) になったため、
//! Agent Tools API (`/v1/responses` + `tools: [{type: "x_search"}]`) に移行。
//! これにより Grok 自身が実時間で X を検索し、実在するポスト URL と
//! エンゲージ数を返してくれる。
//!
//! 出力ポリシー:
//!   - 配列の先頭 3 件は **現時刻でバズっている TOP 3 ポスト/スレッド** (viral=true)
//!   - 残りの top_n-3 件は note 記事化に値する関連トピック (viral=false)
//!   - viral=true の raw_score = 95.0、それ以外は Grok が返した値 (50-80 程度)
//!   - 調査ソースは X Explore の News タブ + Entertainment タブを優先し、
//!     ジャンルに偏りが出ないよう最低 1 件ずつ含めるよう指示している
//!   - category フィールド ("news" / "entertainment" / "other") を metrics に保存

use anyhow::{anyhow, Result};
use serde::Deserialize;
use serde_json::json;

use crate::config::Config;
use crate::trends::TrendItem;

const RESPONSES_ENDPOINT: &str = "https://api.x.ai/v1/responses";
const CHAT_ENDPOINT: &str = "https://api.x.ai/v1/chat/completions";
/// Responses API + tools (x_search) のモデル候補。
/// **server-side tools (x_search) は grok-4 family のみサポート**
/// (grok-3-latest は HTTP 400 で拒否されるので候補から除外)。
/// 候補が全部失敗したら chat/completions API + Grok 学習知識フォールバックへ。
const MODEL_CANDIDATES: &[&str] = &["grok-4-latest"];
/// 各モデルでのリトライ回数 (initial + retry)。
/// x_search はキャパ不足時にハングしがちなので、長く待たずに即フォールバック。
const PER_MODEL_RETRIES: usize = 2;
/// 1 リクエストあたりの総タイムアウト (秒)。x_search は重いが、ハング時の待機を抑える。
const PER_REQUEST_TIMEOUT_SECS: u64 = 75;

/// xAI 用ローカルクライアント。HTTP/1.1 強制で安定化。
fn xai_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("note-auto/0.7 (xai)")
        .timeout(std::time::Duration::from_secs(PER_REQUEST_TIMEOUT_SECS))
        .connect_timeout(std::time::Duration::from_secs(15))
        .http1_only()
        .gzip(true)
        .pool_idle_timeout(std::time::Duration::from_secs(0))
        .pool_max_idle_per_host(0)
        .build()
        .map_err(|e| anyhow!("xai_client build failed: {e}"))
}

fn err_chain(e: &reqwest::Error) -> String {
    use std::error::Error as _;
    let mut s = format!("{}", e);
    let mut src: Option<&(dyn std::error::Error + 'static)> = e.source();
    while let Some(e2) = src {
        s.push_str(&format!(" -> {}", e2));
        src = e2.source();
    }
    s
}

pub async fn fetch(_shared: &reqwest::Client, cfg: &Config) -> Result<Vec<TrendItem>> {
    let Some(api_key) = cfg.trends.xai_api_key.as_deref() else {
        tracing::debug!("XAI_API_KEY 未設定 — X/Grok スキップ");
        return Ok(vec![]);
    };
    let client = xai_client()?;
    let hours = cfg.trends.window_hours.max(1);
    let top_n = cfg.trends.max_per_source.max(5);
    let viral_n = 3usize.min(top_n);
    let extra_n = top_n.saturating_sub(viral_n);

    // 現在時刻 (JST) を埋め込み、x_search に「直近=今」を強く示唆する
    let now_utc = chrono::Utc::now();
    let now_jst = now_utc.with_timezone(&chrono::FixedOffset::east_opt(9 * 3600).unwrap());
    let now_jst_str = now_jst.format("%Y-%m-%d %H:%M JST").to_string();

    let prompt = format!(
        "あなたは X (Twitter) のリアルタイムバズアナリストです。\n\
現在時刻: {now_jst_str}\n\n\
【調査ソース (優先度順)】\n\
1. X Explore — News タブ: https://x.com/explore/tabs/news (直近で伸びているニュース系トピック)\n\
2. X Explore — Entertainment タブ: https://x.com/explore/tabs/entertainment (エンタメ系トレンド)\n\
3. その他、x_search ツールで取得できる現時刻バズポスト全般\n\n\
【タスク】\n\
ツール `x_search` を使って **現時刻 (直近 {hours} 時間) で X 上で実際にバズっているポスト / スレッド** \
を調査し、note 記事化に値するものを以下のルールで返してください。\n\n\
【選定ルール】\n\
1. **先頭 3 件 (viral=true)**: 直近 {hours} 時間で **インプレッション / リポスト / いいね数が突出して高い** \
ポストまたはスレッドの TOP 3。News タブと Entertainment タブを横断して、ジャンルに偏りがないように \
最低 1 件は News 系、最低 1 件は Entertainment 系を含めること。日本語ポストを優先しつつ、\
日本人読者が興味を持てる海外ポストも可。下品/差別/スパム/医療デマ/投資勧誘は除外。\n\
2. **残り {extra_n} 件 (viral=false)**: 同期間の関連トピックや派生議論で、note 記事として価値があるもの。\
   News と Entertainment 双方からバランスよく拾う。\n\n\
【出力形式】 JSON 配列のみ (コードブロック禁止、前後に説明文禁止):\n\
[\n\
  {{\n\
    \"title\": \"記事化用の自然なタイトル (40 文字以内)\",\n\
    \"summary\": \"1-2 文で要点。なぜ今バズっているかが分かるように\",\n\
    \"url\": \"代表ポストの実 URL (https://x.com/.../status/...)\",\n\
    \"viral\": true,\n\
    \"category\": \"news\" または \"entertainment\" または \"other\",\n\
    \"impressions\": 数値(分かれば。不明なら null),\n\
    \"likes\": 数値(分かれば。不明なら null),\n\
    \"reposts\": 数値(分かれば。不明なら null),\n\
    \"score\": 1-100 の数値(impressions が大きいほど高く)\n\
  }},\n\
  ...\n\
]\n\n\
合計 {top_n} 件、先頭 {viral_n} 件は必ず viral=true (現時刻バズ TOP) にしてください。"
    );

    // モデル候補をループしてリクエストし、503 (capacity) は次のモデルへフェイルオーバー。
    // 各モデルで送信エラーは 3 回までリトライ。
    let text = match call_responses_api(&client, api_key, &prompt).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "Responses API 全候補失敗 — chat/completions フォールバックへ");
            // 最終フォールバック: 旧 chat/completions API + Grok 知識ベース (実時間性は低いが取れることが多い)
            call_chat_fallback(&client, api_key, &prompt).await?
        }
    };
    let cleaned = strip_code_fence(&text);

    #[derive(Deserialize)]
    struct Topic {
        title: String,
        #[serde(default)]
        summary: Option<String>,
        #[serde(default)]
        url: Option<String>,
        #[serde(default)]
        viral: bool,
        #[serde(default)]
        category: Option<String>,
        #[serde(default)]
        impressions: Option<i64>,
        #[serde(default)]
        likes: Option<i64>,
        #[serde(default)]
        reposts: Option<i64>,
        #[serde(default)]
        score: Option<f64>,
    }

    let topics: Vec<Topic> = serde_json::from_str(&cleaned).map_err(|e| {
        anyhow!(
            "Grok content JSON parse failed: {e}. content head: {:?}",
            cleaned.chars().take(300).collect::<String>()
        )
    })?;

    let viral_count = topics.iter().filter(|t| t.viral).count();
    tracing::info!(
        total = topics.len(),
        viral = viral_count,
        "x/Grok: x_search returned topics"
    );

    let items: Vec<TrendItem> = topics
        .into_iter()
        .map(|t| {
            let mut it = TrendItem::new("x", t.title);
            it.summary = t.summary;
            it.url = t.url;
            // viral は固定で高スコア (95)、それ以外は Grok の自己採点
            it.raw_score = if t.viral { 95.0 } else { t.score.unwrap_or(50.0) };
            it.metrics = json!({
                "viral": t.viral,
                "category": t.category.unwrap_or_else(|| "other".to_string()),
                "impressions": t.impressions,
                "likes": t.likes,
                "reposts": t.reposts,
            });
            it
        })
        .collect();

    Ok(items)
}

/// Responses API + tools:[{type:x_search}] を MODEL_CANDIDATES の順で呼び、
/// 最初に 200 を返したモデルの本文を返す。送信エラーは各モデル 3 回までリトライ。
/// 503 (capacity) や 429 (rate limit) は次のモデルへフェイルオーバー。
async fn call_responses_api(
    client: &reqwest::Client,
    api_key: &str,
    prompt: &str,
) -> Result<String> {
    let mut all_errs: Vec<String> = Vec::new();
    for model in MODEL_CANDIDATES {
        let body = json!({
            "model": model,
            "input": prompt,
            "tools": [{ "type": "x_search" }],
        });
        let mut send_err: Option<String> = None;
        for attempt in 0..PER_MODEL_RETRIES {
            match client
                .post(RESPONSES_ENDPOINT)
                .bearer_auth(api_key)
                .json(&body)
                .send()
                .await
            {
                Ok(r) => {
                    let status = r.status();
                    if status.is_success() {
                        let raw: serde_json::Value = match r.json().await {
                            Ok(v) => v,
                            Err(e) => {
                                all_errs.push(format!("{model}: json parse: {e}"));
                                break;
                            }
                        };
                        if let Some(t) = extract_text(&raw) {
                            tracing::info!(model = model, "x/Grok: Responses API OK");
                            return Ok(t);
                        }
                        all_errs.push(format!(
                            "{model}: text 抽出失敗 head={:?}",
                            raw.to_string().chars().take(200).collect::<String>()
                        ));
                        break;
                    }
                    let txt = r.text().await.unwrap_or_default();
                    let msg = format!("{model}: HTTP {} {}", status.as_u16(), txt.chars().take(200).collect::<String>());
                    tracing::warn!(model = model, status = %status, "Responses API non-2xx");
                    // 503/429 は次のモデルへ。それ以外 (4xx) もモデル切り替えの価値はあるので break。
                    all_errs.push(msg);
                    break;
                }
                Err(e) => {
                    let detail = err_chain(&e);
                    tracing::warn!(model = model, attempt, error = %detail, "Responses API send err, retry");
                    send_err = Some(format!("{model}: send err {} ({})", attempt + 1, detail));
                    tokio::time::sleep(std::time::Duration::from_millis(800 * (attempt as u64 + 1))).await;
                }
            }
        }
        if let Some(e) = send_err {
            all_errs.push(e);
        }
    }
    Err(anyhow!(
        "Responses API 全候補失敗: {}",
        all_errs.join(" | ")
    ))
}

/// 最終フォールバック: 旧 /v1/chat/completions API を Grok の学習知識のみで叩く。
/// x_search なしなので実時間性は劣るが、503 でも応答が返ることが多い。
async fn call_chat_fallback(
    client: &reqwest::Client,
    api_key: &str,
    user_prompt: &str,
) -> Result<String> {
    let body = json!({
        "model": "grok-3-latest",
        "stream": false,
        "temperature": 0.4,
        "messages": [
            {"role": "system", "content": "出力は厳密に JSON 配列のみ。前後に説明文を入れない。\
あなたは X と web に関する最新知識を持つアナリスト。x_search ツールが使えないため、\
学習データの範囲内で『最近バズりそうなトピック』を推測で構わないので返してください。"},
            {"role": "user", "content": user_prompt}
        ]
    });
    let resp = client
        .post(CHAT_ENDPOINT)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow!("chat fallback send err: {}", err_chain(&e)))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let txt = resp.text().await.unwrap_or_default();
        return Err(anyhow!("chat fallback {}: {}", status, txt.chars().take(300).collect::<String>()));
    }
    let v: serde_json::Value = resp.json().await?;
    let content = v
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .ok_or_else(|| anyhow!("chat fallback: choices[0].message.content 不在"))?;
    tracing::info!("x/Grok: chat fallback OK");
    Ok(content.to_string())
}

/// xAI Responses API のレスポンスから本文テキストを取り出す。
/// 形式 1: { "output": [ { "content": [ { "text": "..." } ] } ] }
/// 形式 2: { "output_text": "..." }
fn extract_text(v: &serde_json::Value) -> Option<String> {
    if let Some(arr) = v.get("output").and_then(|o| o.as_array()) {
        let mut parts = vec![];
        for item in arr {
            if let Some(content) = item.get("content").and_then(|c| c.as_array()) {
                for c in content {
                    if let Some(t) = c.get("text").and_then(|t| t.as_str()) {
                        if !t.trim().is_empty() {
                            parts.push(t.to_string());
                        }
                    }
                }
            }
        }
        if !parts.is_empty() {
            return Some(parts.join("\n").trim().to_string());
        }
    }
    for k in ["output_text", "text", "content"] {
        if let Some(s) = v.get(k).and_then(|x| x.as_str()) {
            if !s.trim().is_empty() {
                return Some(s.trim().to_string());
            }
        }
    }
    None
}

fn strip_code_fence(s: &str) -> String {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix("```json") {
        return rest.trim_end_matches("```").trim().to_string();
    }
    if let Some(rest) = t.strip_prefix("```") {
        return rest.trim_end_matches("```").trim().to_string();
    }
    // 文中に JSON 配列が混じっている場合に最初の `[` から最後の `]` を抽出
    if let (Some(l), Some(r)) = (t.find('['), t.rfind(']')) {
        if l < r {
            return t[l..=r].to_string();
        }
    }
    t.to_string()
}
