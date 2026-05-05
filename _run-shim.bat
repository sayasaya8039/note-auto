@echo off
REM ─────────────────────────────────────────────────────────────────────
REM note-auto shared run shim (v0.7.6)
REM
REM Inputs (caller sets via `set` before `call "%~dp0_run-shim.bat"`):
REM   CAT  : category slug (note|x|google|hn|konbini|hyakkin|gnews|all)
REM   TOP  : --top value (1〜7)
REM
REM Output: logs\<CAT>_<date>_<time>.log
REM Exit  : note-auto.exe の exitcode を伝播
REM
REM 互換: U3 完了 (main.rs の `run --category` 実装) 後、本ファイルだけを
REM       書き換えれば全 8 本の bat が一括で `run --category %CAT%` に切り替わる。
REM ─────────────────────────────────────────────────────────────────────

chcp 65001 > nul
cd /d "%~dp0"
if not exist logs mkdir logs

for /f "tokens=1-3 delims=/" %%a in ("%date%") do set D=%%a%%b%%c
for /f "tokens=1-3 delims=:. " %%a in ("%time%") do set T=%%a%%b%%c
set LOGFILE=logs\%CAT%_%D%_%T%.log

if "%CAT%"=="all" (
    set CONFIGFILE=config.toml
) else (
    set CONFIGFILE=configs\%CAT%.toml
)

echo ==== note-auto %CAT% start: %date% %time% ==== > "%LOGFILE%"
".\target\release\note-auto.exe" --config "%CONFIGFILE%" once --top %TOP% >> "%LOGFILE%" 2>&1
set EXITCODE=%errorlevel%
echo ==== exit code: %EXITCODE% at %date% %time% ==== >> "%LOGFILE%"
exit /b %EXITCODE%
