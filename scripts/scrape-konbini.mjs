#!/usr/bin/env node
/**
 * コンビニ来週新商品スクレイパー (Playwright サイドカー) v2
 *
 * 入力: stdin に { "top_n": N } JSON
 * 出力: stdout に [{title, summary, url, score, chain, image_urls[]}, ...] JSON
 *
 * v4 改良:
 *   - チェーン別比率を強制 (ファミリーマート > ローソン > セブン-イレブン = 5:3:2)
 *   - dedupe キーに chain を含めて、チェーン横断の取りこぼしを防止
 *
 * v2 改良:
 *   - lazy-load 属性を網羅 (data-src / data-lazy-src / data-original / srcset)
 *   - <picture> / <source> 対応
 *   - og:image をページ全体フォールバックに採用
 *   - URL/タイトル下限フィルタ強化 (実商品リンクのみ)
 *   - 画像サイズフィルタ (width/height 属性 <100 は捨てる)
 */

import { chromium } from "playwright";

const log = (...a) => console.error("[scrape-konbini]", ...a);

async function readStdin() {
  const chunks = [];
  for await (const c of process.stdin) chunks.push(c);
  const txt = Buffer.concat(chunks).toString("utf8").trim();
  if (!txt) return {};
  try { return JSON.parse(txt); } catch { return {}; }
}

/**
 * ページから og:image / twitter:image を取得 (ページ全体フォールバック用)
 */
async function getOgImage(page) {
  return await page.evaluate(() => {
    const og = document.querySelector('meta[property="og:image"]')?.content
      || document.querySelector('meta[name="twitter:image"]')?.content
      || document.querySelector('meta[name="og:image"]')?.content;
    return og || null;
  }).catch(() => null);
}

/**
 * カード DOM から画像 URL を抽出 (lazy-load 全パターン対応)
 */
const extractImageScript = `(card) => {
  const candidates = [];
  const imgs = card.querySelectorAll("img, source");
  for (const img of imgs) {
    const w = parseInt(img.getAttribute("width") || "0", 10);
    const h = parseInt(img.getAttribute("height") || "0", 10);
    if ((w > 0 && w < 80) || (h > 0 && h < 80)) continue;  // アイコン除外
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
    // background-image style
    const bg = img.style?.backgroundImage || "";
    const m = bg.match(/url\\(['"]?([^'"\\)]+)['"]?\\)/);
    if (m) {
      try { candidates.push(new URL(m[1], location.origin).href); } catch {}
    }
  }
  return [...new Set(candidates)].slice(0, 3);
}`;

