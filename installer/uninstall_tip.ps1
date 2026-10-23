<#
  Feihe IME - remove from the current user's keyboard list.

  input.dll has NO UninstallLayoutOrTip export, but InstallLayoutOrTip takes
  ILOT_UNINSTALL (0x00000001), which is the supported way to remove an entry.
  Registry cleanup is kept as a fallback and as a residue sweep.

  This file is UTF-8 WITH BOM on purpose (same as install_tip.ps1).
  PowerShell 5.1 reads a BOM-less .ps1 with the ANSI codepage - on Chinese
  Windows that is cp936, and UTF-8 Chinese bytes then decode as GBK, which
  can swallow the closing quote of a string and break parsing entirely
  ("字符串缺少终止符"). Do NOT re-save it as plain UTF-8 or ASCII.
  All console output is English.
#>
$ErrorActionPreference = 'Continue'

# 同 install_tip.ps1：32 位 PowerShell 会把 HKLM\SOFTWARE\Classes 重定向到
# Wow6432Node，导致「清理 32 位视图」那一段实际去删了错误的位置。自动切 64 位。
if ([IntPtr]::Size -eq 4 -and $env:PROCESSOR_ARCHITEW6432) {
    $ps64 = Join-Path $env:WINDIR 'Sysnative\WindowsPowerShell\v1.0\powershell.exe'
    if (Test-Path $ps64) {
        Write-Host "[com] 检测到 32 位 PowerShell，切换到 64 位实例重跑"
        & $ps64 -NoProfile -ExecutionPolicy Bypass -File "`"$PSCommandPath`""
        exit $LASTEXITCODE
    }
}

$Clsid   = '{70CB823D-D08A-4B57-B8BE-08BCE2539FF3}'
$Profile = '{DA6B9170-1DB7-495F-8741-D2C6F3B5E28F}'
$KbdGuid = '{34745C63-B2F0-4784-8B67-5E12C8701A31}'
$LangId  = '0x00000804'
$ILOT_UNINSTALL = 1

Write-Host "[tip] removing Feihe (clsid $Clsid)"

# -- 1. Official removal path -----------------------------------------
$sig = @"
using System;
using System.Runtime.InteropServices;
public static class FeiheInputUn {
    [DllImport("input.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern bool InstallLayoutOrTip(string psz, uint dwFlags);
}
"@
if (-not ('FeiheInputUn' -as [type])) { Add-Type -TypeDefinition $sig }

$psz = "0804:$Clsid$Profile"
$ok = [FeiheInputUn]::InstallLayoutOrTip($psz, $ILOT_UNINSTALL)
if ($ok) {
    Write-Host "  InstallLayoutOrTip(ILOT_UNINSTALL) => OK"
} else {
    $err = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    Write-Host "  InstallLayoutOrTip(ILOT_UNINSTALL) => failed, err=$err (registry cleanup will follow)"
}

# -- 2. User Profile value (the actual keyboard list) ------------------
$valueName = "0804:$Clsid$Profile"
$upKey = 'HKCU:\Control Panel\International\User Profile\zh-Hans-CN'
if (Test-Path $upKey) {
    $k = Get-Item $upKey
    $hit = @($k.GetValueNames() | Where-Object { $_ -ieq $valueName })
    if ($hit.Count -gt 0) {
        Remove-ItemProperty -Path $upKey -Name $hit[0] -Force
        Write-Host "  removed User Profile value: $($hit[0])"
    } else {
        Write-Host "  User Profile: no Feihe value (already clean)."
    }
} else {
    Write-Host "  User Profile key not found (skipped)."
}

# -- 3. SortOrder\AssemblyItem\<lang>\<kbd>\NNNNNNNN -------------------
# Only remove the leaf whose Profile matches Feihe. Never delete the whole
# <kbd> key: 00000000 is the system layout and 00000001 is Microsoft Pinyin.
$soBase = "HKCU:\Software\Microsoft\CTF\SortOrder\AssemblyItem\$LangId\$KbdGuid"
if (Test-Path $soBase) {
    $removed = $false
    foreach ($c in Get-ChildItem $soBase -ErrorAction SilentlyContinue) {
        $p = (Get-ItemProperty -LiteralPath $c.PSPath -ErrorAction SilentlyContinue).Profile
        if ($p -eq $Profile) {
            Remove-Item -LiteralPath $c.PSPath -Recurse -Force
            Write-Host "  removed SortOrder entry: $($c.PSChildName)"
            $removed = $true
        }
    }
    if (-not $removed) { Write-Host "  SortOrder: no Feihe entry (already clean)." }
} else {
    Write-Host "  SortOrder base not found (skipped)."
}

# -- 4. Assemblies -----------------------------------------------------
$asBase = "HKCU:\Software\Microsoft\CTF\Assemblies\$LangId\$KbdGuid"
if (Test-Path $asBase) {
    $removed = $false
    foreach ($c in Get-ChildItem $asBase -ErrorAction SilentlyContinue) {
        $v = (Get-ItemProperty -LiteralPath $c.PSPath -ErrorAction SilentlyContinue).CLSID
        if ($v -eq $Clsid) {
            Remove-Item -LiteralPath $c.PSPath -Recurse -Force
            Write-Host "  removed Assemblies entry: $($c.PSChildName)"
            $removed = $true
        }
    }
    if (-not $removed) { Write-Host "  Assemblies: no Feihe entry (already clean)." }
} else {
    Write-Host "  Assemblies base not found (skipped)."
}

# -- 5. Per-user TIP leftovers ----------------------------------------
$hkcuTip = "HKCU:\SOFTWARE\Microsoft\CTF\TIP\$Clsid"
if (Test-Path $hkcuTip) {
    Remove-Item $hkcuTip -Recurse -Force
    Write-Host "  removed HKCU TIP key"
} else {
    Write-Host "  HKCU TIP key not found (already clean)."
}

# -- 6. Machine-wide COM + TIP definition (needs Administrator) --------
# 64-bit view and 32-bit view (Wow6432Node) are both registered by install_tip.ps1,
# so both must be swept here — a leftover 32-bit entry would keep a stale DLL alive
# for 32-bit hosts after uninstall.
foreach ($view in @(
    @{ Name = '64'; Tip = 'HKLM:\SOFTWARE\Microsoft\CTF\TIP';                    Cls = 'HKLM:\SOFTWARE\Classes\CLSID' },
    @{ Name = '32'; Tip = 'HKLM:\SOFTWARE\Wow6432Node\Microsoft\CTF\TIP';        Cls = 'HKLM:\SOFTWARE\Classes\Wow6432Node\CLSID' }
)) {
    $hklmTip = Join-Path $view.Tip $Clsid
    if (Test-Path $hklmTip) {
        try {
            Remove-Item $hklmTip -Recurse -Force -ErrorAction Stop
            Write-Host "  removed HKLM TIP key ($($view.Name)-bit)"
        } catch {
            Write-Host "  [warn] cannot remove HKLM TIP key ($($view.Name)-bit), run as Administrator: $($_.Exception.Message)"
        }
    } else {
        Write-Host "  HKLM TIP key ($($view.Name)-bit) not found (already clean)."
    }

    $hklmClsid = Join-Path $view.Cls $Clsid
    if (Test-Path $hklmClsid) {
        try {
            Remove-Item $hklmClsid -Recurse -Force -ErrorAction Stop
            Write-Host "  removed HKLM CLSID registration ($($view.Name)-bit)"
        } catch {
            Write-Host "  [warn] cannot remove HKLM CLSID ($($view.Name)-bit), run as Administrator: $($_.Exception.Message)"
        }
    } else {
        Write-Host "  HKLM CLSID ($($view.Name)-bit) not found (already clean)."
    }
}

# -- 7. Restart ctfmon so the DLL mappings are dropped ------------------
# Symmetric to install_tip.ps1 step 5. The DLL is mapped into ctfmon.exe and
# Windows cannot delete a mapped file, so without this the uninstaller's
# Delete of feihe.dll / feihe32.dll fails silently and leaves the install
# directory behind ("removable after sign-out"). Killing ctfmon drops the
# last mapping; Windows relaunches it on demand at medium integrity.
# Kill only - never launch ctfmon from an elevated process: it would run at
# high integrity and UIPI blocks its window messages to normal apps.
try {
    Stop-Process -Name ctfmon -Force -ErrorAction Stop
    Write-Host "  ctfmon restarted (DLL mappings dropped)"
} catch {
    Write-Host "  ctfmon not running (skipped)"
}

Write-Host "[tip] removal done."
Write-Host "      Sign out and back in for the input list to refresh."
