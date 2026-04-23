//! Phase 2 writer パイプライン
//!
//! research → brief → body → 4画像並列 → placeholder 置換 → markdown 保存

use anyhow::{anyhow, Context, Result};
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::ai::{
    anthropic::AnthropicClient, http_client, openai::OpenAiImageClient, pollo::PolloClient,
    xai::GrokClient, ArticleBrief, ArticleDraft, ImageAsset, ResearchResult,
};
use crate::config::Config;
use crate::scoring::SelectedTrend;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrittenArticle {
    pub title: String,
    pub slug: String,
    pub category: String,
    pub tags: Vec<String>,
    pub char_count: usize,
    pub md_path: PathBuf,
    /// 見出し画像 (hero)
    pub image_path: Option<PathBuf>,
    /// 本文挿入用画像パス (最大3枚)
    #[serde(default)]
    pub inline_image_paths: Vec<PathBuf>,
    pub source_url: Option<String>,
}

pub async fn run(cfg: &Config, trends: &[SelectedTrend], out_dir: &Path) -> Result<Vec<WrittenArticle>> {
    if trends.is_empty() {
        return Err(anyhow!("no trends to write"));
    }
    std::fs::create_dir_all(out_dir)?;

    let tasks = trends.iter().enumerate().map(|(i, t)| {
        let out = out_dir.to_path_buf();
        let cfg = cfg.clone();
        let trend = t.clone();
        async move { write_one(&cfg, &trend, &out, i).await }
    });

    let results = join_all(tasks).await;
    let mut written = Vec::new();
    for (i, r) in results.into_iter().enumerate() {
        match r {
            Ok(a) => {
                tracing::info!(slug = %a.slug, chars = a.char_count, "article written");
                written.push(a);
            }
            Err(e) => tracing::error!(index = i, error = %e, "article failed"),
        }
    }
    if written.is_empty() {
        return Err(anyhow!("all articles failed"));
    }

    let manifest_path = out_dir.join("articles.json");
    std::fs::write(&manifest_path, serde_json::to_string_pretty(&written)?)?;
    tracing::info!(path = %manifest_path.display(), "manifest saved");

    Ok(written)
}

async fn write_one(
    cfg: &Config,
    trend: &SelectedTrend,
    out_dir: &Path,
    index: usize,
) -> Result<WrittenArticle> {
    let http = http_client();
    let dry_run = cfg.writer.dry_run;

    // 1. リサーチ
    let research = if dry_run {
        GrokClient::stub(trend)
    } else {
        let key = cfg.trends.xai_api_key.as_deref()
            .ok_or_else(|| anyhow!("XAI_API_KEY required for research (or set writer.dry_run=true)"))?;
        GrokClient::new(&http, key, &cfg.writer.grok_model).research(trend).await
            .with_context(|| format!("grok research failed for {:?}", trend.item.title))?
    };

    // 2. ブリーフ (Haiku) — 20タグ + 4画像プロンプト含む
    let brief = if dry_run {
        stub_brief(trend, index)
    } else {
        let key = cfg.writer.anthropic_api_key.as_deref()
            .ok_or_else(|| anyhow!("ANTHROPIC_API_KEY required (or set writer.dry_run=true)"))?;
        AnthropicClient::new(&http, key, &cfg.writer.opus_model, &cfg.writer.haiku_model, cfg.writer.max_tokens)
            .brief(trend, &research).await
            .with_context(|| "Haiku brief failed")?
    };
    tracing::info!(index, slug = %brief.slug, tags = brief.tags.len(), imgs = brief.image_prompts.len(), "brief done");

    // 3. 本文 (Opus) + 4画像並列生成
    let safe_slug = sanitize_slug(&brief.slug, index);

    let (draft_res, images_res) = tokio::join!(
        async {
            if dry_run {
                Ok::<ArticleDraft, anyhow::Error>(stub_draft(&brief, &research))
            } else {
                let key = cfg.writer.anthropic_api_key.as_deref().unwrap();
                AnthropicClient::new(&http, key, &cfg.writer.opus_model, &cfg.writer.haiku_model, cfg.writer.max_tokens)
                    .write(&brief, &research, trend, cfg.writer.target_chars).await
                    .with_context(|| "Opus write failed")
            }
        },
        generate_images(cfg, &http, &brief, dry_run),
    );
    let draft = draft_res?;
    let images = images_res; // Vec<Option<ImageAsset>> (常に長さ 4)
    tracing::info!(index, chars = draft.char_count, "body done");

    // 4. 画像ファイル保存 + placeholder 置換
    let (image_path, inline_paths, body_with_images) =
        embed_images(out_dir, &safe_slug, &images, &draft.body_markdown)?;

    // 5. front matter + 保存
    let front_matter = format!(
        "---\ntitle: \"{}\"\ncategory: \"{}\"\ntags: [{}]\nslug: \"{}\"\nsource_url: \"{}\"\nchar_count: {}\n{}---\n\n",
        brief.title.replace('"', "'"),
        brief.category,
        brief.tags.iter().map(|t| format!("\"{}\"", t.replace('"', "'"))).collect::<Vec<_>>().join(", "),
        safe_slug,
        trend.item.url.clone().unwrap_or_default(),
        draft.char_count,
        image_path.as_ref()
            .map(|p| format!("image: \"{}\"\n", p.file_name().unwrap().to_string_lossy()))
            .unwrap_or_default(),
    );

    let md_path = out_dir.join(format!("{safe_slug}.md"));
    let full = format!("{front_matter}{body_with_images}\n");
    std::fs::write(&md_path, full)?;

    Ok(WrittenArticle {
        title: brief.title,
        slug: safe_slug,
        category: brief.category,
        tags: brief.tags,
        char_count: draft.char_count,
        md_path,
        image_path,
        inline_image_paths: inline_paths,
        source_url: trend.item.url.clone(),
    })
}

