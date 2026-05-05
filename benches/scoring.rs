//! AF3 (Phase 3 v0.9.0): scoring 周辺の bench harness
//!
//! note-auto の `util::title_similarity` (bigram Jaccard) と関連 helpers の
//! 計測 baseline。将来の perf 改善 (e.g. bigram キャッシュ、SmallVec 化等) の
//! 採否判断材料として利用する。
//!
//! ## bench 一覧
//! - `title_similarity_short`: 短いタイトル (~30 chars) のペア比較
//! - `title_similarity_long`:  長いタイトル (~80 chars) のペア比較
//! - `bigrams_construction`:   bigram HashSet 構築単体のコスト
//!
//! ## 注意
//! crate を library として expose していない (binary のみ) ため、対象関数を
//! bench 内に再実装する duplicated approach を採用。本 PR の scope は
//! "計測基盤の確立" であり、本実装の関数を直接呼ぶ refactor (lib 化) は
//! 別 PR (将来) で扱う。

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::collections::HashSet;
use unicode_segmentation::UnicodeSegmentation;

/// `src/util.rs::bigrams` と等価な実装 (private 関数のため再実装)
fn bigrams(s: &str) -> HashSet<String> {
    let lower = s.to_lowercase();
    let gs: Vec<&str> = lower.graphemes(true).collect();
    if gs.len() < 2 {
        return std::iter::once(lower).collect();
    }
    gs.windows(2).map(|w| w.concat()).collect()
}

/// `src/util.rs::title_similarity` と等価な実装 (private 関数のため再実装)
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

fn bench_title_similarity_short(c: &mut Criterion) {
    let a = "AI ロボットが note 記事を自動生成する未来";
    let b = "ロボット AI が記事を作る note の自動化最前線";
    c.bench_function("title_similarity_short", |bencher| {
        bencher.iter(|| title_similarity(black_box(a), black_box(b)))
    });
}

fn bench_title_similarity_long(c: &mut Criterion) {
    let a = "AI ロボットが note 記事を完全自動生成する未来 — 2026 年版生成 AI 最前線レポート";
    let b = "生成 AI 時代の note 記事 — ロボットが書く時代の到来と人間ライターの新しい役割について";
    c.bench_function("title_similarity_long", |bencher| {
        bencher.iter(|| title_similarity(black_box(a), black_box(b)))
    });
}

fn bench_bigrams_construction(c: &mut Criterion) {
    let s = "AI ロボットが note 記事を完全自動生成する未来 2026 年版";
    c.bench_function("bigrams_construction", |bencher| {
        bencher.iter(|| bigrams(black_box(s)))
    });
}

criterion_group!(
    benches,
    bench_title_similarity_short,
    bench_title_similarity_long,
    bench_bigrams_construction
);
criterion_main!(benches);
