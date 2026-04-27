#!/usr/bin/env node
/**
 * コンビニ来週新商品スクレイパー (Playwright サイドカー)
 *
 * 入力: stdin に { "top_n": N } JSON
 * 出力: stdout に [{title, summary, url, score, chain}, ...] JSON
 * stderr にログ
 *
 * 対象:
 *   - セブン-イレブン:    https://www.sej.co.jp/products/a/week_new/
 *   - ローソン:           https://www.lawson.co.jp/recommend/new/
 *   - ファミリーマート:   https://www.family.co.jp/goods.html
 */

import { chromium } from "playwright";

const log = (...a) => console.error("[scrape-konbini]", ...a);

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

async function scrapeSeven(page) {
  const items = [];
  try {
    await page.goto("https://www.sej.co.jp/products/a/week_new/", {
      waitUntil: "networkidle",
      timeout: 30_000,
    });
    await page.waitForTimeout(1500);
    const found = await page.$$eval(
      "a[href*='/products/a/item/'], .item_list li a, .productList a",
      (els) =>
        els
          .map((e) => ({
            title: (e.textContent || "").trim().replace(/\s+/g, " "),
            url: e.href,
          }))
          .filter((x) => x.title && x.title.length > 3 && x.title.length < 120)
          .slice(0, 30),
    );
    for (const f of found) {
      items.push({ ...f, chain: "セブン-イレブン", score: 70 });
    }
  } catch (e) {
    log("seven failed:", e.message);
  }
  return items;
}

async function scrapeLawson(page) {
  const items = [];
  try {
    await page.goto("https://www.lawson.co.jp/recommend/new/", {
      waitUntil: "networkidle",
      timeout: 30_000,
    });
    await page.waitForTimeout(1500);
    const found = await page.$$eval(
      "a[href*='/recommend/new/'], .new-item-list li a, article a",
      (els) =>
        els
          .map((e) => ({
            title: (e.textContent || "").trim().replace(/\s+/g, " "),
            url: e.href,
          }))
          .filter((x) => x.title && x.title.length > 3 && x.title.length < 120)
          .slice(0, 30),
    );
    for (const f of found) {
      items.push({ ...f, chain: "ローソン", score: 70 });
    }
  } catch (e) {
    log("lawson failed:", e.message);
  }
  return items;
}

async function scrapeFamilyMart(page) {
  const items = [];
  try {
    await page.goto("https://www.family.co.jp/goods.html", {
      waitUntil: "networkidle",
      timeout: 30_000,
    });
    await page.waitForTimeout(1500);
    const found = await page.$$eval(
      ".splide__slideItem a, .ly-mnav-side-newproducts a, a[href*='/goods/']",
      (els) =>
        els
          .map((e) => ({
            title: (e.textContent || e.getAttribute("aria-label") || "")
              .trim()
              .replace(/\s+/g, " "),
            url: e.href,
          }))
          .filter((x) => x.title && x.title.length > 3 && x.title.length < 120)
          .slice(0, 30),
    );
    for (const f of found) {
      items.push({ ...f, chain: "ファミリーマート", score: 70 });
    }
  } catch (e) {
    log("familymart failed:", e.message);
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
    all.push(...(await scrapeSeven(page)));
    all.push(...(await scrapeLawson(page)));
    all.push(...(await scrapeFamilyMart(page)));

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
