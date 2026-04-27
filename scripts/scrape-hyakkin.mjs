#!/usr/bin/env node
/**
 * 100均新商品スクレイパー (Playwright サイドカー)
 *
 * 入力: stdin に { "top_n": N } JSON
 * 出力: stdout に [{title, summary, url, score, chain}, ...] JSON
 * stderr にログ
 *
 * 対象:
 *   - ダイソー:    https://jp.daisojapan.com/
 *   - セリア:      https://www.seria-group.com/
 *   - キャンドゥ:  https://www.cando-web.co.jp/item/new/
 *   - ワッツ:      https://watts-jp.com/
 */

import { chromium } from "playwright";

const log = (...a) => console.error("[scrape-hyakkin]", ...a);

async function readStdin() {
  const chunks = [];
  for await (const c of process.stdin) chunks.push(c);
  const txt = Buffer.concat(chunks).toString("utf8").trim();
  if (!txt) return {};
  try {
    return JSON.parse(txt);
  } catch {
    return {};
  }
}

async function scrapeDaiso(page) {
  const items = [];
  try {
    await page.goto("https://jp.daisojapan.com/", {
      waitUntil: "networkidle",
      timeout: 30_000,
    });
    await page.waitForTimeout(1500);
    const found = await page.$$eval(
      "a[href*='/Page/Item/'], .new-arrival a, .product-item a",
      (els) =>
        els
          .map((e) => {
            const card = e.closest("li, .product-item, .new-arrival, article") || e;
            const img = card.querySelector("img");
            const src = img?.getAttribute("src") || img?.getAttribute("data-src") || "";
            const absSrc = src ? new URL(src, location.origin).href : "";
            return {
              title: (e.textContent || e.getAttribute("aria-label") || "")
                .trim()
                .replace(/\s+/g, " "),
              url: e.href,
              image_urls: absSrc ? [absSrc] : [],
            };
          })
          .filter((x) => x.title && x.title.length > 3 && x.title.length < 120)
          .slice(0, 30),
    );
    for (const f of found) {
      items.push({ ...f, chain: "ダイソー", score: 70 });
    }
  } catch (e) {
    log("daiso failed:", e.message);
  }
  return items;
}

async function scrapeSeria(page) {
  const items = [];
  try {
    await page.goto("https://www.seria-group.com/", {
      waitUntil: "domcontentloaded",
      timeout: 30_000,
    });
    await page.waitForTimeout(2000);
    const found = await page.$$eval(
      ".new_item a, .item_box a, a[href*='item']",
      (els) =>
        els
          .map((e) => {
            const card = e.closest("li, .item_box, .item-list, article") || e;
            const img = card.querySelector("img");
            const src = img?.getAttribute("src") || img?.getAttribute("data-src") || "";
            const absSrc = src ? new URL(src, location.origin).href : "";
            return {
              title: (e.textContent || "").trim().replace(/\s+/g, " "),
              url: e.href,
              image_urls: absSrc ? [absSrc] : [],
            };
          })
          .filter((x) => x.title && x.title.length > 3 && x.title.length < 120)
          .slice(0, 30),
    );
    for (const f of found) {
      items.push({ ...f, chain: "セリア", score: 65 });
    }
  } catch (e) {
    log("seria failed:", e.message);
  }
  return items;
}

async function scrapeCando(page) {
  const items = [];
  try {
    await page.goto("https://www.cando-web.co.jp/item/new/", {
      waitUntil: "domcontentloaded",
      timeout: 30_000,
    });
    await page.waitForTimeout(2000);
    const found = await page.$$eval(
      ".item-list a, .new-item a, a[href*='/item/']",
      (els) =>
        els
          .map((e) => {
            const card = e.closest("li, .item_box, .item-list, article") || e;
            const img = card.querySelector("img");
            const src = img?.getAttribute("src") || img?.getAttribute("data-src") || "";
            const absSrc = src ? new URL(src, location.origin).href : "";
            return {
              title: (e.textContent || "").trim().replace(/\s+/g, " "),
              url: e.href,
              image_urls: absSrc ? [absSrc] : [],
            };
          })
          .filter((x) => x.title && x.title.length > 3 && x.title.length < 120)
          .slice(0, 30),
    );
    for (const f of found) {
      items.push({ ...f, chain: "キャンドゥ", score: 65 });
    }
  } catch (e) {
    log("cando failed:", e.message);
  }
  return items;
}

async function scrapeWatts(page) {
  const items = [];
  try {
    await page.goto("https://watts-jp.com/", {
      waitUntil: "domcontentloaded",
      timeout: 30_000,
    });
    await page.waitForTimeout(2000);
    const found = await page.$$eval(
      ".new-item a, .product a, a[href*='item']",
      (els) =>
        els
          .map((e) => {
            const card = e.closest("li, .item_box, .item-list, article") || e;
            const img = card.querySelector("img");
            const src = img?.getAttribute("src") || img?.getAttribute("data-src") || "";
            const absSrc = src ? new URL(src, location.origin).href : "";
            return {
              title: (e.textContent || "").trim().replace(/\s+/g, " "),
              url: e.href,
              image_urls: absSrc ? [absSrc] : [],
            };
          })
          .filter((x) => x.title && x.title.length > 3 && x.title.length < 120)
          .slice(0, 30),
    );
    for (const f of found) {
      items.push({ ...f, chain: "ワッツ", score: 60 });
    }
  } catch (e) {
    log("watts failed:", e.message);
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
  log("starting, top_n =", topN);

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
    all.push(...(await scrapeDaiso(page)));
    all.push(...(await scrapeSeria(page)));
    all.push(...(await scrapeCando(page)));
    all.push(...(await scrapeWatts(page)));

    const deduped = dedupe(all).slice(0, topN);
    log("collected:", deduped.length);
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
