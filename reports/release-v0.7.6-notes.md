## 🚀 Performance
- LTO fat + panic=abort + mimalloc → release binary 5-15% faster, 20-30% smaller
- HTTP client shared (`LazyLock<Client>`) — TLS handshake reuse across 11 trend sources
- tokio features minimized, quick-xml dependency removed

## 🎨 UI / UX
- macOS Big Sur dark theme CLI palette (#007AFF accent)
- New flags: `--color={auto,always,never}` / `--ascii` / `NO_COLOR` env
- New subcommand: `note-auto run --category <name>` (replaces 9 separate bats)
- 9 batch files reduced to thin shims (Task Scheduler compatible)
- 3-layer logging: JSON file appender + tracing→progress bridge + stderr WARN/ERROR

## 🔧 Hotfix
- Restored `gemini/nvidia` modules referenced by `ai/mod.rs` and `writer` (PR #4)

## 📝 Next: v0.7.7
- quality team to wire up indicatif/comfy-table/owo-colors/console/supports-color
- Q1-Q4: bug fixes, unwrap safety, dedup HashMap, refactor

## ⚠ Compat
- Old cmd.exe (Raster Fonts): use `--ascii` flag for box characters
- Bat argument spec preserved
