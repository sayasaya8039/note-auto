# lowlevel Phase 4 監査レポート (Fix-E 派生 + slack.rs 精査)

**担当**: lowlevel
**作成日**: 2026-05-07
**対象 main HEAD**: `4e629e1` v0.9.3
**コンテキスト**: v0.9.4 production hot-fix の派生監査

---

## 1. Fix-E2 適用確認 ✅

- **PR #45**: https://github.com/sayasaya8039/note-auto/pull/45
- **branch**: `fix/v0.9.4-fix-e2-dotenvy`
- **修正**: `src/main.rs::main()` 冒頭に `let _ = dotenvy::dotenv();` 5 行追加 (コメント 3 行 + 1 行 + 空行)
- **挿入位置**: `Cli::parse()` より前 (環境変数を CLI parse / Theme detect / Config load より先に確立)
- **build**: 0 errors / 12.45s incremental / warnings 0 / clippy default 全 pass
- **branch 確証 4 段階**: 全段階 `fix/v0.9.4-fix-e2-dotenvy` 維持 (accidental 前科を踏まえた厳守)

---

## 2. slack.rs 監査

### 2.1 dry-run 経路 ✅
```rust
if cfg.publish.dry_run {
    tracing::info!("[dry-run] Slack通知スキップ");
    return Ok(());
}
```
正常: dry-run 時は早期 return、http 呼び出しなし。**問題なし**。

### 2.2 SLACK_WEBHOOK_URL 未設定経路 ✅
```rust
let Some(url) = cfg.publish.slack_webhook_url.as_deref() else {
    tracing::info!("SLACK_WEBHOOK_URL 未設定 — Slack通知スキップ");
    return Ok(());
};
```
正常: 未設定時は silent skip + info ログ。**Fix-E2 後は env 経由で設定取得が確実化、この path は本来不要だが防御層として残置妥当**。

### 2.3 error 経路 ⚠️ retry 不採用
```rust
let resp = client.post(url).json(&body).send().await?;
if !resp.status().is_success() {
    return Err(anyhow!("Slack webhook {}: {}", status, txt));
}
```
**潜在改善余地**: `crate::util::send_with_retry` 未使用。

- Slack webhook 5xx (Slack 側障害) で即 fail
- caller (`daemon::execute_cycle`) が `.ok()` で握り潰すため production への影響は小さい
- ただし silent loss の温床、M1/M2 と同パターンの retry 適用が望ましい

**優先度**: 中 (production 通知信頼性の地味改善、cooling-off 後の v1.0.0 候補)。

### 2.4 post_progress (best-effort path) ✅
```rust
match client.post(url).json(&body).send().await {
    Ok(r) if !r.status().is_success() => tracing::warn!(...),
    Err(e) => tracing::warn!(...),
    _ => {}
}
```
正常: 完全 best-effort、warn ログのみで進行継続。**問題なし**。

---

## 3. dotenvy 配線忘れ類似の潜在バグ精査

### 3.1 env::var 直接呼び出し箇所 (12 件) ✅
全て `src/config.rs::env_*` helper として集約済:

| Helper | 環境変数 | Config field |
|--------|---------|-------------|
| env_xai_key | XAI_API_KEY | trends.xai_api_key |
| env_anthropic_key | ANTHROPIC_API_KEY | writer.anthropic_api_key |
| env_openai_key | OPENAI_API_KEY | writer.openai_api_key |
| env_pollo_key | POLLO_API_KEY | writer.pollo_api_key |
| env_nvidia_key | NVIDIA_API_KEY | writer.nvidia_api_key |
| env_gemini_key | GEMINI_API_KEY / GOOGLE_API_KEY | writer.gemini_api_key |
| env_x_api_key | X_API_KEY | publish.x_api_key |
| env_x_api_secret | X_API_SECRET | publish.x_api_secret |
| env_x_access_token | X_ACCESS_TOKEN | publish.x_access_token |
| env_x_access_secret | X_ACCESS_SECRET | publish.x_access_secret |
| env_slack_webhook | SLACK_WEBHOOK_URL | publish.slack_webhook_url |

→ 全 env_* helper は `Config::load` 経由で評価される (`#[serde(default = "env_...")]`)。

### 3.2 display.rs の env 直接参照 (3 件) ✅
```rust
std::env::var_os("NO_COLOR")
std::env::var("LANG")
std::env::var_os("WT_SESSION")
```
これらは **runtime 環境変数** (色制御 / locale 検出) であり、`.env` 経由で設定する性質ではない。`dotenvy::dotenv()` の対象外として **問題なし**。

### 3.3 Config::load 内の dotenvy 重複呼び出し ✅
`src/config.rs:328`:
```rust
let _ = dotenvy::from_filename(".env");
```
- main.rs の `dotenvy::dotenv()` (Fix-E2) と重複呼び出し
- dotenvy は idempotent (再呼び出しで害なし、既存 env は上書きしない)
- **保険として妥当、削除不要**

### 3.4 dotenvy 配線抜け類似のバグ: **なし** ✅

ai/* / publish/* / writer/* のいずれも env_* helper 経由で取得しており、main entry の `dotenvy::dotenv()` で全箇所カバー。Fix-E2 適用で root cause 完全解消。

---

## 4. 推奨

### 即対応 (本 v0.9.4 で完了)
- ✅ Fix-E2 (PR #45) commit 済 → commander 自走マージ待ち

### v1.0.0 cooling-off 後の改善候補
| ID | 内容 | 優先度 | 工数 |
|----|------|-------|------|
| **AUDIT-1** | `slack.rs::post_summary` に `send_with_retry` 適用 (M1/M2 と同パターン) | 中 | 15min |
| AUDIT-2 | dotenvy 配線確認の integration test (起動時に env を assert) | 低 | 30min |
| AUDIT-3 | env_* helper の Default 値検討 (.env 不在 + env 不在時のエラーメッセージ強化) | 低 | 30min |

### 却下
- env_* helper の集約解除: 既に綺麗に集約されており、変更価値なし
- Config::load の dotenvy 重複削除: 保険として妥当、削除すると外部利用時のリスク

---

## 5. 結論

- **Fix-E2 適用で v0.9.4 production CRITICAL は完全救済**
- slack.rs は dry-run / 未設定 / best-effort 経路すべて問題なし、唯一の改善余地は `post_summary` の retry 適用 (AUDIT-1)
- dotenvy 配線抜け類似のバグは **他に存在しない** (env_* helper 集約 + Config::load 経由で evaluation 統一)
- v0.9.4 hot-fix は本 PR-B (#45) + ui-macos PR-A (Fix-A/B/D/S) のマージで完了見込み

cooling-off 期間中の AUDIT-1 投入も可能 (15 分作業)、commander 判断で v0.9.5 ad-hoc PR or v1.0.0 phase 統合のいずれかで処遇。

以上、Phase 4 hot-fix lowlevel スコープ完了。
