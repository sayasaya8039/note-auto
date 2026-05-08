# v0.9.4 hot-fix 改修方針案 (Phase 4) — production CRITICAL 救済

> **Status**: 設計案上申中 (実装未着手 / team-lead 承認待ち)
> **Owner**: ui-macos teammate
> **Target version**: v0.9.4 (緊急 hot-fix release)
> **Scope**: `scripts/note-publish.mjs` のみ (Rust 側変更なし)
> **Worktree**: `note-auto-phase4` 推奨 (commander 提案)
> **承認後着手見込み**: 受領後 5 分以内 → 実装完了 ~2h (内訳下記) → smoke + PR up ~30min

---

## サマリ

| Fix | 課題 | 想定工数 | 優先度 |
|-----|------|---------|--------|
| **Fix-A** | managed chromium fallback の UA 非統一で HeadlessChrome 検出 → CORS 弾き | 5min | CRITICAL |
| **Fix-B** | SingletonLock 衝突で Chrome 子プロセス exit 21 即死 / port 待機が長すぎる | 30min | CRITICAL |
| **Fix-D** | 「下書き保存」click が `.catch(() => {})` で握りつぶされ status=draft 詐称 | 30min | CRITICAL |
| **Fix-S** | `waitForSelector` 30s が React hydrate 遅延時に失敗、失敗痕跡なし | 45min | HIGH |
| **小計** | (実装) | **~1h50min** | — |
| smoke + PR 整備 | review 通過まで | ~30min | — |
| **総計** | | **~2h20min** | — |

---

## Fix-A: managed chromium fallback UA 統一

### 課題

`launchViaCDP` (line 59-99) では UA を `Chrome/148.0.0.0` に偽装しているが、fallback 経路の `tryLaunch` (line 127-154) は **UA 引数なしで起動**する。これにより managed chromium が使われた瞬間 `HeadlessChrome/...` の素の UA が露呈し、note.com の API が CORS で弾く。CDP 経路が落ちると本番が即死する。

### 修正箇所

| ファイル | 範囲 | 内容 |
|----------|------|------|
| `scripts/note-publish.mjs` | line 65 周辺 | `UA` 定数を module スコープに昇格 (`launchViaCDP` 外へ) |
| `scripts/note-publish.mjs` | line 129-135 (`baseArgs`) | `--user-agent=${UA}` を追加 |
| `scripts/note-publish.mjs` | line 138, 145 (`opts`) | persist 側も非 persist 側も同じ baseArgs を経由するので 1 箇所修正で両方反映 |

### 修正内容（疑似コード）

```js
// line 1-50 付近 (module top)
const COMMON_UA = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36";

// line 65 (launchViaCDP 内)
const UA = COMMON_UA; // 既存ロジック維持

// line 129-135 (tryLaunch baseArgs)
const baseArgs = [
  "--disable-blink-features=AutomationControlled",
  "--disable-features=IsolateOrigins,site-per-process",
  `--user-agent=${COMMON_UA}`,  // ← 追加
];
```

### リスク

- **低**: UA 追加のみ、挙動変更は note.com 側の検出回避のみ
- system msedge / system chrome channel では UA がブラウザ既定値を上書きするため、Edge 系 UA を期待する API がもしあれば反応が変わる可能性 (現状未確認、影響軽微)

---

## Fix-B: SingletonLock 衝突回避 + port timeout 短縮

### 課題

CDP launch (line 80) 後に `child.on("exit")` で `code=21` (SingletonLock 競合) が出ると port が永遠に開かず、`waitForPort` (line 83) のデフォルト timeout (=20s 想定) を浪費して全 candidates が直列で時間切れになる。本番が >2 分で fail する原因。さらに `--user-data-dir` 競合時にプロファイルロックが残骸として滞留する。

### 修正箇所

| ファイル | 範囲 | 内容 |
|----------|------|------|
| `scripts/note-publish.mjs` | line 60-62 | profileDir 確保時に `SingletonLock` / `SingletonCookie` / `SingletonSocket` を unlink (best-effort) |
| `scripts/note-publish.mjs` | line 80-83 | `child.on("exit")` で exit code 21 を検出した瞬間 reject、`waitForPort` を打ち切る |
| `scripts/note-publish.mjs` | `waitForPort` 定義箇所 (line 30-50 付近想定) | timeout を 20s → **5s** に短縮 |
| `scripts/note-publish.mjs` | line 83 後 | exit 21 検出時は **5 秒待機 → 同 candidate を 1 回 retry**（残骸ロック解放猶予） |

### 修正内容（疑似コード）

```js
// line 60-62 付近
const lockFiles = ["SingletonLock", "SingletonCookie", "SingletonSocket"];
for (const lock of lockFiles) {
  try { unlinkSync(resolve(profileDir, lock)); } catch {}
}

// line 80-83 付近を Promise.race 化
const exitPromise = new Promise((_, rej) => {
  child.once("exit", (code) => {
    if (code === 21) rej(new Error(`SingletonLock collision (exit=21), retry after 5s`));
    else rej(new Error(`browser exited code=${code}`));
  });
});
const portPromise = waitForPort(port, 5_000); // 20s → 5s
try {
  await Promise.race([portPromise, exitPromise]);
} catch (e) {
  if (/SingletonLock/.test(e.message)) {
    await sleep(5_000);
    // 同 candidate を 1 回 retry (再帰呼び出し or flag で深さ制御)
    return await launchViaCDP(executablePath, cookieDir, headless, /*retry=*/true);
  }
  throw e;
}
```

