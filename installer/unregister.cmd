@echo off
REM 飞鹤输入法卸载（管理员运行）
set CLSID={70CB823D-D08A-4B57-B8BE-08BCE2539FF3}
reg delete "HKLM\SOFTWARE\Microsoft\CTF\TIP\%CLSID%" /f
reg delete "HKCR\CLSID\%CLSID%" /f
echo 已卸载，注销或重启后生效。
