use crate::trends::TrendItem;
use unicode_segmentation::UnicodeSegmentation;

/// 正規化後の選定結果
#[derive(Debug, Clone, serde::Serialize)]
pub struct SelectedTrend {
    #[serde(flatten)]
    pub item: TrendItem,
    pub normalized_score: f64,
    pub composite_score: f64,
}

/// 上位 top 件を選定。
/// 1. ソース内でスコアを 0-1 に正規化
/// 2. ソース別ウェイト適用
/// 3. タイトル類似度で重複除去
/// 4. 上位 top 件
pub fn select_top(items: Vec<TrendItem>, top: usize) -> Vec<SelectedTrend> {
    if items.is_empty() {
        return vec![];
    }

    let default_weights = crate::config::ScoringConfig::default().source_weights;

    let mut by_source: std::collections::HashMap<String, Vec<TrendItem>> = Default::default();
    for item in items {
        by_source.entry(item.source.clone()).or_default().push(item);
    }

    let mut normalized: Vec<SelectedTrend> = Vec::new();
    for (src, mut group) in by_source {
        let max = group.iter().map(|t| t.raw_score).fold(0.0_f64, f64::max);
        let weight = default_weights.get(&src).copied().unwrap_or(1.0);
        group.sort_by(|a, b| b.raw_score.partial_cmp(&a.raw_score).unwrap_or(std::cmp::Ordering::Equal));
        for item in group {
            let norm = if max > 0.0 { item.raw_score / max } else { 0.0 };
            let composite = norm * weight;
            normalized.push(SelectedTrend {
                item,
                normalized_score: norm,
                composite_score: composite,
            });
        }
    }

    normalized.sort_by(|a, b| b.composite_score.partial_cmp(&a.composite_score).unwrap_or(std::cmp::Ordering::Equal));

    // 重複除去
    let mut selected: Vec<SelectedTrend> = Vec::new();
    for cand in normalized {
        let dup = selected.iter().any(|s| title_similarity(&s.item.title, &cand.item.title) >= 0.65);
        if !dup {
            selected.push(cand);
            if selected.len() >= top {
                break;
            }
        }
    }
    selected
}

/// Jaccard類似度（grapheme bigramベース）
fn title_similarity(a: &str, b: &str) -> f64 {
    let ga = bigrams(a);
    let gb = bigrams(b);
    if ga.is_empty() || gb.is_empty() {
        return 0.0;
    }
    let inter = ga.intersection(&gb).count() as f64;
    let union = ga.union(&gb).count() as f64;
    inter / union
}

fn bigrams(s: &str) -> std::collections::HashSet<String> {
    let lower = s.to_lowercase();
    let gs: Vec<&str> = lower.graphemes(true).collect();
    if gs.len() < 2 {
        return std::iter::once(lower).collect();
    }
    gs.windows(2).map(|w| w.concat()).collect()
}