/// 4枚の画像を並列生成。image_prompts が足りない時はテンプレートで補完。
async fn generate_images(
    cfg: &Config,
    http: &reqwest::Client,
    brief: &ArticleBrief,
    dry_run: bool,
) -> Vec<Option<ImageAsset>> {
    if dry_run {
        return vec![None; 4];
    }
    let prompts = ensure_four_prompts(brief);
    let futs: Vec<_> = prompts.iter().enumerate().map(|(i, p)| {
        let cfg = cfg.clone();
        let http = http.clone();
        let prompt = p.clone();
        async move {
            match generate_single_image(&cfg, &http, &prompt).await {
                Ok(img) => {
                    tracing::info!(idx = i, "image ready");
                    Some(img)
                }
                Err(e) => {
                    tracing::warn!(idx = i, error = %e, "image failed, skipping");
                    None
                }
            }
        }
    }).collect();
    join_all(futs).await
}

async fn generate_single_image(
    cfg: &Config,
    http: &reqwest::Client,
    prompt: &str,
) -> anyhow::Result<ImageAsset> {
    match cfg.writer.image_provider.as_str() {
        "pollo" => {
            let key = cfg.writer.pollo_api_key.as_deref()
                .ok_or_else(|| anyhow!("POLLO_API_KEY 未設定"))?;
            PolloClient::new(http, key, &cfg.writer.image_model, &cfg.writer.image_size)
                .with_quality("high")
                .generate_with_prompt(prompt).await
        }
        "openai" => {
            let key = cfg.writer.openai_api_key.as_deref()
                .ok_or_else(|| anyhow!("OPENAI_API_KEY 未設定"))?;
            OpenAiImageClient::new(http, key, &cfg.writer.image_model, &cfg.writer.image_size)
                .generate_prompt(prompt).await
        }
        other => Err(anyhow!("unknown image_provider: {}", other)),
    }
}

/// 画像を保存し、本文中の {{IMAGE_HEADER}} / {{IMAGE_1..3}} を markdown 画像リンクに置換
fn embed_images(
    out_dir: &Path,
    slug: &str,
    images: &[Option<ImageAsset>],
    body_md: &str,
) -> Result<(Option<PathBuf>, Vec<PathBuf>, String)> {
    let mut body = body_md.to_string();
    let mut hero_path: Option<PathBuf> = None;
    let mut inline_paths: Vec<PathBuf> = Vec::new();

    let filenames = [
        ("{{IMAGE_HEADER}}", format!("{slug}-hero.png")),
        ("{{IMAGE_1}}", format!("{slug}-1.png")),
        ("{{IMAGE_2}}", format!("{slug}-2.png")),
        ("{{IMAGE_3}}", format!("{slug}-3.png")),
    ];

    for (i, (placeholder, fname)) in filenames.iter().enumerate() {
        match images.get(i).and_then(|o| o.as_ref()) {
            Some(img) => {
                let path = out_dir.join(fname);
                std::fs::write(&path, &img.png_bytes)?;
                let alt = if i == 0 { "hero" } else { &format!("image {i}") };
                let md_link = format!("![{alt}]({fname})");
                body = body.replace(placeholder, &md_link);
                if i == 0 {
                    hero_path = Some(path);
                } else {
                    inline_paths.push(path);
                }
            }
            None => {
                // 画像生成失敗時はプレースホルダを空白に置き換え
                body = body.replace(placeholder, "");
            }
        }
    }

    Ok((hero_path, inline_paths, body))
}

