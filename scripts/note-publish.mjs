/**
 * note.com 自動投稿 Playwright サイドカー (Pure node .mjs 版)
 *
 * bun だと Playwright の remote-debugging-pipe 確立に失敗する環境向け。
 * node v20+ で動作。
 *
 * 初回:
 *   node scripts/note-publish.mjs --login .cookies
 *
 * 本番:
 *   echo '{"md_path":"...", ...}' | node scripts/note-publish.mjs
 */

console.error("[note-publish] script start (node)");

import { chromium } from "playwright";
import { readFileSync, existsSync, mkdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { spawn } from "node:child_process";
import { createConnection } from "node:net";
import { setTimeout as sleep } from "node:timers/promises";

console.error("[note-publish] playwright imported");

async function readStdin() {
  const chunks = [];
  for await (const chunk of process.stdin) chunks.push(chunk);
  return Buffer.concat(chunks).toString("utf8");
}

function parseFrontMatter(md) {
  const m = md.match(/^---\n([\s\S]*?)\n---\n+([\s\S]*)$/);
  if (!m) return { title: "", body: md };
  const body = m[2];
  const titleMatch = m[1].match(/title:\s*"([^"]+)"/);
  return { title: titleMatch?.[1] ?? "", body };
}

/** ポートが LISTEN になるまで待つ (最大 timeoutMs) */
async function waitForPort(port, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const ok = await new Promise((r) => {
      const s = createConnection({ port, host: "127.0.0.1" });
      s.once("connect", () => { s.destroy(); r(true); });
      s.once("error", () => r(false));
    });
    if (ok) return;
    await sleep(200);
  }
  throw new Error(`port ${port} did not open within ${timeoutMs}ms`);
}

/**
 * CDP ポート経由で Chrome/Edge を起動して connectOverCDP で接続。
 * Playwright の managed spawn が remote-debugging-pipe 問題で失敗する Windows 環境向け。
 */
async function launchViaCDP(executablePath, cookieDir, headless) {
  const profileDir = resolve(cookieDir, "browser-profile");
  mkdirSync(profileDir, { recursive: true });
  const port = 9222 + Math.floor(Math.random() * 1000); // ランダムで衝突回避
  // HeadlessChrome/... の UA を note.com の API が CORS 弾きにするので
  // 通常 Chrome の UA に偽装する
  const UA = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36";
  const args = [
    `--user-data-dir=${profileDir}`,
    `--remote-debugging-port=${port}`,
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-features=Translate",
    // headless detection 回避
    "--disable-blink-features=AutomationControlled",
    `--user-agent=${UA}`,
  ];
  if (headless) args.push("--headless=new");
  args.push("about:blank");

  console.error(`[cdp] spawning ${executablePath} on port ${port}...`);
  const child = spawn(executablePath, args, { detached: false, stdio: "ignore", windowsHide: false });
  child.on("exit", (code) => console.error(`[cdp] browser exited code=${code}`));

  await waitForPort(port);
  console.error(`[cdp] port ${port} ready, connecting...`);
  const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
  const ctx = browser.contexts()[0] ?? await browser.newContext();

  // すべての新規ページに webdriver 痕跡を消す init script を注入
  await ctx.addInitScript(() => {
    Object.defineProperty(navigator, "webdriver", { get: () => undefined });
    // @ts-ignore
    if (!window.chrome) window.chrome = { runtime: {} };
    Object.defineProperty(navigator, "languages", { get: () => ["ja-JP", "ja", "en-US", "en"] });
    Object.defineProperty(navigator, "plugins", { get: () => [1, 2, 3, 4, 5] });
  });

  console.error(`[cdp] connected. contexts=${browser.contexts().length}`);
  return { ctx, browser, browserChild: child, label: `cdp:${executablePath.split(/[\\/]/).pop()}` };
}

