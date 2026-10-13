@echo off
REM 飞鹤输入法卸载（管理员运行）
setlocal

set CLSID={70CB823D-D08A-4B57-B8BE-08BCE2539FF3}
set PROFILE={DA6B9170-1DB7-495F-8741-D2C6F3B5E28F}

REM COM
reg delete "HKCR\CLSID\%CLSID%" /f >nul 2>&1

REM TSF TIP
reg delete "HKLM\SOFTWARE\Microsoft\CTF\TIP\%CLSID%" /f >nul 2>&1

REM 当前用户语言档案
reg delete "HKCU\Control Panel\International\User Profile\zh-Hans-CN" /v "0804:%CLSID%%PROFILE%" /f >nul 2>&1

echo 已卸载，注销或重启后生效。
endlocal
