//! 共通ユーティリティ

use std::collections::HashSet;
use std::sync::LazyLock;
use unicode_segmentation::UnicodeSegmentation;

/// JSON レスポンスからコードフェンスを剥がす
pub fn strip_code_fence(s: &str) -> String {
    let t = s.trim();
    for prefix in ["```json", "```JSON", "```"] {
        if let Some(rest) = t.strip_prefix(prefix) {
            return rest.trim_end_matches("```").trim().to_string();
        }
    }
    // 文中に JSON 配列が混じっている場合に最初の `[` から最後の `]` を抽出
    if let (Some(l), Some(r)) = (t.find('['), t.rfind(']')) {
        if l < r {
            return t[l..=r].to_string();
        }
    }
    t.to_string()
}

static RE_HTML: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"<[^>]+>").unwrap());

/// HTML タグを除去してプレーンテキストにする
pub fn strip_html(s: &str) -> String {
    let stripped = RE_HTML.replace_all(s, " ");
    html_escape::decode_html_entities(&stripped)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Jaccard 類似度 (grapheme bigram ベース)
pub fn title_similarity(a: &str, b: &str) -> f64 {
    let ga = bigrams(a);
    let gb = bigrams(b);
    if ga.is_empty() || gb.is_empty() {
        return 0.0;
    }
    let inter = ga.intersection(&gb).count() as f64;
    let union = ga.union(&gb).count() as f64;
    inter / union
}

fn bigrams(s: &str) -> HashSet<String> {
    let lower = s.to_lowercase();
    let gs: Vec<&str> = lower.graphemes(true).collect();
    if gs.len() < 2 {
        return std::iter::once(lower).collect();
    }
    gs.windows(2).map(|w| w.concat()).collect()
}

/// 共通 HTTP クライアント (Result 返却)
pub fn http_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent("note-auto/0.1 (+https://note.com)")
        .timeout(std::time::Duration::from_secs(30))
        .gzip(true)
        .build()?)
}

/// AI 用 HTTP クライアント (タイムアウト長め、Result 返却)
pub fn http_client_long() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent("note-auto/0.2")
        .timeout(std::time::Duration::from_secs(180))
        .gzip(true)
        .build()?)
}

/// YAML 値のエスケープ (インジェクション防止)
pub fn yaml_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "")
}
