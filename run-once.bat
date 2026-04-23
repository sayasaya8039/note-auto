@echo off
REM note-auto once ラッパー — Task Scheduler 用
REM ワーキングディレクトリを明示的に設定してから実行する
REM 実行ログは logs/YYYYMMDD_HHMMSS.log に保存

cd /d "%~dp0"
if not exist logs mkdir logs

REM 日時を YYYYMMDD_HHMMSS 形式で取得
for /f "tokens=1-3 delims=/" %%a in ("%date%") do set D=%%a%%b%%c
for /f "tokens=1-3 delims=:. " %%a in ("%time%") do set T=%%a%%b%%c
set LOGFILE=logs\%D%_%T%.log

echo ==== note-auto once start: %date% %time% ==== > "%LOGFILE%"
".\target\release\note-auto.exe" once >> "%LOGFILE%" 2>&1
set EXITCODE=%errorlevel%
echo ==== exit code: %EXITCODE% at %date% %time% ==== >> "%LOGFILE%"
exit /b %EXITCODE%
