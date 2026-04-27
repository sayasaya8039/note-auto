@echo off
REM コンビニ来週新商品 単独リサーチ — 1 記事生成 + 公式画像 inline 利用
REM 出力: drafts\YYYY-MM-DD\<slug>.md  /  ログ: logs\konbini_*.log

cd /d "%~dp0"
if not exist logs mkdir logs

for /f "tokens=1-3 delims=/" %%a in ("%date%") do set D=%%a%%b%%c
for /f "tokens=1-3 delims=:. " %%a in ("%time%") do set T=%%a%%b%%c
set LOGFILE=logs\konbini_%D%_%T%.log

echo ==== note-auto konbini start: %date% %time% ==== > "%LOGFILE%"
".\target\release\note-auto.exe" --config "configs\konbini.toml" once --top 1 >> "%LOGFILE%" 2>&1
set EXITCODE=%errorlevel%
echo ==== exit code: %EXITCODE% at %date% %time% ==== >> "%LOGFILE%"
exit /b %EXITCODE%
