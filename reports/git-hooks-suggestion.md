# Git Hooks 推奨設定（任意）

> **目的**: v4.1 ルール（`git status -sb` 確認義務化）の補強として、ローカル `.git/hooks/` で物理ガード。各人が任意で adopt。
> **対象**: lowlevel / ui-macos / quality / commander の各 worktree
> **採用条件**: 任意（`git status -sb` 確認だけで十分なら不要）

---

## pre-push hook: main / master 直 push を物理ブロック

`.git/hooks/pre-push` (実行権限 `chmod +x` 付与必須):

```sh
#!/bin/sh
# Block direct push to main / master from local
branch=$(git symbolic-ref --short HEAD 2>/dev/null)
if [ "$branch" = "main" ] || [ "$branch" = "master" ]; then
  echo ""
  echo "  🚫 ERROR: direct push to '$branch' is blocked locally."
  echo "  Please use a feature branch:"
  echo ""
  echo "      git checkout -b feat/<feature-name>"
  echo "      git status -sb         # confirm branch"
  echo "      git push -u origin feat/<feature-name>"
  echo ""
  echo "  This is a local safety guard (not in remote branch protection)."
  echo "  To bypass for emergency: --no-verify (admin override only)"
  echo ""
  exit 1
fi
exit 0
```

### インストール手順

#### Linux / macOS / WSL / Git Bash
```bash
cd /path/to/note-auto
cat > .git/hooks/pre-push <<'EOF'
#!/bin/sh
branch=$(git symbolic-ref --short HEAD 2>/dev/null)
if [ "$branch" = "main" ] || [ "$branch" = "master" ]; then
  echo "  🚫 ERROR: direct push to '$branch' blocked. Use feature branch."
  exit 1
fi
exit 0
EOF
chmod +x .git/hooks/pre-push
```

#### Windows (PowerShell)
```powershell
cd D:\NEXTCLOUD\Windows_app\note-auto
@"
#!/bin/sh
branch=`$(git symbolic-ref --short HEAD 2>/dev/null)
if [ "`$branch" = "main" ] || [ "`$branch" = "master" ]; then
  echo "  🚫 ERROR: direct push to '`$branch' blocked. Use feature branch."
  exit 1
fi
exit 0
"@ | Out-File -Encoding ASCII -NoNewline .git/hooks/pre-push
```

#### worktree の場合
**注意**: `git worktree add` で作成した worktree は `.git` がファイル (`gitdir:` ポインタ) になる。hook は **共通 `.git/` ディレクトリ**にあるため、main worktree で 1 度設定すれば全 worktree に効く。

```bash
# main worktree (note-auto/) で 1 回設定 → 全 worktree (note-auto-tui/, note-auto-bench/ 等) に効く
cd /path/to/note-auto
ls -la .git/hooks/pre-push   # 確認
```

### 動作確認
```bash
git checkout main
git commit --allow-empty -m "test"
git push origin main
# → "🚫 ERROR: direct push to 'main' blocked. Use feature branch." で終了
```

### 緊急 bypass
admin override (silent quality / 緊急 hot-fix) が必要な場合:
```bash
git push --no-verify origin main
# → hook を bypass、push 通る（記録は admin-override-log.md に必須）
```

---

## pre-commit hook (任意・高度): branch 確認

`.git/hooks/pre-commit`:

```sh
#!/bin/sh
branch=$(git symbolic-ref --short HEAD 2>/dev/null)
if [ "$branch" = "main" ] || [ "$branch" = "master" ]; then
  echo ""
  echo "  ⚠️ WARNING: committing to '$branch' directly."
  echo "  Press Enter to continue, Ctrl+C to abort."
  echo ""
  read confirmation
fi
exit 0
```

→ commit 段階で confirmation を求める。push hook より早い段階のガード。ただしインタラクティブなので CI / 自動化には不向き。

---

## 共有設定の検討（リポジトリレベル）

`pre-push` を全員に強制したい場合は **`pre-commit` フレームワーク** や **husky** 等を使えば共有可能だが:
- Rust プロジェクトでは過剰になりがち
- 現状は **任意設定**で各人が adopt するスタイルが note-auto の運用に合う

---

## 関連

- 通信ルール v3 + v4.1: `git status -sb` 確認義務化
- 学習スキル: `~/.claude/skills/learned/accidental-main-direct-commit-recovery.md`
- branch protection: https://github.com/sayasaya8039/note-auto/settings/branches
- 過去事例: `reports/admin-override-log.md` の `4ba4ddb` (lowlevel accidental main commit)
