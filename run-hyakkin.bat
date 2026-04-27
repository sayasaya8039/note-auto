@echo off
chcp 65001 > nul
REM 100yen shops (Daiso/Seria/CanDo/Watts) only -- 1 article + source images
REM output: drafts\YYYY-MM-DD\<slug>.md  /  log: logs\hyakkin_*.log

cd /d "%~dp0"
if not exist logs mkdir logs

for /f "tokens=1-3 delims=/" %%a in ("%date%") do set D=%%a%%b%%c
for /f "tokens=1-3 delims=:. " %%a in ("%time%") do set T=%%a%%b%%c
set LOGFILE=logs\hyakkin_%D%_%T%.log

echo ==== note-auto hyakkin start: %date% %time% ==== > "%LOGFILE%"
".\target\release\note-auto.exe" --config "configs\hyakkin.toml" once --top 1 >> "%LOGFILE%" 2>&1
set EXITCODE=%errorlevel%
echo ==== exit code: %EXITCODE% at %date% %time% ==== >> "%LOGFILE%"
exit /b %EXITCODE%
