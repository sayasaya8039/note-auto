@echo off
chcp 65001 > nul
REM Google News (JP) only -- 1 article
REM output: drafts\YYYY-MM-DD\<slug>.md  /  log: logs\gnews_*.log

cd /d "%~dp0"
if not exist logs mkdir logs

for /f "tokens=1-3 delims=/" %%a in ("%date%") do set D=%%a%%b%%c
for /f "tokens=1-3 delims=:. " %%a in ("%time%") do set T=%%a%%b%%c
set LOGFILE=logs\gnews_%D%_%T%.log

echo ==== note-auto gnews start: %date% %time% ==== > "%LOGFILE%"
".\target\release\note-auto.exe" --config "configs\gnews.toml" once --top 1 >> "%LOGFILE%" 2>&1
set EXITCODE=%errorlevel%
echo ==== exit code: %EXITCODE% at %date% %time% ==== >> "%LOGFILE%"
exit /b %EXITCODE%
