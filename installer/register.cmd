@echo off
REM Feihe IME - register (run as Administrator)
REM Prerequisite: feihe.dll built by build.ps1
REM
REM Deploys BOTH architectures:
REM   feihe.dll   (64-bit) -> registered in HKLM\SOFTWARE\Classes\CLSID
REM   feihe32.dll (32-bit) -> registered in HKLM\SOFTWARE\Classes\Wow6432Node\CLSID
REM In-proc COM requires the DLL bitness to match the host process, so a 64-bit
REM Explorer and a 32-bit Office need two different files behind the same CLSID.
setlocal

set "SRC64=%~dp0..\target\x86_64-pc-windows-gnullvm\release\feihe.dll"
set "SRC32=%~dp0..\target\i686-pc-windows-gnullvm\release\feihe.dll"
set "DESTDIR=%ProgramFiles%\Feihe"
set "DEST64=%DESTDIR%\feihe.dll"
set "DEST32=%DESTDIR%\feihe32.dll"

REM --- Administrator check ---
fltmc >nul 2>&1
if errorlevel 1 (
  echo [ERROR] Administrator privileges required.
  echo         Right-click this file and choose "Run as administrator".
  exit /b 1
)

if not exist "%SRC64%" (
  echo [ERROR] %SRC64% not found.
  echo         Build it first:  powershell -ExecutionPolicy Bypass -File build.ps1
  exit /b 1
)

if not exist "%DESTDIR%" mkdir "%DESTDIR%" >nul 2>&1

REM --- Deploy to a stable path; target\ is wiped by cargo clean ---
call :deploy "%SRC64%" "%DEST64%"
if errorlevel 1 exit /b 1
echo [dll] %DEST64%

if exist "%SRC32%" (
  call :deploy "%SRC32%" "%DEST32%"
  if errorlevel 1 exit /b 1
  echo [dll] %DEST32%
  REM --- Registry work is done in PowerShell (UTF-8 BOM keeps the Chinese name intact) ---
  powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install_tip.ps1" -DllPath "%DEST64%" -DllPath32 "%DEST32%"
) else (
  echo [warn] 32-bit DLL not found: %SRC32%
  echo        Only 64-bit hosts will work. To build it:
  echo            powershell -ExecutionPolicy Bypass -File build.ps1 -Arch x86
  powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install_tip.ps1" -DllPath "%DEST64%"
)
if errorlevel 1 (
  echo [ERROR] registration failed.
  exit /b 1
)

endlocal
exit /b 0

REM ------------------------------------------------------------------
:deploy
REM %1 = source dll, %2 = destination dll
copy /y %1 %2 >nul 2>&1
if not errorlevel 1 exit /b 0
REM DLL may be loaded by ctfmon/explorer; Windows allows renaming a mapped file.
echo [warn] %~nx2 is in use, renaming the old copy and retrying...
if exist %2.old del /f /q %2.old >nul 2>&1
move /y %2 %2.old >nul 2>&1
copy /y %1 %2 >nul 2>&1
if not exist %2 (
  echo [ERROR] failed to deploy %1 to %2
  exit /b 1
)
exit /b 0