async function scrapeChain(page, name, url, chain, score, anchorSelector, cardSelector, urlPattern) {
  const items = [];
  try {
    log(`fetching ${name}...`);
    await page.goto(url, { waitUntil: "networkidle", timeout: 30_000 });
    await page.waitForTimeout(2000);
    // 遅延ロード対策にスクロール
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
          // urlPattern が指定されてる場合のフィルタ
          if (urlPattern && !new RegExp(urlPattern).test(href)) continue;
          // タイトル品質フィルタ: 5文字以上 + 「日付のみ」「価格のみ」を除外
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
      // 画像が空なら og:image を 1 枚だけセット
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
    // チェーンも含めて重複判定（同名商品でもチェーン違いは別物として扱う）
    const key = `${it.chain || ""}|${(it.title || "").slice(0, 40)}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(it);
  }
  return out;
}

/**
 * チェーン別の取得比率を強制する。
 * ユーザー指定: ファミリーマート > ローソン > セブン-イレブン (= 5 : 3 : 2)
 * 1. チェーン別のキューに振り分け
 * 2. パターン配列で循環ピック → 比率を保ちつつ topN 件選ぶ
 * 3. 空キューに当たったらスキップして次のチェーンから取る
 *    （これにより、あるチェーンが少なくても他のチェーンで埋まる）
 */
function pickWithRatio(items, topN) {
  const order = ["ファミリーマート", "ローソン", "セブン-イレブン"];
  const buckets = Object.fromEntries(order.map((c) => [c, []]));
  for (const it of items) {
    if (buckets[it.chain]) buckets[it.chain].push(it);
  }
  // 5:3:2 の循環パターン
  const pattern = [
    "ファミリーマート", "ファミリーマート", "ファミリーマート", "ファミリーマート", "ファミリーマート",
    "ローソン", "ローソン", "ローソン",
    "セブン-イレブン", "セブン-イレブン",
  ];
  const out = [];
  let i = 0;
  let guard = 0;
  while (out.length < topN && guard < topN * 20) {
    guard++;
    const chain = pattern[i++ % pattern.length];
    const item = buckets[chain].shift();
    if (item) {
      out.push(item);
      continue;
    }
    // 全キュー空なら終了
    if (order.every((c) => buckets[c].length === 0)) break;
  }
  return out;
}

/**
 * JST の曜日に応じて「今週 / 来週」を決定。
 *   - 火〜金 (Tue/Wed/Thu/Fri = 2,3,4,5): 今週の新商品
 *   - 土〜月 (Sat/Sun/Mon = 6,0,1): 来週の新商品
 * セブン-イレブン公式は火曜から新商品が並ぶサイクルなので、
 * 火曜以降は「今週」を見て、土曜以降は次のサイクルの「来週」を先取り。
 * Lawson は本ロジック対象外 (常に固定 URL)。
 */
function pickSevenFamilyMode() {
  const nowMs = Date.now();
  const jstDay = new Date(nowMs + 9 * 60 * 60 * 1000).getUTCDay();
  const isThisWeek = [2, 3, 4, 5].includes(jstDay);
  return {
    isThisWeek,
    label: isThisWeek ? "this-week" : "next-week",
    jstDay,
  };
}

async function main() {
  const input = await readStdin();
  const topN = Math.max(5, input.top_n || 20);
  const mode = pickSevenFamilyMode();
  log(`starting v4 (FM:Lawson:Seven=5:3:2), top_n = ${topN}, jstDay=${mode.jstDay}, mode=${mode.label}`);

  const sevenUrl = mode.isThisWeek
    ? "https://www.sej.co.jp/products/a/thisweek/"
    : "https://www.sej.co.jp/products/a/nextweek/";
  const familymartUrl = mode.isThisWeek
    ? "https://www.family.co.jp/goods/newgoods.html"
    : "https://www.family.co.jp/goods/newgoods/nextweek.html";

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
      `seven(${mode.label})`,
      sevenUrl,
      "セブン-イレブン",
      75,
      "a[href*='/products/a/item/'], .item_list a[href*='/products/'], main a[href*='/products/']",
      "li, .item, article, .productCard, .pbContainer",
      "/products/a/(item|categry|category)",
    )));
    all.push(...(await scrapeChain(
      page,
      "lawson",
      "https://www.lawson.co.jp/recommend/new/",
      "ローソン",
      75,
      "a[href*='/recommend/'][href*='detail'], a[href*='/recommend/goods/'], main article a",
      "li, article, .new-item, .product-item",
      "lawson",
    )));
    all.push(...(await scrapeChain(
      page,
      `familymart(${mode.label})`,
      familymartUrl,
      "ファミリーマート",
      75,
      "a[href*='/goods/']:not([href$='goods.html']):not([href$='newgoods.html']):not([href$='nextweek.html']):not([href$='lastweek.html']), .splide__slideItem a",
      ".splide__slideItem, li, article, .ly-card",
      // FM 実商品 URL は /goods/<category>/<数字>.html の形 — カテゴリトップ /goods/foo.html を弾く
      "/goods/[a-z_]+/\\d+",
    )));

    const deduped = dedupe(all);
    const picked = pickWithRatio(deduped, topN);
    const withImg = picked.filter((i) => i.image_urls.length > 0).length;
    const breakdown = picked.reduce((acc, x) => {
      acc[x.chain] = (acc[x.chain] || 0) + 1;
      return acc;
    }, {});
    log(`collected: ${picked.length} (${withImg} with images), breakdown=${JSON.stringify(breakdown)}`);
    process.stdout.write(JSON.stringify(picked));
  } finally {
    await browser.close();
  }
}

main().catch((e) => {
  log("fatal:", e.message);
  process.stdout.write("[]");
  process.exit(1);
});
