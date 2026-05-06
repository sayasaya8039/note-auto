use crate::trends::TrendItem;
use crate::util::title_similarity;

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
/// 2. ソース別ウェイト適用 (`scoring_cfg.source_weights`)
/// 3. タイトル類似度で重複除去 (`scoring_cfg.dedup_threshold`)
/// 4. 上位 top 件
///
/// L10: `#[tracing::instrument]` で stage 経過時間を自動計測。
#[tracing::instrument(name = "score", skip_all, fields(input = items.len(), top))]
pub fn select_top(
    items: Vec<TrendItem>,
    top: usize,
    scoring_cfg: &crate::config::ScoringConfig,
) -> Vec<SelectedTrend> {
    if items.is_empty() {
        return vec![];
    }

    // L7: ScoringConfig::default() でなく呼び出し側 config を尊重
    //     (旧実装は dedup_threshold を 0.65 ハードコードで無視していた)
    let dedup_th = scoring_cfg.dedup_threshold;

    let mut by_source: std::collections::HashMap<String, Vec<TrendItem>> = Default::default();
    for item in items {
        by_source.entry(item.source.clone()).or_default().push(item);
    }

    let mut normalized: Vec<SelectedTrend> = Vec::new();
    for (src, mut group) in by_source {
        let max = group.iter().map(|t| t.raw_score).fold(0.0_f64, f64::max);
        let weight = scoring_cfg.source_weights.get(&src).copied().unwrap_or(1.0);
        group.sort_by(|a, b| b.raw_score.total_cmp(&a.raw_score));
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

    normalized.sort_by(|a, b| b.composite_score.total_cmp(&a.composite_score));

    // カテゴリ/ソース分散を強制:
    //  1st pass: 各 source から高スコア順に 1 件ずつ ラウンドロビンで拾う
    //  それでも top に満たない場合は通常のスコア順で補充
    //  タイトル類似度は bigram >= dedup_th で重複除去
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
                        title_similarity(&s.item.title, &cand.item.title) >= dedup_th
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
    // L8: 既選 title の HashSet で完全一致チェックを O(N²) → O(N) 化
    if selected.len() < top {
        let mut chosen_titles: std::collections::HashSet<String> =
            selected.iter().map(|s| s.item.title.clone()).collect();
        for cand in normalized {
            if chosen_titles.contains(&cand.item.title) { continue; }
            let dup = selected.iter().any(|s|
                title_similarity(&s.item.title, &cand.item.title) >= dedup_th
            );
            if !dup {
                chosen_titles.insert(cand.item.title.clone());
                selected.push(cand);
                if selected.len() >= top { break; }
            }
        }
    }

    selected
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ScoringConfig;
    use crate::trends::TrendItem;

    fn item(source: &str, title: &str, raw_score: f64) -> TrendItem {
        let mut t = TrendItem::new(source, title);
        t.raw_score = raw_score;
        t
    }

    /// TEST-1 case 1: 入力空 → 出力空
    #[test]
    fn select_top_empty_input() {
        let cfg = ScoringConfig::default();
        let r = select_top(vec![], 5, &cfg);
        assert!(r.is_empty());
    }

    /// TEST-1 case 2: 単一 source、top より多い候補 → 上位 top 件
    /// (注: bigram Jaccard >= 0.65 で dedup されるため、タイトルを十分多様化)
    #[test]
    fn select_top_single_source_truncates_to_top() {
        let cfg = ScoringConfig::default();
        let items = vec![
            item("hn", "Quantum Physics Theory 2026", 100.0),
            item("hn", "AI Robot Future Society", 80.0),
            item("hn", "Cooking Italian Pasta Recipe", 60.0),
            item("hn", "Travel Tokyo Sightseeing", 40.0),
        ];
        let r = select_top(items, 2, &cfg);
        assert_eq!(r.len(), 2);
        // ラウンドロビン上で同 source なら raw_score 順
        assert_eq!(r[0].item.title, "Quantum Physics Theory 2026");
        assert_eq!(r[1].item.title, "AI Robot Future Society");
    }

    /// TEST-1 case 3: 重複タイトル (bigram 類似度 >= dedup_threshold) → 1 件のみ採用
    #[test]
    fn select_top_dedup_similar_titles() {
        let cfg = ScoringConfig::default(); // dedup_threshold = 0.65 (default)
        let items = vec![
            // 完全に同一タイトル → 確実に dedup
            item("hn", "Same Title 12345", 100.0),
            item("google", "Same Title 12345", 50.0),
        ];
        let r = select_top(items, 2, &cfg);
        // dedup により 1 件のみ採用 (高スコア優先)
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].item.title, "Same Title 12345");
    }

    /// TEST-1 case 4: 複数 source ラウンドロビン → 各 source から 1 件ずつ拾う
    #[test]
    fn select_top_round_robin_multi_source() {
        let cfg = ScoringConfig::default();
        let items = vec![
            item("hn", "HN-1", 100.0),
            item("hn", "HN-2", 90.0),
            item("hn", "HN-3", 80.0),
            item("google", "GG-1", 70.0),
            item("google", "GG-2", 60.0),
            item("note", "NT-1", 50.0),
        ];
        let r = select_top(items, 3, &cfg);
        assert_eq!(r.len(), 3);
        // 各 source から最低 1 件ずつ採用されているはず (ラウンドロビン)
        let sources: std::collections::HashSet<&str> =
            r.iter().map(|s| s.item.source.as_str()).collect();
        assert_eq!(sources.len(), 3, "3 source 全部から最低 1 件採用されるべき");
    }

    /// TEST-1 case 5: ラウンドロビンで埋まらず、補充ループで追加 (top > sources × 1)
    /// (タイトル多様化で bigram dedup を回避)
    #[test]
    fn select_top_fallback_fills_remaining() {
        let cfg = ScoringConfig::default();
        let items = vec![
            item("hn", "Quantum Physics Theory", 100.0),
            item("hn", "Cooking Italian Pasta", 90.0),
            item("hn", "Travel Tokyo Spots", 80.0),
            item("google", "Movie Review Action", 70.0),
        ];
        // top=4 だが source は 2、ラウンドロビン後の補充で hn の余りを使う
        let r = select_top(items, 4, &cfg);
        assert_eq!(r.len(), 4);
        // hn 3 件 + google 1 件 が含まれる
        let hn_count = r.iter().filter(|s| s.item.source == "hn").count();
        let gg_count = r.iter().filter(|s| s.item.source == "google").count();
        assert_eq!(hn_count, 3);
        assert_eq!(gg_count, 1);
    }
}
