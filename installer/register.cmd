@echo off
REM 飞鹤输入法 M1 安装脚本（管理员运行）
REM 前提：feihe.dll 已构建（cargo build --target x86_64-pc-windows-msvc --release）

set CLSID={70CB823D-D08A-4B57-B8BE-08BCE2539FF3}
set PROFILE={DA6B9170-1DB7-495F-8741-D2C6F3B5E28F}
set LANGID=0804
set DLL=%~dp0..\target\x86_64-pc-windows-msvc\release\feihe.dll
set CLSID_NOSTR={70CB823D-D08A-4B57-B8BE-08BCE2539FF3}

REM 1) COM 注册
reg add "HKCR\CLSID\%CLSID%" /ve /d "飞鹤拼音" /f
reg add "HKCR\CLSID\%CLSID%\InprocServer32" /ve /d "%DLL%" /f
reg add "HKCR\CLSID\%CLSID%\InprocServer32" /v "ThreadingModel" /d "Apartment" /f

REM 2) CTF TIP 注册（输入法条目）
reg add "HKLM\SOFTWARE\Microsoft\CTF\TIP\%CLSID%" /f
reg add "HKLM\SOFTWARE\Microsoft\CTF\TIP\%CLSID%\Enable" /v "00000804" /t REG_DWORD /d 1 /f
reg add "HKLM\SOFTWARE\Microsoft\CTF\TIP\%CLSID%\%LANGID%" /f
reg add "HKLM\SOFTWARE\Microsoft\CTF\TIP\%CLSID%\%LANGID%\%PROFILE%" /f
reg add "HKLM\SOFTWARE\Microsoft\CTF\TIP\%CLSID%\%LANGID%\%PROFILE%" /v "Enable" /t REG_DWORD /d 1 /f
reg add "HKLM\SOFTWARE\Microsoft\CTF\TIP\%CLSID%\%LANGID%\%PROFILE%" /v "Description" /d "飞鹤拼音" /f
reg add "HKLM\SOFTWARE\Microsoft\CTF\TIP\%CLSID%\%LANGID%\%PROFILE%" /v "IconFile" /d "%%SystemRoot%%\System32\imjp9_0.dll" /f
reg add "HKLM\SOFTWARE\Microsoft\CTF\TIP\%CLSID%\%LANGID%\%PROFILE%" /v "IconIndex" /t REG_DWORD /d 0 /f

REM 3) 类别：键盘输入法（CTF 分类）
reg add "HKLM\SOFTWARE\Microsoft\CTF\TIP\%CLSID%\Category" /f
reg add "HKLM\SOFTWARE\Microsoft\CTF\TIP\%CLSID%\Category\{6A497209-DF5C-4CF4-A7AA-3E2A11198019}" /f
reg add "HKLM\SOFTWARE\Microsoft\CTF\TIP\%CLSID%\Category\{6A497209-DF5C-4CF4-A7AA-3E2A11198019}\{34745C9C-66ED-42A3-90CD-2F34F1C0BA4B}" /f

echo.
echo 注册完成。到 Windows 设置 - 时间和语言 - 语言 - 中文 - 键盘 里启用 "飞鹤拼音"
echo 卸载：installer\unregister.cmd
