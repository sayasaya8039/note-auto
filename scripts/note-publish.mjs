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
import { marked } from "marked";

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
    const { title, body: mdBodyRaw } = parseFrontMatter(md);
    const finalTitle = title || input.title;
    // 本文先頭の `# タイトル` は note のタイトル欄と重複するため除去
    const mdBody = mdBodyRaw.replace(/^\s*#\s+.+?\n+/, "");

    // タイトル (textarea[placeholder="記事タイトル"])
    const titleSel = 'textarea[placeholder="記事タイトル"]';
    await page.waitForSelector(titleSel, { timeout: 30000 });
    await page.fill(titleSel, finalTitle);

    // 本文 (ProseMirror エディタ)
    // markdown を HTML に変換し、DataTransfer 経由で paste イベントを dispatch する
    const bodySel = 'div.ProseMirror[contenteditable="true"]';
    await page.waitForSelector(bodySel, { timeout: 30000 });
    const bodyLocator = page.locator(bodySel).first();
    await bodyLocator.click();
    await page.waitForTimeout(500);

    // 画像リンクを処理:
    //  - Hero (image_path と同じファイル名) は body から完全除去。別途 modal で upload。
    //  - Inline は markdown → ユニークマーカー (`NOTEIMAGESLOTX`) に置換して paste 後差し替え。
    // マーカーは ProseMirror の入力ルール (`__`, `**` 等) を避けてハイフン/大文字のみ。
    let heroPath = null;
    const inlineUploads = [];
    let slotIdx = 0;
    const heroFilename = input.image_path ? input.image_path.split(/[\\/]/).pop() : null;

    const bodyForEditor = mdBody.replace(/!\[[^\]]*\]\(([^)]+)\)/g, (_m, src) => {
      const filename = src.split(/[\\/]/).pop();
      if (heroFilename && filename === heroFilename) {
        heroPath = input.image_path;
        return ""; // body から完全除去
      }
      const inlineByName = (input.inline_image_paths || [])
        .find((p) => p.split(/[\\/]/).pop() === filename);
      if (!inlineByName || !existsSync(inlineByName)) return "";
      const labels = ["ALPHA", "BETA", "GAMMA", "DELTA", "EPSILON", "ZETA"];
      const marker = `NOTEIMAGESLOT${labels[slotIdx] ?? `X${slotIdx}`}`;
      inlineUploads.push({ marker, path: inlineByName });
      slotIdx++;
      return `\n\n${marker}\n\n`;
    });
    // 連続する改行を 2 つに圧縮 (hero 除去で空行が増えるのを防止)
    const bodyTrimmed = bodyForEditor.replace(/\n{3,}/g, "\n\n").replace(/^\s+/, "");
    const html = marked.parse(bodyTrimmed, { breaks: true, gfm: true });

    await page.evaluate(({ sel, html }) => {
      const el = document.querySelectorAll(sel);
      const editor = el[el.length - 1];
      if (!editor) throw new Error("editor not found");
      editor.focus();
      const dt = new DataTransfer();
      dt.setData("text/html", html);
      dt.setData("text/plain", html.replace(/<[^>]+>/g, ""));
      const ev = new ClipboardEvent("paste", {
        clipboardData: dt,
        bubbles: true,
        cancelable: true,
      });
      editor.dispatchEvent(ev);
    }, { sel: bodySel, html });
    await page.waitForTimeout(3000);

    // Hero: 画像を追加 ボタン → filechooser
    if (heroPath && existsSync(heroPath)) {
      try {
        console.error(`[publish] hero uploading: ${heroPath}`);
        const heroBtn = page.getByRole("button", { name: "画像を追加" }).first();
        await heroBtn.click({ timeout: 5000 });
        await page.waitForTimeout(1800);
        const fcPromise = page.waitForEvent("filechooser", { timeout: 8000 });
        const uploadBtn = page.getByRole("button", { name: /画像をアップロード/ }).first();
        await uploadBtn.click({ timeout: 5000 });
        try {
          const fc = await fcPromise;
          await fc.setFiles(heroPath);
          console.error("[publish] hero filechooser ok");
        } catch {
          console.error("[publish] hero filechooser timeout, trying input scan");
          await setFileInAnyInput(page, heroPath);
        }
        await page.waitForTimeout(10000);
        const confirmBtn = page.getByRole("button", { name: /(保存|OK|決定|確定|完了)/ }).first();
        await confirmBtn.click({ timeout: 3000 }).catch(() => {});
        await page.keyboard.press("Escape").catch(() => {});
        await page.waitForTimeout(1500);
      } catch (e) {
        console.error(`[publish] hero upload failed:`, String(e.message).slice(0, 150));
      }
    }

    // Inline マーカーを順次画像に差し替え
    for (const { marker, path } of inlineUploads) {
      try {
        console.error(`[publish] inserting inline image at ${marker}: ${path}`);
        // マーカーを含む段落の全体を選択して削除する (空 <p> を残さないため)
        const found = await page.evaluate((marker) => {
          const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
          let node;
          while ((node = walker.nextNode())) {
            if (node.textContent && node.textContent.includes(marker)) {
              // マーカーを含む最も近い block element (<p>, <div>) を対象に
              let block = node.parentElement;
              while (block && !["P", "DIV", "LI", "BLOCKQUOTE", "H1", "H2", "H3", "H4"].includes(block.tagName)) {
                block = block.parentElement;
              }
              const target = block || node.parentElement;
              // 段落全体を選択 (含まれる改行も取り除けるよう、前後の空段落もクリーンアップ用に記録)
              const range = document.createRange();
              range.selectNodeContents(target);
              const sel = window.getSelection();
              sel.removeAllRanges();
              sel.addRange(range);
              return true;
            }
          }
          return false;
        }, marker);
        if (!found) {
          console.error(`[publish] marker ${marker} not found, skipping`);
          continue;
        }
        // 選択範囲(段落内容)を削除 → 空段落になる
        await page.keyboard.press("Delete");
        await page.waitForTimeout(150);
        // Backspace で前段落に merge (空段落を消す)
        await page.keyboard.press("Backspace");
        await page.waitForTimeout(150);

        // ProseMirror エディタに image File を paste event で投入
        const b64 = readFileSync(path).toString("base64");
        await page.evaluate(async ({ b64, sel }) => {
          const all = document.querySelectorAll(sel);
          const editor = all[all.length - 1];
          if (!editor) throw new Error("editor not found for paste");
          editor.focus();
          const byteStr = atob(b64);
          const bytes = new Uint8Array(byteStr.length);
          for (let j = 0; j < byteStr.length; j++) bytes[j] = byteStr.charCodeAt(j);
          const blob = new Blob([bytes], { type: "image/png" });
          const file = new File([blob], "image.png", { type: "image/png" });
          const dt = new DataTransfer();
          dt.items.add(file);
          const ev = new ClipboardEvent("paste", {
            clipboardData: dt,
            bubbles: true,
            cancelable: true,
          });
          editor.dispatchEvent(ev);
        }, { b64, sel: bodySel });
        await page.waitForTimeout(8000);
      } catch (e) {
        console.error(`[publish] insert failed at ${marker}:`, String(e.message).slice(0, 150));
      }
    }
    await page.waitForTimeout(3000);

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

