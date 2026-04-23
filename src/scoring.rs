use crate::trends::TrendItem;
use unicode_segmentation::UnicodeSegmentation;

/// 正規化後の選定結果
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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

    // カテゴリ/ソース分散を強制:
    //  1st pass: 各 source から高スコア順に 1 件ずつ ラウンドロビンで拾う
    //  それでも top に満たない場合は通常のスコア順で補充
    //  タイトル類似度は bigram 0.65 で重複除去
    let mut selected: Vec<SelectedTrend> = Vec::new();

    // source 別にキューを作る (スコア降順)
    let mut queues: std::collections::BTreeMap<String, std::collections::VecDeque<SelectedTrend>> = Default::default();
    for cand in normalized.iter().cloned() {
        queues.entry(cand.item.source.clone()).or_default().push_back(cand);
    }

    // ラウンドロビン
    let sources: Vec<String> = queues.keys().cloned().collect();
    let max_rounds = top.max(1) + sources.len();
    for _ in 0..max_rounds {
        if selected.len() >= top { break; }
        for src in &sources {
            if selected.len() >= top { break; }
            if let Some(q) = queues.get_mut(src) {
                while let Some(cand) = q.pop_front() {
                    let dup = selected.iter().any(|s|
                        title_similarity(&s.item.title, &cand.item.title) >= 0.65
                    );
                    if !dup {
                        selected.push(cand);
                        break;
                    }
                }
            }
        }
    }

    // 万一まだ足りない時はスコア順で補充
    if selected.len() < top {
        for cand in normalized {
            if selected.iter().any(|s| s.item.title == cand.item.title) { continue; }
            let dup = selected.iter().any(|s|
                title_similarity(&s.item.title, &cand.item.title) >= 0.65
            );
            if !dup {
                selected.push(cand);
                if selected.len() >= top { break; }
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
