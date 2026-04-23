//! Phase 2 writer パイプライン
//!
//! research → brief → body → image → markdown保存

use anyhow::{anyhow, Context, Result};
use futures::future::join_all;
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::ai::{
    anthropic::AnthropicClient, http_client, openai::OpenAiImageClient, xai::GrokClient,
    ArticleBrief, ArticleDraft, ImageAsset, ResearchResult,
};
use crate::config::Config;
use crate::scoring::SelectedTrend;

#[derive(Debug, Serialize)]
pub struct WrittenArticle {
    pub title: String,
    pub slug: String,
    pub category: String,
    pub tags: Vec<String>,
    pub char_count: usize,
    pub md_path: PathBuf,
    pub image_path: Option<PathBuf>,
    pub source_url: Option<String>,
}

pub async fn run(cfg: &Config, trends: &[SelectedTrend], out_dir: &Path) -> Result<Vec<WrittenArticle>> {
    if trends.is_empty() {
        return Err(anyhow!("no trends to write"));
    }

    std::fs::create_dir_all(out_dir)?;

    // 記事ごとに並行処理 (API 側のレート制限に注意)
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

    // manifest 保存
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
    tracing::debug!(index, "research done");

    // 2. ブリーフ (Haiku)
    let brief = if dry_run {
        stub_brief(trend, index)
    } else {
        let key = cfg.writer.anthropic_api_key.as_deref()
            .ok_or_else(|| anyhow!("ANTHROPIC_API_KEY required (or set writer.dry_run=true)"))?;
        AnthropicClient::new(&http, key, &cfg.writer.opus_model, &cfg.writer.haiku_model, cfg.writer.max_tokens)
            .brief(trend, &research).await
            .with_context(|| "Haiku brief failed")?
    };
    tracing::debug!(index, slug = %brief.slug, "brief done");

    // 3. 本文 (Opus)
    let draft = if dry_run {
        stub_draft(&brief, &research)
    } else {
        let key = cfg.writer.anthropic_api_key.as_deref().unwrap();
        AnthropicClient::new(&http, key, &cfg.writer.opus_model, &cfg.writer.haiku_model, cfg.writer.max_tokens)
            .write(&brief, &research, trend, cfg.writer.target_chars).await
            .with_context(|| "Opus write failed")?
    };
    tracing::debug!(index, chars = draft.char_count, "body done");

    // 4. 画像 (OpenAI) — エラーは warn して継続
    let image: Option<ImageAsset> = if dry_run {
        None
    } else if let Some(key) = cfg.writer.openai_api_key.as_deref() {
        let client = OpenAiImageClient::new(&http, key, &cfg.writer.image_model, &cfg.writer.image_size);
        match client.generate(&brief).await {
            Ok(img) => Some(img),
            Err(e) => {
                tracing::warn!(error = %e, "image generation failed, continuing without image");
                None
            }
        }
    } else {
        None
    };

    // 5. 保存
    let safe_slug = sanitize_slug(&brief.slug, index);
    let md_path = out_dir.join(format!("{safe_slug}.md"));
    let image_path = image.as_ref().map(|_| out_dir.join(format!("{safe_slug}.png")));

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

    let full = format!("{front_matter}{}\n", draft.body_markdown);
    std::fs::write(&md_path, full)?;

    if let (Some(img), Some(path)) = (image.as_ref(), image_path.as_ref()) {
        std::fs::write(path, &img.png_bytes)?;
    }

    Ok(WrittenArticle {
        title: brief.title,
        slug: safe_slug,
        category: brief.category,
        tags: brief.tags,
        char_count: draft.char_count,
        md_path,
        image_path,
        source_url: trend.item.url.clone(),
    })
}

fn sanitize_slug(raw: &str, fallback_idx: usize) -> String {
    let s = slug::slugify(raw);
    if s.is_empty() {
        format!("article-{}", fallback_idx + 1)
    } else {
        // note は URL 長めでも OK だが 60 char 程度に制限
        s.chars().take(60).collect()
    }
}

fn stub_brief(trend: &SelectedTrend, idx: usize) -> ArticleBrief {
    ArticleBrief {
        title: format!("【{}】{}", trend.item.source, trend.item.title),
        slug: format!("stub-{}", idx + 1),
        category: "トレンド".into(),
        outline: vec![
            "概要".into(),
            "背景".into(),
            "詳細".into(),
            "インパクト".into(),
            "まとめ".into(),
        ],
        tags: vec!["トレンド".into(), trend.item.source.clone()],
        hook: "[dry-run] これはスタブ記事です".into(),
    }
}

fn stub_draft(brief: &ArticleBrief, research: &ResearchResult) -> ArticleDraft {
    let body = format!(
        "# {}\n\n{}\n\n## 概要\n\n{}\n\n## 詳細\n\n- {}\n\n## 出典\n\n{}\n\n#{}\n",
        brief.title,
        brief.hook,
        research.summary,
        research.key_facts.join("\n- "),
        research.citations.iter().map(|c| format!("- {}", c)).collect::<Vec<_>>().join("\n"),
        brief.tags.join(" #"),
    );
    let char_count = body.chars().count();
    ArticleDraft {
        title: brief.title.clone(),
        body_markdown: body,
        char_count,
    }
}
