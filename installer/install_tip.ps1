# 把飞鹤 TIP 注册进当前用户的输入法列表（等效于"设置-添加键盘"）
# 调用 input.dll!InstallLayoutOrTip
param(
    [string]$Tip = "0804:{70CB823D-D08A-4B57-B8BE-08BCE2539FF3}{DA6B9170-1DB7-495F-8741-D2C6F3B5E28F}"
)

$sig = @"
using System;
using System.Runtime.InteropServices;
public static class FeiheInput {
    [DllImport("input.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    public static extern bool InstallLayoutOrTip(string psz, uint dwFlags);
    [DllImport("input.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    public static extern bool UninstallLayoutOrTip(string psz, uint dwFlags);
}
"@
Add-Type -TypeDefinition $sig

$ok = [FeiheInput]::InstallLayoutOrTip($Tip, 0)
if ($ok) {
    Write-Host "[tip] InstallLayoutOrTip OK: $Tip"
} else {
    $err = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    Write-Host "[tip] InstallLayoutOrTip 失败, err=$err" -ForegroundColor Red
}
