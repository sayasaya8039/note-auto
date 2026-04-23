@echo off
REM note-auto once wrapper for Task Scheduler
REM sets working directory then runs, saves log to logs\*.log

cd /d "%~dp0"
if not exist logs mkdir logs

for /f "tokens=1-3 delims=/" %%a in ("%date%") do set D=%%a%%b%%c
for /f "tokens=1-3 delims=:. " %%a in ("%time%") do set T=%%a%%b%%c
set LOGFILE=logs\%D%_%T%.log

echo ==== note-auto once start: %date% %time% ==== > "%LOGFILE%"
".\target\release\note-auto.exe" once >> "%LOGFILE%" 2>&1
set EXITCODE=%errorlevel%
echo ==== exit code: %EXITCODE% at %date% %time% ==== >> "%LOGFILE%"
exit /b %EXITCODE%