/** ページ内のどこかにある <input type="file"> に setInputFiles する */
async function setFileInAnyInput(page, path) {
  const fileInput = page.locator('input[type="file"]').last();
  const count = await fileInput.count();
  if (count === 0) return false;
  await fileInput.setInputFiles(path).catch(() => {});
  return true;
}

/** ターゲット要素に drop イベントを発火 (drag&drop emulation) */
async function dropFileOnEditor(page, path, sel) {
  const b64 = readFileSync(path).toString("base64");
  await page.evaluate(async ({ b64, sel }) => {
    const el = document.querySelector(sel);
    if (!el) return;
    const byteStr = atob(b64);
    const bytes = new Uint8Array(byteStr.length);
    for (let i = 0; i < byteStr.length; i++) bytes[i] = byteStr.charCodeAt(i);
    const blob = new Blob([bytes], { type: "image/png" });
    const file = new File([blob], "image.png", { type: "image/png" });
    const dt = new DataTransfer();
    dt.items.add(file);
    for (const name of ["dragenter", "dragover", "drop"]) {
      const ev = new DragEvent(name, { dataTransfer: dt, bubbles: true, cancelable: true });
      el.dispatchEvent(ev);
    }
  }, { b64, sel });
}

async function inspectAfterClickFlow(cookieDir) {
  console.error("[inspect-click] opening note editor and clicking 画像を追加...");
  const persist = existsSync(join(cookieDir, "browser-profile"));
  const { ctx, browser, browserChild } = await launchContext(cookieDir, true, persist);
  try {
    const page = await ctx.newPage();
    page.on("filechooser", (fc) => console.error("[filechooser event fired!]", fc.element().toString()));
    await page.goto("https://note.com/notes/new", { waitUntil: "networkidle", timeout: 60000 });
    await page.waitForURL(/\/edit\//, { timeout: 30000 }).catch(() => {});
    await page.waitForTimeout(8000);
    // Click body first to make toolbar appear
    await page.locator('div.ProseMirror[contenteditable="true"]').first().click();
    await page.waitForTimeout(1000);
    // Before click
    console.error("=== BEFORE CLICK ===");
    const before = await page.evaluate(() => ({
      totalNodes: document.querySelectorAll("*").length,
      fileInputs: document.querySelectorAll('input[type="file"]').length,
    }));
    console.error(JSON.stringify(before));
    // Click
    await page.getByRole("button", { name: "画像を追加" }).first().click({ timeout: 5000 });
    await page.waitForTimeout(2000);
    console.error("=== AFTER CLICK ===");
    const after = await page.evaluate(() => {
      const fileInputs = Array.from(document.querySelectorAll('input[type="file"]'));
      return {
        totalNodes: document.querySelectorAll("*").length,
        fileInputCount: fileInputs.length,
        fileInputs: fileInputs.map((f) => ({
          accept: f.accept, name: f.name, id: f.id,
          hidden: f.hidden, styleDisplay: getComputedStyle(f).display,
          parent: f.parentElement?.className?.slice(0, 100) || "",
        })),
        buttons: Array.from(document.querySelectorAll("button")).slice(0, 15).map(b => ({
          aria: b.getAttribute("aria-label") || "",
          text: (b.textContent || "").trim().slice(0, 40),
          cls: b.className.slice(0, 40),
        })),
        dialogs: Array.from(document.querySelectorAll('[role="dialog"], .modal, [class*="modal"], [class*="Modal"]')).slice(0, 5).map(d => ({
          role: d.getAttribute("role") || "",
          cls: d.className.slice(0, 80),
          textHead: (d.textContent || "").trim().slice(0, 100),
        })),
      };
    });
    console.log(JSON.stringify(after, null, 2));
    await page.screenshot({ path: resolve(cookieDir, "inspect-click.png"), fullPage: true });
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
  const inspectClickIdx = args.indexOf("--inspect-click");
  if (inspectClickIdx >= 0) {
    const cookieDir = args[inspectClickIdx + 1] ?? ".cookies";
    await inspectAfterClickFlow(cookieDir);
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
