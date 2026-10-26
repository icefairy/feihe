@echo off
REM Feihe IME - unregister (per-user, NO Administrator required)
REM
REM 只删程序文件，**不删配置 / 词频 / 自造词 / 置顶**——那是用户自己的打字习惯，
REM 重装还要用，而且整个目录还留着当备份。
setlocal

set "DESTDIR=%LOCALAPPDATA%\Feihe"

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