/// image_prompts が不足している場合の補完 (品質ガード)
fn ensure_four_prompts(brief: &ArticleBrief) -> Vec<String> {
    let base_style = "Photorealistic Japanese setting, Japanese people, detailed skin with subsurface scattering, \
lens flare, ray-traced lighting, shallow depth of field, 16:9 aspect ratio, cinematic portrait, \
vivid but natural color palette, no text, no letters, no logos.";
    let mut out = brief.image_prompts.clone();
    while out.len() < 4 {
        let i = out.len();
        let role = match i {
            0 => format!(
                "Hero image illustrating the article titled '{}'. Category: {}. Keywords: {}. \
Include a tiny elegant bottom-right badge showing 'cityriver.sayasaya.workers.dev' as a subtle watermark. \
{base_style}",
                brief.title, brief.category, brief.tags.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
            ),
            1 => format!(
                "Inline image 1 depicting the intro scene of the article '{}'. A Japanese person pondering, \
natural daytime light. {base_style}",
                brief.title
            ),
            2 => format!(
                "Inline image 2 depicting the main body of the article '{}'. A Japanese professional in action, \
workspace or urban Tokyo setting. {base_style}",
                brief.title
            ),
            _ => format!(
                "Inline image 3 depicting hopeful future from the article '{}'. A Japanese person smiling, \
warm golden-hour light. {base_style}",
                brief.title
            ),
        };
        out.push(role);
    }
    out.truncate(4);
    out
}

fn sanitize_slug(raw: &str, fallback_idx: usize) -> String {
    let s = slug::slugify(raw);
    if s.is_empty() {
        format!("article-{}", fallback_idx + 1)
    } else {
        s.chars().take(60).collect()
    }
}

fn stub_brief(trend: &SelectedTrend, idx: usize) -> ArticleBrief {
    ArticleBrief {
        title: format!("【{}】{}", trend.item.source, trend.item.title),
        slug: format!("stub-{}", idx + 1),
        category: "トレンド".into(),
        outline: vec![
            "概要".into(), "背景".into(), "詳細".into(), "インパクト".into(), "まとめ".into(),
        ],
        tags: (1..=20).map(|n| format!("トレンド{n}")).collect(),
        hook: "[dry-run] これはスタブ記事です".into(),
        image_prompts: vec![],
    }
}

fn stub_draft(brief: &ArticleBrief, research: &ResearchResult) -> ArticleDraft {
    let body = format!(
        "# {}\n\n{{{{IMAGE_HEADER}}}}\n\n{}\n\n{{{{IMAGE_1}}}}\n\n## 概要\n\n{}\n\n{{{{IMAGE_2}}}}\n\n## 詳細\n\n- {}\n\n{{{{IMAGE_3}}}}\n\n## 参考リンク\n\n{}\n\n---\n\n皆様の意見はどうでしょうか？\n良かったらコメントで教えて下さい。\nフォロー＆スキもお願いします♪\n\nこの記事への感想やご質問、お仕事のご依頼など、\nお気軽にメッセージをお送りください♪\n📩メッセージはこちらから\nhttps://note.com/alvis8039/message\n\n{}\n",
        brief.title,
        brief.hook,
        research.summary,
        research.key_facts.join("\n- "),
        research.citations.iter().map(|c| format!("- {}", c)).collect::<Vec<_>>().join("\n"),
        brief.tags.iter().map(|t| format!("#{}", t)).collect::<Vec<_>>().join(" "),
    );
    let char_count = body.chars().count();
    ArticleDraft {
        title: brief.title.clone(),
        body_markdown: body,
        char_count,
    }
}
