@echo off
REM Feihe IME - unregister (run as Administrator)
setlocal

set "DESTDIR=%ProgramFiles%\Feihe"

REM --- Administrator check ---
fltmc >nul 2>&1
if errorlevel 1 (
  echo [ERROR] Administrator privileges required.
  echo         Right-click this file and choose "Run as administrator".
  exit /b 1
)

REM --- Remove from the user keyboard list + COM/TIP registration ---
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0uninstall_tip.ps1"

REM --- Drop the deployed dlls ---
for %%F in (feihe.dll feihe32.dll feihe.dll.old feihe32.dll.old) do (
  if exist "%DESTDIR%\%%F" (
    del /f /q "%DESTDIR%\%%F" >nul 2>&1
    if errorlevel 1 echo [warn] %%F still loaded; it will be removable after sign-out.
  )
)

echo.
echo [done] Feihe IME unregistered.
endlocal
