@echo off
REM 全7ソース並列リサーチ — 既定 top=3 記事生成
REM 出力: drafts\YYYY-MM-DD\<slug>.md  /  ログ: logs\all_*.log
REM 設定: config.toml (デフォルト全ソース有効)

cd /d "%~dp0"
if not exist logs mkdir logs

for /f "tokens=1-3 delims=/" %%a in ("%date%") do set D=%%a%%b%%c
for /f "tokens=1-3 delims=:. " %%a in ("%time%") do set T=%%a%%b%%c
set LOGFILE=logs\all_%D%_%T%.log

echo ==== note-auto all start: %date% %time% ==== > "%LOGFILE%"
".\target\release\note-auto.exe" --config "config.toml" once --top 3 >> "%LOGFILE%" 2>&1
set EXITCODE=%errorlevel%
echo ==== exit code: %EXITCODE% at %date% %time% ==== >> "%LOGFILE%"
exit /b %EXITCODE%