/** 既知のブラウザパス候補 (Windows) */
function browserCandidates() {
  return [
    "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
    "C:/Program Files/Microsoft/Edge/Application/msedge.exe",
    "C:/Program Files/Google/Chrome/Application/chrome.exe",
    "C:/Program Files (x86)/Google/Chrome/Application/chrome.exe",
  ].filter(existsSync);
}

async function launchContext(cookieDir, headless, persist) {
  const profileDir = resolve(cookieDir, "browser-profile");
  const storageFile = join(cookieDir, "note.json");
  const TIMEOUT_MS = 30_000;

  // 1. CDP 経由 (remote-debugging-pipe 問題を回避)
  for (const exe of browserCandidates()) {
    try {
      return await launchViaCDP(exe, cookieDir, headless);
    } catch (e) {
      const msg = String(e.message).split("\n")[0].slice(0, 200);
      console.error(`[launch] cdp via ${exe} FAILED: ${msg}`);
    }
  }

  // 2. 通常の Playwright launch (pipe 方式) — ほとんどの Windows で失敗するが一応試す
  const tryLaunch = async (label, channel, noSandbox) => {
    console.error(`[launch] trying ${label}...`);
    const baseArgs = [
      "--disable-blink-features=AutomationControlled",
      "--disable-features=IsolateOrigins,site-per-process",
    ];
    const args = noSandbox
      ? [...baseArgs, "--no-sandbox", "--disable-gpu-sandbox", "--disable-setuid-sandbox"]
      : baseArgs;
    if (persist) {
      mkdirSync(profileDir, { recursive: true });
      const opts = { headless, timeout: TIMEOUT_MS };
      if (channel) opts.channel = channel;
      if (args) opts.args = args;
      const ctx = await chromium.launchPersistentContext(profileDir, opts);
      console.error(`[launch] OK: ${label} (persistent)`);
      return { ctx, label };
    } else {
      const opts = { headless, timeout: TIMEOUT_MS };
      if (channel) opts.channel = channel;
      if (args) opts.args = args;
      const browser = await chromium.launch(opts);
      const contextOpts = existsSync(storageFile) ? { storageState: storageFile } : {};
      const ctx = await browser.newContext(contextOpts);
      console.error(`[launch] OK: ${label}`);
      return { ctx, browser, label };
    }
  };

  const candidates = [
    { label: "system msedge", channel: "msedge", noSandbox: false },
    { label: "system chrome", channel: "chrome", noSandbox: false },
    { label: "managed chromium (no-sandbox)", channel: undefined, noSandbox: true },
    { label: "managed chromium (default)",    channel: undefined, noSandbox: false },
  ];
  let lastErr;
  for (const c of candidates) {
    try {
      return await tryLaunch(c.label, c.channel, c.noSandbox);
    } catch (e) {
      const msg = String(e.message).split("\n")[0].slice(0, 200);
      console.error(`[launch] ${c.label} FAILED: ${msg}`);
      lastErr = e;
    }
  }
  throw lastErr instanceof Error ? lastErr : new Error("all launch candidates failed");
}

async function loginFlow(cookieDir) {
  mkdirSync(cookieDir, { recursive: true });
  console.error(`[login] cookie_dir=${resolve(cookieDir)}`);
  const { ctx, browser, browserChild, label } = await launchContext(cookieDir, false, true);
  const page = ctx.pages()[0] ?? await ctx.newPage();
  await page.goto("https://note.com/login");
  console.error(`[login] ${label} 開きました。note.com にログインして、完了したらブラウザを閉じてください...`);

  await new Promise((r) => {
    ctx.on("close", () => r());
    if (browser) browser.on("disconnected", () => r());
    if (browserChild) browserChild.on("exit", () => r());
  });

  try {
    await ctx.storageState({ path: join(cookieDir, "note.json") });
  } catch {}
  console.error("[login] Cookie を保存しました: " + resolve(cookieDir));
}