### リスク

- **中**: SingletonLock unlink は他の Chrome instance が同 user-data-dir を握っていた場合に競合を起こす可能性 (本番では note-publish 専用 profile なので衝突確率は低いが、ユーザの手動 launch と被ると壊れる) → README で「該当 Chrome を閉じてから実行」を明記
- 5s timeout は port が遅い WSL/低速 PC では false negative の可能性 → fallback で次 candidate に進む既存ロジックがあるので致命的ではない
- retry 1 回限定なので無限ループは無し

---

## Fix-D: 下書き保存 button click のエラー透過化 + selector 強化

### 課題

line 384:

```js
await page.getByRole("button", { name: "下書き保存" }).click({ timeout: 10000 }).catch(() => {});
await page.waitForTimeout(2000);
return { status: "draft", url: page.url() };
```

- `.catch(() => {})` で全エラーを握りつぶし、button が見つからなくても `status: "draft"` を返す → **draft 失敗を成功と誤報**
- selector が単一 (`"下書き保存"` 完全一致) で、note.com 側 button label 変更や aria-label 化で即死
- `page.url()` が `/notes/new` のままでも success 扱い

### 修正箇所

| ファイル | 範囲 | 内容 |
|----------|------|------|
| `scripts/note-publish.mjs` | line 383-386 | `.catch(() => {})` 削除、明示的な try/catch + 失敗時 `status: "error"` |
| `scripts/note-publish.mjs` | line 384 | selector を 3 候補で順次試行 (regex / 別ボタン名 / aria-label) |
| `scripts/note-publish.mjs` | line 385 後 | URL が `/edit/<id>` パターンに合致するか検証、未合致なら error |

### 修正内容（疑似コード）

```js
// line 383-386 を全置換
const draftSelectors = [
  () => page.getByRole("button", { name: /^下書き保存$/ }),
  () => page.getByRole("button", { name: /下書き(保存|を保存)/ }),
  () => page.locator('button[aria-label*="下書き"]').first(),
];
let clicked = false;
let lastErr;
for (const sel of draftSelectors) {
  try {
    await sel().click({ timeout: 5000 });
    clicked = true;
    break;
  } catch (e) { lastErr = e; }
}
if (!clicked) {
  return { status: "error", error: `下書き保存 button not found: ${lastErr?.message ?? "unknown"}` };
}
await page.waitForTimeout(2000);
const url = page.url();
if (!/note\.com\/notes\/(new|[a-zA-Z0-9_-]+\/edit)/.test(url) && !/\/edit\//.test(url)) {
  return { status: "error", error: `unexpected URL after draft save: ${url}` };
}
return { status: "draft", url };
```

### リスク

- **低-中**: selector 候補追加で誤クリックリスク (例: 「下書きを破棄」等の類似文言にマッチ) → regex を `^下書き保存$` で開始/終了アンカー化済み
- 既存 success 経路で URL pattern が想定外だと false negative (error 化) する可能性 → URL pattern を緩めに (`/edit/` 含むなら OK) 設計
- 戻り値が `status: "error"` に変わるので Rust 側 (`src/publisher/...`) で error handling が draft 期待ロジックを壊さないか確認必要 → 既存 error 経路に合流するだけなので影響軽微

---

## Fix-S: waitForSelector 強化 + 失敗時 screenshot + 1-retry reload

### 課題

line 224, 230:

```js
await page.waitForSelector(titleSel, { timeout: 30000 }); // line 224
await page.waitForSelector(bodySel, { timeout: 30000 });  // line 230
```

- React hydrate が遅い (= note.com フロントエンド更新時 / CPU 負荷時) と 30s で打ち切られる
- 失敗時は exception throw → catch (line 388-390) で `status: "error"` 返却するが **何が見えていたかの痕跡なし** → debug 不可
- 1 回失敗で諦めるので transient な hydrate 遅延でも再起動 / 手動介入が必要

### 修正箇所

| ファイル | 範囲 | 内容 |
|----------|------|------|
| `scripts/note-publish.mjs` | line 224, 230 | timeout 30000 → **60000** |
| `scripts/note-publish.mjs` | line 224, 230 を関数化 | 失敗時に screenshot 保存 + page reload + 1 回 retry |
| `scripts/note-publish.mjs` | logs/screenshots ディレクトリ確保 | `cookieDir/screenshots/<timestamp>-<stage>.png` |

### 修正内容（疑似コード）

