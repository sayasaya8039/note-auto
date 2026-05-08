# B2: enforce_admins 切替検討レポート

> **担当**: commander
> **日付**: 2026-05-06
> **目的**: `branch protection` の `enforce_admins=false → true` 切替の是非を、Phase 2.5 の admin override 3 件をシミュレーションして評価

---

## 結論

**現状維持 (`enforce_admins=false`)** + 補強策強化を推奨。

理由: 過去 3 件中 1 件 (silent quality) は本質的に admin override が**必要**な運用、残 2 件 (lowlevel accidental) は **v4.1 + pre-push hook で予防可能**。完全強制は柔軟性を失う割に得るものが少ない。

---

## 過去 admin override 3 件のシミュレーション

仮に `enforce_admins=true` だった場合、各 override は:

### Case 1: `4ba4ddb feat(ai): unified retry/backoff (M2)` — lowlevel accidental

| 項目 | enforce_admins=false (実際) | enforce_admins=true (シミュレーション) |
|------|---------------------------|------------------------------------|
| push 結果 | ✅ 成功（admin bypass） | ❌ block |
| 検出タイミング | push 後の commander dirty check | push 試行直後の error |
| 復旧手順 | admin-override-log 記録 + 内容追認 | feat ブランチに force checkout + push、PR 作成 (5 分) |
| 開発スピード影響 | なし（既に main にある） | **+5 分**（PR 作成 + マージ承認待ち） |
| process 健全性 | ⚠ ルール違反として記録 | ✅ ルール厳守 |

→ enforce_admins=true なら **5 分の手戻り**だが、process 厳守。

### Case 2: `eba2567 refactor(quality): M1 + Warning 1/3/4` — quality silent worker

| 項目 | enforce_admins=false (実際) | enforce_admins=true (シミュレーション) |
|------|---------------------------|------------------------------------|
| push 結果 | ✅ 成功（silent commit パターン） | ❌ block |
| silent worker としての貢献 | 機能、main 直接 commit で完結 | **不可能**、PR 経由必須でメッセージ応答も必要 |
| 復旧手順 | admin-override-log で記録、追認 | 別エージェント (commander) が代行 PR 化、commit 帰属を `Co-Authored-By` で記録 |
| 開発スピード影響 | なし | **+15-30 分**（commander が代行 PR 作成） |
| 運用継続性 | silent worker の理想形 | silent worker パターン**廃止** |

→ enforce_admins=true なら **silent worker 運用が機能不全**になる。  
→ team-lead が以前確認した「silent quality は admin override で運用継続」方針と矛盾。

### Case 3: `0a36b9f openai M2 (PR-M scope-up)` — lowlevel accidental (2 度目)

Case 1 と同様。enforce_admins=true なら **+5 分**の手戻り、ただし v4.1 + pre-push hook 完備後なので発生しない見込み。

---

## 集計: enforce_admins=true へ切り替えた場合の影響

| 観点 | 影響 |
|------|------|
| **lowlevel accidental 防止** | ✅ 物理ブロック（ただし pre-push hook で代替可能） |
| **silent quality 運用** | ❌ 廃止、commander 代行で +15-30 分/件のオーバーヘッド |
| **緊急 hot-fix 対応** | ⚠ admin も PR 経由必須、レビュー待ち発生 |
| **process 厳守** | ✅ 完璧 |
| **governance 強度** | ✅ 最大 |

→ **トレードオフ**: lowlevel accidental は予防策で代替可能、silent quality は廃止コスト大。

---

## 推奨案

### Plan: 現状維持 + 補強強化

1. **`enforce_admins=false` を維持** （admin = sayasaya8039 = silent quality 経路を温存）
2. **pre-push hook を全担当に必須化**（ローカル accidental 物理ブロック）
3. **v4.1 ルール（branch 確認義務化）を継続**
4. **admin-override-log.md を月次レビュー**（abuse 監視）
5. **Phase 4 で silent quality の通信化検討**（quality に SendMessage 復帰促す）

### 段階的移行の長期ビジョン

| フェーズ | 設定 | 条件 |
|---------|------|------|
| Phase 3 (現状) | enforce_admins=false | silent quality 必須、admin override 月 5 件以下を維持 |
| Phase 4 候補 | enforce_admins=false | silent quality が SendMessage 復帰、admin override 月 2 件以下に減少 |
| Phase 5 候補 | **enforce_admins=true** | silent quality 完全廃止、admin override 不要な体制完成 |

→ silent quality の通信化が Phase 5 移行の前提条件。Phase 3 ではまだ無理。

---

## 監視メトリクス（月次レビュー用）

`reports/admin-override-log.md` から以下を月次集計:

- 累計 admin override 数
- 帰属別 (silent quality / lowlevel accidental / 緊急 hot-fix)
- 平均月次件数 (3 ヶ月移動平均)
- ルール違反率 (= accidental / 総数)

**目標値**:
- 累計月次: 5 件以下
- accidental 比率: 30% 以下（早晩 v4.1 + pre-push hook で 0% 目標）

---

## team-lead への判断要請

1. **現状維持 (enforce_admins=false)** で良いか? (commander 推奨: YES)
2. 月次レビュー仕組みを **正式運用化**するか? (commander 推奨: YES、commander 自身が毎月 1 日レビュー)
3. Phase 5 移行条件 (silent quality 通信化 + admin override < 月 2 件) を**今ロードマップに記載**するか? (commander 推奨: YES)

承認後、現状運用継続 + 月次レビュー仕組み化を進めます。

---

## 副次提案: silent quality との通信実験

team-lead 同意の上で、silent quality に **「SendMessage に応答してみてください」リクエスト**を 1 度送ってみる試行 (non-blocking)。応答があれば Phase 4 移行が早まる、なければ silent worker パターン継続。

「強制」ではなく「招待」のスタンス、quality の自主性尊重。
