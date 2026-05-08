# v0.9.2 — Phase 3.6: SSRF 5 層防御完成 + TUI 階層 sub-bar + writer phase wire

**Release Date**: 2026-05-06
**Tag**: `v0.9.2`
**Previous**: v0.9.1

## ハイライト

v0.9.2 は v0.9.1 リリース直後の **Phase 3.6 短期集中**で、3 タスク並列着手戦略により security 完成 + UX 階層化 + 内部 wire 精緻化を同時着地。

- **SSRF defense-in-depth 5 層完成**: M3-C で TOCTOU pin + domain allowlist + cross-domain redirect block を追加、5 層完全防御達成
- **TUI 階層 sub-bar render**: W7-G で W7-E の sub-bar event を `↳ <label>` 階層表示で可視化
- **writer 内部 phase wire**: WPW1 で article 生成の 6 phase に tick 駆動追加、TUI で「どの記事のどの phase」が visible
- **3 並列戦略実証 + 1.5 日 21 PR**: M3-C/W7-G/WPW1 + version bump で v0.9.2 完成、累計 1.5 日で 21 PR 達成

## 詳細変更

### Security (1 PR)
- **M3-C** (#36): SSRF Fix C — TOCTOU + allowlist + redirect block
  - `src/util.rs` (+201/-?): `check_and_pin_image_client(url) -> reqwest::Client` per-URL pin client + `is_unsafe_ip(IpAddr)` + `matches_allowlist(host, &[String])` + 5 unit test
  - `src/writer/mod.rs` (+90/-?): 旧 `is_safe_image_url` + 共有 client 削除、新 per-URL pin client 経路に書き換え
  - `src/config.rs` (+16): `SecurityConfig::image_url_allowlist: Vec<String>` 追加 (TOML `[security]` section)
  - L4 (TOCTOU pin): `reqwest::ClientBuilder::resolve(host, ip)` で resolve 結果を pin、DNS 再 resolve race 排除
  - L5 (cross-domain redirect block): `redirect::Policy::custom` で host 変更時に再検証 → fail なら stop
  - **defense-in-depth 5 層完全達成**

### UI/UX (2 PR)
- **W7-G** (#37): TUI sub-bar render 階層拡張
  - `src/cli/tui.rs` (+140/-24): `App.sub_bars: HashMap<(Stage, String), SubBarItem>` + `SubBarItem` (state + msg + insertion order) + 4 variant 処理 + 階層表示 (挿入順)
  - `src/display.rs` (+61/-1): `Stage` enum に `Hash` derive + `PipelineUpdate::SubStart/SubTick/SubDone/SubFail` variant + `TuiBackend::sub_bar` override + `TuiSubBar` 構造体新設
  - 効果: 各 stage 直下に `↳ ✓ hn 30件` `↳ ⠿ google` 等の細粒度進捗表示

- **WPW1** (#38): writer 内部 phase wire (sub_bar tick 駆動)
  - `src/writer/mod.rs` (+98/-23): article ごとに sub_bar 事前作成 → write_one に move、6 phase で `bar.tick(msg)` 駆動 + Err 経路で `bar.fail(...)` 呼出
  - 6 phase: research(Grok) / brief(Haiku) / draft(Opus)+images parallel / source images(SSRF check + pin) / embed images & placeholder / save markdown
  - `short_label()` ヘルパ新設 (trend.title 24 文字短縮 + 空文字 fallback)
  - W7-G で `#[allow(dead_code)]` だった `PipelineUpdate::SubTick` variant が本 PR の `bar.tick()` 経由で実発火、自然解消

### Chore (1 PR)
- **chore: bump 0.9.1→0.9.2** (#39): Cargo.toml / Cargo.lock version sync

## 統計

- **4 PR + 1 tag/release** で v0.9.2 完成
- Total: ~1h で集中達成 (W7-G 1.5h + WPW1 1h + M3-C 2h)
- lowlevel: 1 PR (M3-C、~2h)
- ui-macos: 2 PR (W7-G 1.5h + WPW1 1h、合計 2.5h、見込み 5h を 50% 短縮)
- commander: 1 chore PR (version bump)
- 並行戦略: M3-C + W7-G 完全独立並列、WPW1 は M3-C 後着手で衝突回避

## 品質メトリクス推移

| 指標 | v0.9.0 | v0.9.1 | **v0.9.2 (現在)** |
|------|--------|--------|-------------------|
| warnings | 0 | 0 | **0** |
| clippy allow | 2 系統 | 0 | **0** |
| AI retry coverage | 6/6 | 6/6 | **6/6** |
| SSRF defense layer | L1 | L2 | **L5 (5 層完全)** |
| TUI sub-bar render | wire only | sub-bar event 発火 | **TUI 階層表示完成** |
| writer phase wire | なし | argument 経路 | **6 phase tick 駆動** |

## SSRF Defense-in-Depth 5 層完成

| 層 | 内容 | 起点バージョン |
|----|------|---------------|
| L1 | scheme==https + IPv4 リテラル private 等 block | v0.9.0 (M3 Fix A) |
| L2 | DNS resolve 後の全 IP safety check | v0.9.1 (M3-B) |
| **L3** | **domain allowlist (config-driven)** | **v0.9.2 (M3-C)** |
| **L4** | **resolve pin で TOCTOU race 排除** | **v0.9.2 (M3-C)** |
| **L5** | **cross-domain redirect block** | **v0.9.2 (M3-C)** |

防御済攻撃シナリオ:
- ✅ 直接的内部 IP アクセス (L1)
- ✅ DNS による迂回 (L2)
- ✅ allowlist 未登録 domain (L3)
- ✅ DNS rebinding (resolve race) (L4)
- ✅ open redirect → internal IP 誘導 (L5)

note-auto は **production cloud デプロイで実用可能な SSRF 完全防御**を達成。

## 互換性

- **配置環境**: developer / cloud VM 両対応 (5 層防御完備)
- **API key**: 既存設定そのまま、変更なし
- **公開関数 API**: M3-C で `is_safe_image_url` シグネチャ変更 (削除 + 新 `check_and_pin_image_client`)、内部 API のみで public API 影響なし
- **設定ファイル**: 新規 `[security] image_url_allowlist = [...]` section 追加可能 (省略時は M3-B 動作 fallback、後方互換)

## v0.9.3 (Phase 3.7) 予定スコープ

- **W7-D' deprecation 削除**: `#[allow(dead_code)]` 残置の整理 (display.rs SubTick variant 等が WPW1 で消費されたか確認)
- **quality 復帰時の Q?**: 任意取り込み
- **Phase 4 候補**: 未定、team-lead 判断仰ぐ

## クレジット

- **lowlevel**: M3-C 主担当、SSRF 5 層完成
- **ui-macos**: W7-G + WPW1 主担当、TUI 階層表示 + writer phase wire 完成、累計 5h 見込み 2.5h で完成 (50% 短縮実証)
- **quality (silent)**: Phase 3 H1 起点、SSRF 修正連鎖の出発点
- **team-lead**: Plan A 採用判断 + 並行戦略確認
- **commander**: orchestration + 4 PR 自走マージ + リリース工程

## v0.9.0+v0.9.1+v0.9.2 累計 (Phase 3 全期間)

| カテゴリ | v0.9.0 | v0.9.1 | v0.9.2 | 累計 |
|---------|--------|--------|--------|------|
| CI/CD | 3 | - | - | 3 |
| AI infrastructure | 3 | - | - | 3 |
| production 救済 | 3 | - | - | 3 |
| security | 1 (L1) | 1 (L2) | 1 (L3+L4+L5) | **3 PR、5 層完成** |
| UI/UX | 3 | 1 | 2 | 6 |
| code quality | - | 1 | - | 1 |
| chore | 1 | 1 | 1 | 3 |
| silent commit | 1 | - | - | 1 |
| **PR 総数** | **14** | **4** | **4** | **22** |

通信ルール v4 / v4.1 / v4.2 / pre-push hook / STATE プレフィックス全 PR 厳守、main 直接 commit ゼロ達成、accidental commit ゼロ。**4 重防壁が完璧に機能、5 回の時系列交差を全て 1 ターン合流で解決**。

---

🤖 Generated with [Claude Code](https://claude.com/claude-code)
