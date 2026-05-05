//! AF3 (Phase 3 v0.9.0): JSON parse の bench harness
//!
//! 将来の simd-json 採否判断の **baseline**。anthropic / xai / openai 等の
//! AI レスポンスに近い形状の JSON を `serde_json::from_str` でパースする
//! 速度を計測する。
//!
//! Phase 3 ベンチ調査 (reports/lowlevel-phase3-bench.md) では、
//! 公開 bench + 実 size 推定で simd-json の改善幅を 0.003-0.005% (ノイズレベル)
//! と判断し却下した。ただし将来 AI レスポンスサイズが大きく変化した場合や、
//! parse が支配的なホットパスが出現した場合に再計測できるよう baseline を残す。
//!
//! ## bench 一覧
//! - `serde_json_parse_anthropic_30kb`: anthropic body レスポンス相当 (30KB)
//! - `serde_json_parse_xai_10kb`:        xai research レスポンス相当 (10KB)
//! - `serde_json_parse_small_5kb`:       小型レスポンス (image gen 結果等、5KB)

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use serde::Deserialize;

/// anthropic Messages API の content blocks に近い構造
#[derive(Deserialize)]
#[allow(dead_code)]
struct AnthropicLikeResp {
    id: Option<String>,
    content: Vec<Block>,
}
#[derive(Deserialize)]
struct Block {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    kind: String,
    #[allow(dead_code)]
    text: Option<String>,
}

/// xAI Grok chat completions の choices 配列
#[derive(Deserialize)]
struct XaiLikeResp {
    #[allow(dead_code)]
    choices: Vec<Choice>,
    #[serde(default)]
    #[allow(dead_code)]
    citations: Vec<String>,
}
#[derive(Deserialize)]
struct Choice {
    #[allow(dead_code)]
    message: Msg,
}
#[derive(Deserialize)]
struct Msg {
    #[allow(dead_code)]
    content: String,
}

/// 指定サイズの content text を持つ anthropic-like JSON を構築
fn build_anthropic_json(text_chars: usize) -> String {
    let body: String = "あ".repeat(text_chars);
    serde_json::json!({
        "id": "msg_test_bench",
        "type": "message",
        "role": "assistant",
        "content": [
            { "type": "text", "text": body }
        ]
    })
    .to_string()
}

/// 指定サイズの content を持つ xAI-like JSON を構築
fn build_xai_json(content_chars: usize) -> String {
    let body: String = "test ".repeat(content_chars / 5);
    serde_json::json!({
        "choices": [
            { "message": { "role": "assistant", "content": body } }
        ],
        "citations": ["https://example.com/a", "https://example.com/b"]
    })
    .to_string()
}

fn bench_serde_json_parse_anthropic_30kb(c: &mut Criterion) {
    let json = build_anthropic_json(10_000); // 約 30KB (3 bytes/char × 10k)
    c.bench_function("serde_json_parse_anthropic_30kb", |bencher| {
        bencher.iter(|| {
            let _: AnthropicLikeResp =
                serde_json::from_str(black_box(&json)).unwrap();
        })
    });
}

fn bench_serde_json_parse_xai_10kb(c: &mut Criterion) {
    let json = build_xai_json(10_000);
    c.bench_function("serde_json_parse_xai_10kb", |bencher| {
        bencher.iter(|| {
            let _: XaiLikeResp = serde_json::from_str(black_box(&json)).unwrap();
        })
    });
}

fn bench_serde_json_parse_small_5kb(c: &mut Criterion) {
    let json = build_anthropic_json(1_500); // 約 5KB
    c.bench_function("serde_json_parse_small_5kb", |bencher| {
        bencher.iter(|| {
            let _: AnthropicLikeResp =
                serde_json::from_str(black_box(&json)).unwrap();
        })
    });
}

criterion_group!(
    benches,
    bench_serde_json_parse_anthropic_30kb,
    bench_serde_json_parse_xai_10kb,
    bench_serde_json_parse_small_5kb
);
criterion_main!(benches);
