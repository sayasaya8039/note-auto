/**
 * note.com 自動投稿 Playwright サイドカー
 *
 * 入力: stdin から JSON
 *   { md_path, image_path?, title, tags[], publish, cookie_dir }
 *
 * 出力: stdout の最終行に JSON
 *   { status: "published" | "draft" | "needs_login" | "error", url?, error? }
 *
 * Cookie: <cookie_dir>/note.json (storageState) または
 *         <cookie_dir>/browser-profile/ (persistentContext)
 *
 * 初回セットアップ:
 *   bun scripts/note-publish.ts --login [cookie_dir]
 */

console.error("[note-publish] script start");

import { chromium, type BrowserContext, type Browser } from "playwright";
import { readFileSync, existsSync, mkdirSync } from "node:fs";
import { join, resolve } from "node:path";

console.error("[note-publish] playwright imported");

type Input = {
  md_path: string;
  image_path?: string;
  title: string;
  tags: string[];
  publish: boolean;
  cookie_dir: string;
};

type Output = {
  status: "published" | "draft" | "needs_login" | "error";
  url?: string;
  error?: string;
};

async function readStdin(): Promise<string> {
  const chunks: Buffer[] = [];
  for await (const chunk of process.stdin) chunks.push(chunk as Buffer);
  return Buffer.concat(chunks).toString("utf8");
}

function parseFrontMatter(md: string): { title: string; body: string } {
  const m = md.match(/^---\n([\s\S]*?)\n---\n+([\s\S]*)$/);
  if (!m) return { title: "", body: md };
  const body = m[2];
  const titleMatch = m[1].match(/title:\s*"([^"]+)"/);
  return { title: titleMatch?.[1] ?? "", body };
}

/** launch 候補を順に試す。各 launch は TIMEOUT_MS 以内に成功しなければ次へ。 */
async function launchContext(cookieDir: string, headless: boolean, persist: boolean):
  Promise<{ ctx: BrowserContext; browser?: Browser; label: string }>
{
  const profileDir = resolve(cookieDir, "browser-profile");
  const storageFile = join(cookieDir, "note.json");
  const TIMEOUT_MS = 30_000;

  const tryLaunch = async (label: string, channel?: string, noSandbox = false) => {
    console.error(`[launch] trying ${label}...`);
    const args = noSandbox ? ["--no-sandbox", "--disable-gpu-sandbox", "--disable-setuid-sandbox"] : undefined;
    if (persist) {
      // persistent context: 同じ profile dir を再利用 (Edge/Chrome が Windows で安定)
      mkdirSync(profileDir, { recursive: true });
      const ctx = await chromium.launchPersistentContext(profileDir, {
        headless,
        channel,
        args,
        timeout: TIMEOUT_MS,
      });
      console.error(`[launch] OK: ${label} (persistent)`);
      return { ctx, label };
    } else {
      const opts: Parameters<typeof chromium.launch>[0] = { headless, timeout: TIMEOUT_MS };
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
    { label: "system msedge",  channel: "msedge" as const },
    { label: "system chrome",  channel: "chrome" as const },
    { label: "managed chromium (no-sandbox)", channel: undefined, noSandbox: true },
    { label: "managed chromium (default)",    channel: undefined, noSandbox: false },
  ];
  let lastErr: unknown;
  for (const c of candidates) {
    try {
      return await tryLaunch(c.label, c.channel, (c as any).noSandbox ?? false);
    } catch (e) {
      const msg = (e as Error).message.split("\n")[0].slice(0, 200);
      console.error(`[launch] ${c.label} FAILED: ${msg}`);
      lastErr = e;
    }
  }
  throw lastErr instanceof Error ? lastErr : new Error("all launch candidates failed");
}

async function loginFlow(cookieDir: string) {
  mkdirSync(cookieDir, { recursive: true });
  console.error(`[login] cookie_dir=${resolve(cookieDir)}`);
  const { ctx, browser, label } = await launchContext(cookieDir, false, true);
  const page = ctx.pages()[0] ?? await ctx.newPage();
  await page.goto("https://note.com/login");
  console.error(`[login] ${label} 開きました。note.com にログインして、完了したらブラウザを閉じてください...`);

  // どちらのタイプでも close イベントを待つ
  await new Promise<void>((r) => {
    ctx.on("close", () => r());
    if (browser) browser.on("disconnected", () => r());
  });

  // persistent context の profile 自体を再利用するので storageState は参考保存のみ
  try {
    await ctx.storageState({ path: join(cookieDir, "note.json") });
  } catch { /* persistent 閉じた後は取れないので無視 */ }
  console.error("[login] Cookie を保存しました: " + resolve(cookieDir));
}

async function run(input: Input): Promise<Output> {
  const cookiePath = join(input.cookie_dir, "note.json");
  const profilePath = join(input.cookie_dir, "browser-profile");
  if (!existsSync(cookiePath) && !existsSync(profilePath)) {
    return { status: "needs_login", error: `${input.cookie_dir} が未初期化。--login で初回ログインしてください` };
  }

  const persist = existsSync(profilePath);
  const { ctx, browser } = await launchContext(input.cookie_dir, true, persist);

  try {
    const page = await ctx.newPage();
    await page.goto("https://note.com/notes/new", { waitUntil: "domcontentloaded" });

    if (page.url().includes("/login") || page.url().includes("/signin")) {
      return { status: "needs_login", error: "セッション切れ。--login で再ログインしてください" };
    }

    const md = readFileSync(input.md_path, "utf8");
    const { title, body } = parseFrontMatter(md);
    const finalTitle = title || input.title;

    const titleSel = 'textarea[placeholder*="タイトル"], h1[contenteditable="true"], input[placeholder*="タイトル"]';
    await page.waitForSelector(titleSel, { timeout: 30000 });
    await page.fill(titleSel, finalTitle).catch(async () => {
      await page.click(titleSel);
      await page.keyboard.type(finalTitle, { delay: 10 });
    });

    const bodySel = 'div[contenteditable="true"]';
    const bodyLocator = page.locator(bodySel).last();
    await bodyLocator.click();
    await page.keyboard.insertText(body);

    if (input.image_path && existsSync(input.image_path)) {
      const fileInputs = await page.locator('input[type="file"]').all();
      if (fileInputs.length > 0) {
        await fileInputs[0].setInputFiles(input.image_path).catch(() => {});
      }
    }

    if (input.publish) {
      const publishBtn = page.getByRole("button", { name: /公開/ }).first();
      await publishBtn.click({ timeout: 10000 }).catch(() => {});
      await page.getByRole("button", { name: /(投稿|公開する|確認して公開)/ }).first()
        .click({ timeout: 10000 }).catch(() => {});
      await page.waitForURL(/note\.com\/[^/]+\/n\//, { timeout: 60000 });
      return { status: "published", url: page.url() };
    } else {
      await page.waitForTimeout(3000);
      return { status: "draft", url: page.url() };
    }
  } catch (e: any) {
    return { status: "error", error: String(e?.message || e) };
  } finally {
    if (browser) await browser.close();
    else await ctx.close();
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

  try {
    const raw = await readStdin();
    const input = JSON.parse(raw) as Input;
    const out = await run(input);
    console.log(JSON.stringify(out));
    process.exit(out.status === "error" || out.status === "needs_login" ? 1 : 0);
  } catch (e: any) {
    console.log(JSON.stringify({ status: "error", error: String(e?.message || e) }));
    process.exit(1);
  }
}

void main();
