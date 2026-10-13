@echo off
REM 飞鹤输入法 M1 安装脚本（管理员运行）
REM 前提：feihe.dll 已构建（运行 build.ps1）
setlocal

set CLSID={70CB823D-D08A-4B57-B8BE-08BCE2539FF3}
set PROFILE={DA6B9170-1DB7-495F-8741-D2C6F3B5E28F}
set KEYBOARD={34745C63-B2F0-4784-8B67-5E12C8701A31}
set TIPKEY=HKLM\SOFTWARE\Microsoft\CTF\TIP\%CLSID%
set DLL=%~dp0..\target\x86_64-pc-windows-gnullvm\release\feihe.dll
set NAME=飞鹤拼音

if not exist "%DLL%" (
  echo [错误] 找不到 %DLL%
  echo 请先运行 build.ps1 构建。
  exit /b 1
)

REM ========== 0) 清理旧版/错误结构（幂等）==========
reg delete "%TIPKEY%\0804" /f >nul 2>&1
reg delete "%TIPKEY%\Enable" /f >nul 2>&1
reg delete "%TIPKEY%\Category\{6A497209-DF5C-4CF4-A7AA-3E2A11198019}" /f >nul 2>&1

REM ========== 1) COM 注册 ==========
reg add "HKCR\CLSID\%CLSID%" /ve /d "%NAME%" /f
reg add "HKCR\CLSID\%CLSID%\InprocServer32" /ve /d "%DLL%" /f
reg add "HKCR\CLSID\%CLSID%\InprocServer32" /v "ThreadingModel" /d "Apartment" /f

REM ========== 2) 语言配置 LanguageProfile ==========
set LPKEY=%TIPKEY%\LanguageProfile\0x00000804\%PROFILE%
reg add "%LPKEY%" /f
reg add "%LPKEY%" /v "Description" /d "%NAME%" /f
reg add "%LPKEY%" /v "Enable" /t REG_DWORD /d 1 /f
reg add "%LPKEY%" /v "IconFile" /t REG_EXPAND_SZ /d "%DLL%,-101" /f
reg add "%LPKEY%" /v "IconIndex" /t REG_DWORD /d 0 /f

REM ========== 3) 类别：键盘输入法 ==========
set CATKEY=%TIPKEY%\Category\Category\%KEYBOARD%\%CLSID%
reg add "%CATKEY%" /f
reg add "%TIPKEY%\Category\Item\%CLSID%" /v "Description" /d "%NAME%" /f

REM ========== 4) 当前用户语言档案（使设置里立即出现）==========
set UPKEY=HKCU\Control Panel\International\User Profile
reg add "%UPKEY%\zh-Hans-CN" /v "0804:%CLSID%%PROFILE%" /t REG_DWORD /d 4 /f

echo.
echo 注册完成。请到 设置 - 时间和语言 - 语言和区域 - 中文(简体,中国) - 语言选项 - 键盘
echo 里添加/启用 "%NAME%"。若列表未见，注销后重新登录再看。
echo 卸载：installer\unregister.cmd
endlocal
