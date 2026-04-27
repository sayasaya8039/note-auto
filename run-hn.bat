@echo off
REM Hacker News 単独リサーチ — 1 記事生成
REM 出力: drafts\YYYY-MM-DD\<slug>.md  /  ログ: logs\hn_*.log

cd /d "%~dp0"
if not exist logs mkdir logs

for /f "tokens=1-3 delims=/" %%a in ("%date%") do set D=%%a%%b%%c
for /f "tokens=1-3 delims=:. " %%a in ("%time%") do set T=%%a%%b%%c
set LOGFILE=logs\hn_%D%_%T%.log

echo ==== note-auto hn start: %date% %time% ==== > "%LOGFILE%"
".\target\release\note-auto.exe" --config "configs\hn.toml" once --top 1 >> "%LOGFILE%" 2>&1
set EXITCODE=%errorlevel%
echo ==== exit code: %EXITCODE% at %date% %time% ==== >> "%LOGFILE%"
exit /b %EXITCODE%
