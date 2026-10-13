@echo off
REM 飞鹤输入法卸载（管理员运行）
setlocal

set CLSID={70CB823D-D08A-4B57-B8BE-08BCE2539FF3}
set PROFILE={DA6B9170-1DB7-495F-8741-D2C6F3B5E28F}
set KEYBOARD={34745C63-B2F0-4784-8B67-5E12C8701A31}
set TIP=0804:%CLSID%%PROFILE%

REM 从当前用户输入法列表移除（官方 API）
powershell -NoProfile -ExecutionPolicy Bypass -Command "$s=@'
using System;using System.Runtime.InteropServices;
public static class U{[DllImport(\"input.dll\",CharSet=CharSet.Unicode,SetLastError=true)]public static extern bool UninstallLayoutOrTip(string p,uint f);}
'@;Add-Type -TypeDefinition $s;[U]::UninstallLayoutOrTip('%TIP%',0) | Out-Null"

REM COM
reg delete "HKCR\CLSID\%CLSID%" /f >nul 2>&1

REM TSF TIP
reg delete "HKLM\SOFTWARE\Microsoft\CTF\TIP\%CLSID%" /f >nul 2>&1
reg delete "HKCU\SOFTWARE\Microsoft\CTF\TIP\%CLSID%" /f >nul 2>&1

echo 已卸载，注销或重启后生效。
endlocal