async function run(input) {
  const cookiePath = join(input.cookie_dir, "note.json");
  const profilePath = join(input.cookie_dir, "browser-profile");
  if (!existsSync(cookiePath) && !existsSync(profilePath)) {
    return { status: "needs_login", error: `${input.cookie_dir} が未初期化。--login で初回ログインしてください` };
  }

  const persist = existsSync(profilePath);
  const { ctx, browser, browserChild } = await launchContext(input.cookie_dir, true, persist);

  try {
    const page = await ctx.newPage();
    await page.goto("https://note.com/notes/new", { waitUntil: "networkidle", timeout: 60000 });
    // React hydrate と自動下書き作成 (→ /edit/ に遷移) を待つ
    await page.waitForURL(/\/edit\//, { timeout: 30000 }).catch(() => {});
    await page.waitForTimeout(3000);

    if (page.url().includes("/login") || page.url().includes("/signin")) {
      return { status: "needs_login", error: "セッション切れ。--login で再ログインしてください" };
    }

    const md = readFileSync(input.md_path, "utf8");
    const { title, body: mdBody } = parseFrontMatter(md);
    const finalTitle = title || input.title;

    // タイトル (textarea[placeholder="記事タイトル"])
    const titleSel = 'textarea[placeholder="記事タイトル"]';
    await page.waitForSelector(titleSel, { timeout: 30000 });
    await page.fill(titleSel, finalTitle);

    // 本文 (ProseMirror エディタ)
    const bodySel = 'div.ProseMirror[contenteditable="true"]';
    const bodyLocator = page.locator(bodySel).first();
    await bodyLocator.click();
    // markdown をそのまま流し込み (ProseMirror は insertText で改行も保持)
    await page.keyboard.insertText(mdBody);
    await page.waitForTimeout(1500); // 自動保存の反映待ち

    // アイキャッチ画像アップロード
    if (input.image_path && existsSync(input.image_path)) {
      try {
        const imageBtn = page.getByRole("button", { name: "画像を追加" });
        await imageBtn.click({ timeout: 5000 });
        const fileChooserPromise = page.waitForEvent("filechooser", { timeout: 5000 });
        const fc = await fileChooserPromise;
        await fc.setFiles(input.image_path);
        await page.waitForTimeout(3000);
      } catch (e) {
        console.error("[publish] image upload skipped:", String(e.message).slice(0, 100));
      }
    }

    if (input.publish) {
      // 「公開に進む」→ 公開設定画面 → 「投稿する」or「公開する」
      await page.getByRole("button", { name: "公開に進む" }).click({ timeout: 10000 });
      await page.waitForTimeout(2000);
      await page.getByRole("button", { name: /(投稿|公開する|確認して公開)/ }).first()
        .click({ timeout: 15000 });
      await page.waitForURL(/note\.com\/[^/]+\/n\//, { timeout: 60000 });
      return { status: "published", url: page.url() };
    } else {
      // 下書き保存ボタンを明示クリック (自動保存だが念のため)
      await page.getByRole("button", { name: "下書き保存" }).click({ timeout: 10000 }).catch(() => {});
      await page.waitForTimeout(2000);
      return { status: "draft", url: page.url() };
    }
  } catch (e) {
    return { status: "error", error: String(e?.message || e) };
  } finally {
    if (browser) await browser.close().catch(() => {});
    else await ctx.close().catch(() => {});
    if (browserChild && !browserChild.killed) browserChild.kill();
  }
}

async function inspectFlow(cookieDir) {
  console.error("[inspect] opening note.com/notes/new (headless)...");
  const persist = existsSync(join(cookieDir, "browser-profile"));
  const { ctx, browser, browserChild } = await launchContext(cookieDir, true, persist);
  try {
    const page = await ctx.newPage();

    // まず note.com ホームで認証状態を確認
    page.on("console", (msg) => console.error(`[page:${msg.type()}]`, msg.text().slice(0, 200)));
    page.on("pageerror", (e) => console.error(`[pageerror]`, String(e).slice(0, 200)));
    page.on("requestfailed", (r) => console.error(`[reqfail]`, r.url().slice(0, 80), r.failure()?.errorText));

    await page.goto("https://note.com/", { waitUntil: "networkidle", timeout: 60000 });
    await page.waitForTimeout(3000);
    const homeInfo = await page.evaluate(() => ({
      url: location.href,
      title: document.title,
      totalNodes: document.querySelectorAll("*").length,
      hasLoginLink: !!document.querySelector('a[href*="/login"]'),
      hasCreateBtn: !!document.querySelector('a[href*="/notes/new"], button[class*="post"]'),
      userAgent: navigator.userAgent,
      webdriver: navigator.webdriver,
    }));
    console.error("[home]", JSON.stringify(homeInfo));

    // 次に新規投稿ページへ
    await page.goto("https://note.com/notes/new", { waitUntil: "networkidle", timeout: 60000 });
    await page.waitForTimeout(12000);
    console.error("[inspect] URL:", page.url());

    const info = await page.evaluate(() => {
      // iframe 内部も覗く
      const docs = [document];
      for (const f of Array.from(document.querySelectorAll("iframe"))) {
        try { if (f.contentDocument) docs.push(f.contentDocument); } catch {}
      }
      // 全ての DOM ノード総数
      const stats = docs.map((d, i) => ({
        doc: i === 0 ? "main" : `iframe#${i - 1}`,
        url: i === 0 ? location.href : (d.location?.href || "?"),
        totalNodes: d.querySelectorAll("*").length,
      }));
      const all = [];
      for (const d of docs) {
        for (const el of d.querySelectorAll("input, textarea, [contenteditable], [role='textbox'], button, [class*='title'], [class*='editor']")) {
          all.push(el);
          if (all.length >= 100) break;
        }
      }
      return {
        stats,
        bodyText: (document.body?.innerText || "").slice(0, 300),
        htmlHead: document.documentElement.outerHTML.slice(0, 500),
        iframes: Array.from(document.querySelectorAll("iframe")).map(f => ({ src: f.src, id: f.id, name: f.name })),
        elements: all.slice(0, 40).map((el) => ({
          tag: el.tagName.toLowerCase(),
          type: el.getAttribute("type") || "",
          placeholder: el.getAttribute("placeholder") || "",
          aria: el.getAttribute("aria-label") || "",
          role: el.getAttribute("role") || "",
          id: el.id || "",
          cls: (el.className || "").toString().slice(0, 60),
          ce: el.getAttribute("contenteditable") || "",
          textHead: (el.textContent || "").trim().slice(0, 30),
        })),
      };
    });
    console.log(JSON.stringify({ url: page.url(), elements: info }, null, 2));
    await page.screenshot({ path: resolve(cookieDir, "inspect.png"), fullPage: true });
    console.error(`[inspect] screenshot saved: ${resolve(cookieDir, "inspect.png")}`);
  } finally {
    if (browser) await browser.close().catch(() => {});
    else await ctx.close().catch(() => {});
    if (browserChild && !browserChild.killed) browserChild.kill();
  }
}

async function main() {
  const args = process.argv.slice(2);
  const loginIdx = args.indexOf("--login");
  if (loginIdx >= 0) {
    const cookieDir = args[loginIdx + 1] ?? ".cookies";
    await loginFlow(cookieDir);
    process.exit(0);
  }
  const inspectIdx = args.indexOf("--inspect");
  if (inspectIdx >= 0) {
    const cookieDir = args[inspectIdx + 1] ?? ".cookies";
    await inspectFlow(cookieDir);
    process.exit(0);
  }

  try {
    const raw = await readStdin();
    const input = JSON.parse(raw);
    const out = await run(input);
    console.log(JSON.stringify(out));
    process.exit(out.status === "error" || out.status === "needs_login" ? 1 : 0);
  } catch (e) {
    console.log(JSON.stringify({ status: "error", error: String(e?.message || e) }));
    process.exit(1);
  }
}

void main();
