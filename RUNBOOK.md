# note-auto Runbook — 実運用ガイド

## 概要

トレンド収集 → AI 執筆 → 画像4枚生成 → note 自動投稿 → X 告知 → Slack 通知
を毎朝 07:00 JST に全自動実行する Rust 製デーモン。

---

## 初回セットアップ (完了済み)

1. `.env` に API キー設定済み
   - `ANTHROPIC_API_KEY`
   - `XAI_API_KEY`
   - `POLLO_API_KEY`
   - `X_API_KEY` / `X_API_SECRET` / `X_ACCESS_TOKEN` / `X_ACCESS_SECRET`
   - `SLACK_WEBHOOK_URL`
2. `bun install` + `bunx playwright install chromium` 済み
3. `bun scripts/note-publish.mjs --login .cookies` で note.com ログイン済み
   (通常ターミナルから実行)

---

## コマンド一覧

```powershell
cd D:\NEXTCLOUD\Windows_app\note-auto

# 個別実行
./target/release/note-auto.exe fetch-trends --top 3
./target/release/note-auto.exe write --from drafts/YYYY-MM-DD/trends.json
./target/release/note-auto.exe publish --from drafts/YYYY-MM-DD/articles.json
./target/release/note-auto.exe notify --message "test"
./target/release/note-auto.exe x-test --message "test"

# 統合実行
./target/release/note-auto.exe run --top 3             # fetch + write のみ
./target/release/note-auto.exe once                    # fetch + write + publish + notify
./target/release/note-auto.exe once --dry-run          # 配線のみ確認
./target/release/note-auto.exe daemon                  # 常駐 (07:00 JST 毎日発火)
```

---

## 運用モード

### A. 安全モード (推奨、現状設定)
`config.toml`:
```toml
[publish]
note_publish = false  # 下書き保存のみ
x_announce = true
```
- 毎朝 07:00 に記事 3本が note の下書きに保存される
- X と Slack に通知が飛ぶ
- 人間が内容を確認してから note 公開ボタンを手動クリック
- **1週間運用して問題なければ Mode B へ**

### B. 完全自動公開モード
`config.toml`:
```toml
[publish]
note_publish = true   # 公開ボタン押下まで自動
```

### 常駐起動
```powershell
./target/release/note-auto.exe daemon
```
→ Ctrl+C で停止。バックグラウンド化は Windows Task Scheduler で。

---

## コスト目安 (記事1本あたり)

| API | 用途 | 概算 |
|-----|------|------|
| Grok (xAI) | リサーチ | $0.01-0.03 |
| Claude Haiku 4.5 | ブリーフ | $0.005 |
| Claude Opus 4.7 | 本文執筆 (4000字) | $0.20-0.40 |
| Pollo AI gpt-image-2-0 | 画像4枚 (high quality) | $0.20-0.40 |
| X API | 告知 tweet | $200/月 (Basic plan) |
| **合計** | 記事1本 | **約 $0.40-0.80** |
| **月間 (3記事 × 30日)** | 90 記事 | **約 $40-75 + X $200** |

---

## トラブルシューティング

### note ログイン切れ
```
[publish] needs_login 
```
→ 通常ターミナルから: `bun scripts/note-publish.mjs --login .cookies`

### X 告知 "duplicate content" 403
→ 同じ内容を 24 時間以内に再投稿すると X が拒否。別記事で次回。

### Pollo "billing_limit"
→ https://pollo.ai で残高確認・チャージ。

### Anthropic/OpenAI 残高切れ
→ 該当ダッシュボードでチャージ。

### ブラウザが開かない (Claude Code 内で)
→ Playwright は通常ターミナル経由でのみ動作。Claude Code bash からは MIC の
  制約で起動不可。手動で login / inspect は PowerShell から実行。

---

## ファイル配置

```
D:\NEXTCLOUD\Windows_app\note-auto\
├── .env                        API キー (gitignore)
├── .cookies/                   note Cookie (gitignore)
├── config.toml                 設定
├── Cargo.toml / src/           Rust 本体
├── package.json / scripts/
│   └── note-publish.mjs        Playwright サイドカー
├── drafts/YYYY-MM-DD/          日次生成物
│   ├── trends.json
│   ├── articles.json           manifest
│   ├── {slug}.md               記事本文
│   ├── {slug}-hero.png         見出し (CityRiver バナー)
│   ├── {slug}-1.png            インライン画像 (フォトリアル)
│   ├── {slug}-2.png
│   └── {slug}-3.png
└── target/release/note-auto.exe
```

---

## カスタマイズ

### 記事数を変える
`config.toml`:
```toml
[schedule]
daily_top = 5  # 1日5記事
```

### 発火時刻を変える
```toml
[schedule]
cron = "0 30 6 * * *"  # 毎朝 06:30 JST
```

### 画像プロバイダを OpenAI に戻す
```toml
[writer]
image_provider = "openai"
image_model = "gpt-image-1"
image_size = "1536x1024"
```

### Hero のキャラを変える
`src/writer/mod.rs` の `build_hero_prompt()` 内のキャラクター記述を編集
→ `cargo zigbuild --release`

### カテゴリ重みを調整
```toml
[scoring.source_weights]
x = 1.5      # X 由来を高く
note = 0.8   # note RSS を低く
```

---

## 学習パターン (参考)

開発過程で蓄積された再利用可能な知見:
- `~/.claude/skills/learned/playwright-windows-cdp-bypass.md` (#038 S評価)
- `~/.claude/skills/learned/rich-editor-marker-upload.md` (#039 A評価)
- `~/.claude/skills/learned/rust-playwright-sidecar.md` (#035)
- `~/.claude/skills/learned/ai-pipeline-dry-run-stub-factory.md` (#034)
- `~/.claude/skills/learned/external-api-spec-openapi-probe.md` (#037 S評価)
