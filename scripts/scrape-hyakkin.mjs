#!/usr/bin/env node
/**
 * 100均新商品スクレイパー (Playwright サイドカー) v2
 *
 * 入力: stdin に { "top_n": N } JSON
 * 出力: stdout に [{title, summary, url, score, chain, image_urls[]}, ...] JSON
 *
 * v2 改良: scrape-konbini.mjs と同じ (lazy-load 網羅 / og:image fallback / URL+title フィルタ)
 */

import { chromium } from "playwright";

const log = (...a) => console.error("[scrape-hyakkin]", ...a);

async function readStdin() {
  const chunks = [];
  for await (const c of process.stdin) chunks.push(c);
  const txt = Buffer.concat(chunks).toString("utf8").trim();
  if (!txt) return {};
  try { return JSON.parse(txt); } catch { return {}; }
}

async function getOgImage(page) {
  return await page.evaluate(() => {
    return document.querySelector('meta[property="og:image"]')?.content
      || document.querySelector('meta[name="twitter:image"]')?.content
      || null;
  }).catch(() => null);
}

const extractImageScript = `(card) => {
  const candidates = [];
  const imgs = card.querySelectorAll("img, source");
  for (const img of imgs) {
    const w = parseInt(img.getAttribute("width") || "0", 10);
    const h = parseInt(img.getAttribute("height") || "0", 10);
    if ((w > 0 && w < 80) || (h > 0 && h < 80)) continue;
    for (const attr of ["src", "data-src", "data-lazy-src", "data-original", "data-image", "srcset"]) {
      const raw = img.getAttribute(attr);
      if (!raw) continue;
      const first = raw.split(",")[0].trim().split(" ")[0];
      if (first && !first.startsWith("data:")) {
        try {
          const abs = new URL(first, location.origin).href;
          if (!/\\.(svg|gif)(\\?|$)/i.test(abs) && !/spacer|blank|1x1|loader/i.test(abs)) {
            candidates.push(abs);
          }
        } catch {}
      }
    }
    const bg = img.style?.backgroundImage || "";
    const m = bg.match(/url\\(['"]?([^'"\\)]+)['"]?\\)/);
    if (m) {
      try { candidates.push(new URL(m[1], location.origin).href); } catch {}
    }
  }
  return [...new Set(candidates)].slice(0, 3);
}`;

async function scrapeChain(page, name, url, chain, score, anchorSelector, cardSelector, urlPattern, waitUntil = "networkidle") {
  const items = [];
  try {
    log(`fetching ${name}...`);
    await page.goto(url, { waitUntil, timeout: 30_000 });
    await page.waitForTimeout(2000);
    await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
    await page.waitForTimeout(1500);

    const ogImage = await getOgImage(page);

    const found = await page.$$eval(
      anchorSelector,
      (els, args) => {
        const { cardSelector, urlPattern, extractFnSrc } = args;
        const extract = new Function("return " + extractFnSrc)();
        const out = [];
        const seen = new Set();
        for (const e of els) {
          const card = e.closest(cardSelector) || e;
          const href = e.href || "";
          const titleRaw = (e.textContent || e.getAttribute("aria-label") || "").trim().replace(/\s+/g, " ");
          if (urlPattern && !new RegExp(urlPattern).test(href)) continue;
          if (!titleRaw || titleRaw.length < 5 || titleRaw.length > 150) continue;
          if (/^\d+\/\d+(発売)?$/.test(titleRaw) || /^¥?\d+円$/.test(titleRaw)) continue;
          const key = href || titleRaw;
          if (seen.has(key)) continue;
          seen.add(key);
          out.push({
            title: titleRaw,
            url: href,
            image_urls: extract(card),
          });
          if (out.length >= 20) break;
        }
        return out;
      },
      { cardSelector, urlPattern, extractFnSrc: extractImageScript },
    );

    for (const f of found) {
      if (f.image_urls.length === 0 && ogImage) f.image_urls = [ogImage];
      items.push({ ...f, chain, score });
    }
    log(`${name}: ${items.length} items, ${items.filter((i) => i.image_urls.length > 0).length} with images`);
  } catch (e) {
    log(`${name} failed:`, e.message);
  }
  return items;
}

function dedupe(items) {
  const seen = new Set();
  const out = [];
  for (const it of items) {
    const key = (it.title || "").slice(0, 40);
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(it);
  }
  return out;
}

async function main() {
  const input = await readStdin();
  const topN = Math.max(5, input.top_n || 20);
  log("starting v2, top_n =", topN);

  const browser = await chromium.launch({
    headless: true,
    args: ["--disable-blink-features=AutomationControlled"],
  });
  try {
    const ctx = await browser.newContext({
      userAgent:
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
      locale: "ja-JP",
      viewport: { width: 1280, height: 800 },
    });
    const page = await ctx.newPage();

    const all = [];
    all.push(...(await scrapeChain(
      page,
      "daiso",
      "https://jp.daisojapan.com/",
      "ダイソー",
      70,
      "a[href*='/Page/Item/'], .new-arrival a, .product-item a, main a[href*='product']",
      "li, .product-item, .new-arrival, article",
      "/Page/Item/|product",
    )));
    all.push(...(await scrapeChain(
      page,
      "seria",
      "https://www.seria-group.com/",
      "セリア",
      65,
      ".new_item a, a[href*='item']:not([href*='login']):not([href*='cart']), main a[href*='/products/']",
      "li, .item_box, .new_item, article",
      "item|product",
      "domcontentloaded",
    )));
    all.push(...(await scrapeChain(
      page,
      "cando",
      "https://www.cando-web.co.jp/item/new/",
      "キャンドゥ",
      65,
      ".item-list a[href*='/item/'], a[href*='/item/']:not([href$='/new/'])",
      "li, .item-list, article",
      "/item/",
      "domcontentloaded",
    )));
    all.push(...(await scrapeChain(
      page,
      "watts",
      "https://watts-jp.com/",
      "ワッツ",
      60,
      ".new-item a, .product a, main a[href*='item']",
      "li, .new-item, .product, article",
      "item|product",
      "domcontentloaded",
    )));

    const deduped = dedupe(all).slice(0, topN);
    const withImg = deduped.filter((i) => i.image_urls.length > 0).length;
    log(`collected: ${deduped.length} (${withImg} with images)`);
    process.stdout.write(JSON.stringify(deduped));
  } finally {
    await browser.close();
  }
}

main().catch((e) => {
  log("fatal:", e.message);
  process.stdout.write("[]");
  process.exit(1);
});