```js
// 新規ヘルパー (line 200 付近)
async function waitForSelectorWithRetry(page, selector, stage, cookieDir) {
  try {
    await page.waitForSelector(selector, { timeout: 60_000 });
    return;
  } catch (e1) {
    // 失敗痕跡を保存
    const ts = new Date().toISOString().replace(/[:.]/g, "-");
    const shotDir = resolve(cookieDir, "screenshots");
    mkdirSync(shotDir, { recursive: true });
    const shotPath = join(shotDir, `${ts}-${stage}-fail.png`);
    try { await page.screenshot({ path: shotPath, fullPage: true }); } catch {}
    console.error(`[publish] waitForSelector(${stage}) FAILED, screenshot=${shotPath}, retrying with reload...`);
    // 1 回 reload + retry
    try {
      await page.reload({ waitUntil: "networkidle", timeout: 60_000 });
      await page.waitForTimeout(3000);
      await page.waitForSelector(selector, { timeout: 60_000 });
      console.error(`[publish] waitForSelector(${stage}) OK on retry`);
    } catch (e2) {
      const shotPath2 = join(shotDir, `${ts}-${stage}-fail2.png`);
      try { await page.screenshot({ path: shotPath2, fullPage: true }); } catch {}
      throw new Error(`${stage} not found after reload: ${e2.message}`);
    }
  }
}

// line 224
await waitForSelectorWithRetry(page, titleSel, "title", input.cookie_dir);
// line 230
await waitForSelectorWithRetry(page, bodySel, "body", input.cookie_dir);
```

### リスク

- **低**: timeout 60s に伸ばすと最悪ケースで実行時間が +30s/article = 21 記事で +10min。ただし retry が走るのは異常時のみで通常は `waitForSelector` が早期に解決
- screenshot ディスク使用量増加 (~500KB/shot) → 既存 cookie_dir 下なので運用影響軽微、必要なら定期 cleanup を別タスクで
- reload で意図せず下書きが破棄される可能性 → note.com の auto-save が hydrate 後に走るので、hydrate 前の reload は無害（本文未入力段階）

---

## PR 構造提案

### 案 A: 1 PR 統合 (commander 推奨 / 私も同意)

- **branch**: `feat/v0.9.4-publish-hardening`
- **base**: `main`
- **scope**: Fix-A + Fix-B + Fix-D + Fix-S を 1 PR にまとめる
- **commit 分割**: 4 commit (Fix 単位) で履歴を残す
  - `fix(publish): unify chromium UA across CDP and managed launch (Fix-A)`
  - `fix(publish): handle SingletonLock collision and shorten port timeout (Fix-B)`
  - `fix(publish): de-suppress draft save errors and reinforce selectors (Fix-D)`
  - `fix(publish): retry waitForSelector with screenshot on hydrate delay (Fix-S)`
- **理由**:
  - 全て同一ファイル (`scripts/note-publish.mjs`) で衝突回避不要
  - production CRITICAL なので review-merge-deploy を 1 サイクルで完了したい
  - smoke test (login → draft save) は 4 fix まとめてでないと網羅できない

### 案 B: 2 PR 分離 (代替案)

- PR1 `feat/v0.9.4a-cdp-hardening` (Fix-A + Fix-B): CDP launch 経路の堅牢化
- PR2 `feat/v0.9.4b-publish-resilience` (Fix-D + Fix-S): publish flow のエラー検出強化
- **理由**: review 単位を縮小、リスク段階的展開
- **デメリット**: CRITICAL hot-fix としては時間がかかりすぎる、案 A 推奨

### 私の推奨

**案 A: 1 PR 統合**。理由:
- 全 Fix が `scripts/note-publish.mjs` 1 ファイル内、依存関係も小さい
- production CRITICAL は速度優先、PR 分離コストが見合わない
- commit 単位で 4 分割すれば revert 粒度は確保できる

---

## smoke test 計画

PR 上げ前に worktree で以下を手動検証:

1. `node scripts/note-publish.mjs --login` で cookie 保存 (Fix-A の UA 統一を確認)
2. SingletonLock 残骸を作って `--login` 再実行 (Fix-B の retry 検証)
3. `--draft` で 1 記事投稿 → `status: "draft"` + URL が `/edit/` 含むことを確認 (Fix-D)
4. note.com を一旦 throttling 状態で 1 記事投稿 → screenshot が出力され retry が成功すること (Fix-S)
5. 上記 4 件 OK で PR 投稿

---

## 着手見込み時刻

- **本レポート受領 → team-lead 承認**: 想定 ≤30min
- **承認後 → 実装着手**: 即時 (worktree `note-auto-phase4` 作成)
- **実装完了**: ~2h (内訳: Fix-A 5min, Fix-B 30min, Fix-D 30min, Fix-S 45min, 整合確認 10min)
- **smoke + PR up**: +30min
- **合計**: 承認から ~2h30min で PR up 可能

---

## 担当外への観察

- Rust 側 (`src/publisher/...`) で `status: "error"` 受信時の retry / 通知ロジックが十分か lowlevel 領域での確認推奨 (本 PR の外)
- `cookie_dir/screenshots/` の自動 cleanup を quality 領域で別タスク化推奨 (運用 1 ヶ月で数 GB 蓄積の可能性)

---

> 4 fix すべて `scripts/note-publish.mjs` 同一ファイル内で完結、案 A (1 PR / 4 commit) を推奨。
> team-lead 承認後即時着手、~2h30min で PR up 可能。
