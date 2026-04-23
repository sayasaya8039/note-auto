/**
 * note.com 自動投稿 Playwright サイドカー
 *
 * 入力: stdin から JSON
 *   { md_path, image_path?, title, tags[], publish, cookie_dir }
 *
 * 出力: stdout の最終行に JSON
 *   { status: "published" | "draft" | "needs_login" | "error", url?, error? }
 *
 * Cookie: <cookie_dir>/note.json に Playwright storageState 形式で保存。
 *
 * 初回セットアップ:
 *   bun scripts/note-publish.ts --login
 *     → ブラウザ起動、ユーザーが note.com にログイン、閉じると cookie 保存
 */

import { chromium, type BrowserContext } from "playwright";
import { readFileSync, existsSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";

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

async function loginFlow(cookieDir: string) {
  mkdirSync(cookieDir, { recursive: true });
  const browser = await chromium.launch({ headless: false });
  const ctx = await browser.newContext();
  const page = await ctx.newPage();
  await page.goto("https://note.com/login");
  console.error("note.com にログインしてください。完了したらブラウザを閉じてください...");
  await page.waitForEvent("close", { timeout: 0 }).catch(() => {});
  await ctx.storageState({ path: join(cookieDir, "note.json") });
  await browser.close();
  console.error("Cookie を保存しました。");
}

async function loadContext(cookieDir: string): Promise<BrowserContext | null> {
  const cookiePath = join(cookieDir, "note.json");
  if (!existsSync(cookiePath)) return null;
  const browser = await chromium.launch({ headless: true });
  return await browser.newContext({ storageState: cookiePath });
}

async function run(input: Input): Promise<Output> {
  const ctx = await loadContext(input.cookie_dir);
  if (!ctx) {
    return { status: "needs_login", error: `${input.cookie_dir}/note.json が存在しません。--login で初回ログインしてください` };
  }

  const page = await ctx.newPage();
  try {
    await page.goto("https://note.com/notes/new", { waitUntil: "domcontentloaded" });

    // ログイン状態確認
    if (page.url().includes("/login") || page.url().includes("/signin")) {
      return { status: "needs_login", error: "セッション切れ。--login で再ログインしてください" };
    }

    const md = readFileSync(input.md_path, "utf8");
    const { title, body } = parseFrontMatter(md);
    const finalTitle = title || input.title;

    // タイトル入力 (note のエディタ: h1[placeholder*="タイトル"] 等)
    const titleSel = 'textarea[placeholder*="タイトル"], h1[contenteditable="true"], input[placeholder*="タイトル"]';
    await page.waitForSelector(titleSel, { timeout: 30000 });
    await page.fill(titleSel, finalTitle).catch(async () => {
      // contenteditable の場合は type で
      await page.click(titleSel);
      await page.keyboard.type(finalTitle, { delay: 10 });
    });

    // 本文入力 (contenteditable エディタ)
    const bodySel = 'div[contenteditable="true"]';
    const bodyLocator = page.locator(bodySel).last();
    await bodyLocator.click();
    // 改行は Enter 押下で note がリッチエディタとして処理するので、
    // まず単純化: 全行を1発で insert text
    await page.keyboard.insertText(body);

    // アイキャッチ画像アップロード (input[type=file])
    if (input.image_path && existsSync(input.image_path)) {
      const fileInputs = await page.locator('input[type="file"]').all();
      if (fileInputs.length > 0) {
        await fileInputs[0].setInputFiles(input.image_path).catch(() => {});
      }
    }

    // 下書き保存 or 公開
    if (input.publish) {
      // 「公開設定」→「公開する」フロー — ボタンテキストはUI更新で変わるため複数候補
      const publishBtn = page.getByRole("button", { name: /公開/ }).first();
      await publishBtn.click({ timeout: 10000 }).catch(() => {});
      // 最終確認ダイアログの「投稿」「公開する」ボタン
      await page.getByRole("button", { name: /(投稿|公開する|確認して公開)/ }).first()
        .click({ timeout: 10000 }).catch(() => {});
      // URL が /n/xxxx に遷移するのを待つ
      await page.waitForURL(/note\.com\/[^/]+\/n\//, { timeout: 60000 });
      const url = page.url();
      return { status: "published", url };
    } else {
      // 自動保存されるので明示的に下書き URL を取得
      // URL が /notes/xxx/edit になっていれば下書き保存済み
      await page.waitForTimeout(3000);
      const url = page.url();
      return { status: "draft", url };
    }
  } catch (e: any) {
    return { status: "error", error: String(e?.message || e) };
  } finally {
    await ctx.close();
  }
}

async function main() {
  if (process.argv.includes("--login")) {
    const cookieDir = process.argv[process.argv.indexOf("--login") + 1] ?? ".cookies";
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

void dirname;
void main();
