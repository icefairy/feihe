<#
  Feihe IME - install / register (requires Administrator).

  What it does:
    1. COM registration (HKLM\SOFTWARE\Classes\CLSID\{...}) pointing at a STABLE dll path.
       32-bit host 走 Wow6432Node 那一份（见下方 DllPath32 说明）。
    2. HKLM CTF TIP definition (Category / Item / LanguageProfile)，32 位视图镜像一份。
    3. Adds Feihe to the CURRENT USER keyboard list via input.dll!InstallLayoutOrTip.
    4. Verifies the result and falls back to a direct registry write if needed.

  Notes:
    - input.dll has NO UninstallLayoutOrTip export. Removal uses the same entry point
      with ILOT_UNINSTALL (0x1). See uninstall_tip.ps1.
    - InstallLayoutOrTipUserReg exists but returns FALSE on this machine, so the
      plain InstallLayoutOrTip is used instead (verified to write User Profile +
      SortOrder correctly).
    - psz format is "0804:{CLSID}{PROFILE}" (LangID without 0x prefix is accepted).

  DllPath32（可选）——32 位宿主支持：
    in-proc COM 的位数必须和宿主一致，所以 64/32 位要各自一份 DLL。
    给 -DllPath32 时本脚本会额外写：
      HKLM\SOFTWARE\Classes\Wow6432Node\CLSID\{CLSID}\InprocServer32 -> feihe32.dll
      HKLM\SOFTWARE\Wow6432Node\Microsoft\CTF\TIP\{CLSID}            (镜像)
    ⚠ HKLM\SOFTWARE\Microsoft\CTF\TIP 是被 WOW64 重定向的（不在共享键列表里），
      32 位进程读的是 Wow6432Node 那份，所以 TIP 定义必须写两遍。

  This file is UTF-8 WITH BOM on purpose: PowerShell 5.1 needs the BOM to read the
  Chinese display name correctly. Do not re-save as plain UTF-8 or ASCII.
#>
param(
    [Parameter(Mandatory = $true)][string]$DllPath,
    [string]$DllPath32
)

$ErrorActionPreference = 'Stop'

# ── 必须是 64 位 PowerShell ──────────────────────────────────────
# 本脚本要同时写两个注册表视图：HKLM\SOFTWARE\Classes\CLSID（64 位视图）
# 和 HKLM\SOFTWARE\Classes\Wow6432Node\CLSID（32 位视图）。
# 如果自己就跑在 32 位 PowerShell 里，WOW64 会把 HKLM\SOFTWARE\Classes 重定向到
# Wow6432Node，于是我们写的 "Wow6432Node\..." 会变成 Wow6432Node\Wow6432Node\...，
# 32 位宿主反而查不到。而 NSIS / 32 位 cmd 启动 powershell 时拿到的正是 32 位实例，
# 所以这里自动切到 64 位实例重跑。
if ([IntPtr]::Size -eq 4 -and $env:PROCESSOR_ARCHITEW6432) {
    $ps64 = Join-Path $env:WINDIR 'Sysnative\WindowsPowerShell\v1.0\powershell.exe'
    if (Test-Path $ps64) {
        Write-Host "[com] 检测到 32 位 PowerShell，切换到 64 位实例重跑"
        $a = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"",
               '-DllPath', "`"$DllPath`"")
        if ($DllPath32) { $a += @('-DllPath32', "`"$DllPath32`"") }
        & $ps64 @a
        exit $LASTEXITCODE
    }
}

$Clsid   = '{70CB823D-D08A-4B57-B8BE-08BCE2539FF3}'
$Profile = '{DA6B9170-1DB7-495F-8741-D2C6F3B5E28F}'
$KbdGuid = '{34745C63-B2F0-4784-8B67-5E12C8701A31}'
$LangId  = '0x00000804'
$Name    = '飞鹤拼音'

if (-not (Test-Path -LiteralPath $DllPath)) {
    throw "DLL not found: $DllPath"
}
$DllPath = (Resolve-Path -LiteralPath $DllPath).Path

if ($DllPath32) {
    if (-not (Test-Path -LiteralPath $DllPath32)) {
        throw "32-bit DLL not found: $DllPath32"
    }
    $DllPath32 = (Resolve-Path -LiteralPath $DllPath32).Path
}

# Registry values need an explicit type. Set-ItemProperty has NO -PropertyType
# parameter (only New-ItemProperty does), so every typed write goes through this
# helper: it drops any existing value first, then creates it with the requested
# type. That also keeps the script idempotent when re-run.
function Set-RegValue {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$ValueName,
        $Value,
        [string]$Type = 'String'
    )
    if (-not (Test-Path -LiteralPath $Path)) { throw "registry key not found: $Path" }
    if ($ValueName -eq '(default)') {
        Set-ItemProperty -LiteralPath $Path -Name '(default)' -Value $Value
        return
    }
    $key = Get-Item -LiteralPath $Path
    if (@($key.GetValueNames()) -contains $ValueName) {
        Remove-ItemProperty -LiteralPath $Path -Name $ValueName -Force
    }
    New-ItemProperty -LiteralPath $Path -Name $ValueName -Value $Value -PropertyType $Type | Out-Null
}

# -- 1 & 2. COM + CTF TIP 注册（同一份逻辑跑两遍：64 位视图 / 32 位视图）-----
#
# 参数化的原因：WOW64 把 HKLM\SOFTWARE\Classes 和 HKLM\SOFTWARE\Microsoft\CTF\TIP
# 都重定向了（CTF 只有 SystemShared 是共享键）。64 位宿主查前者，32 位宿主查
# Wow6432Node 那份，两边必须各自有一套完整的 CLSID + TIP 定义。
function Register-TipView {
    param(
        [Parameter(Mandatory = $true)][string]$ClassesRoot,
        [Parameter(Mandatory = $true)][string]$CtfTipRoot,
        [Parameter(Mandatory = $true)][string]$Dll
    )

    # 1. COM registration
    $clsidKey = Join-Path $ClassesRoot "CLSID\$Clsid"
    New-Item -Path $clsidKey -Force | Out-Null
    Set-RegValue -Path $clsidKey -ValueName '(default)' -Value $Name

    $inproc = Join-Path $clsidKey 'InprocServer32'
    New-Item -Path $inproc -Force | Out-Null
    Set-RegValue -Path $inproc -ValueName '(default)' -Value $Dll
    Set-RegValue -Path $inproc -ValueName 'ThreadingModel' -Value 'Apartment'
    Write-Host "[com] $ClassesRoot -> $Dll"

    # 2. CTF TIP definition
    $tip = Join-Path $CtfTipRoot $Clsid

    # 2a. Category: mark this TIP as a keyboard input method
    # 只注册 KEYBOARD 是不够的。Windows 8 以后引入了 capability 分类，
    # 特别是 IMMERSIVESUPPORT —— 缺它的话 UWP / WinUI3 应用（例如 Windows 11
    # 记事本）不会把按键转给 TIP：Activate 照常调用、key sink 也挂上了，
    # 但一个 OnTestKeyDown 都收不到，表现为「切到输入法后字母直接进输入框」。
    $ImmGuid = '{13A016DF-560B-46CD-947A-4C3AF1E0E35D}'   # GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT

    $categories = @($KbdGuid, $ImmGuid)
    $item = Join-Path $tip "Category\Item\$Clsid"
    New-Item -Path $item -Force | Out-Null
    Set-RegValue -Path $item -ValueName 'Description' -Value $Name
    foreach ($cat in $categories) {
        New-Item -Path (Join-Path $tip "Category\Category\$cat\$Clsid") -Force | Out-Null
        New-Item -Path (Join-Path $item $cat) -Force | Out-Null
    }
    Write-Host ("[tip] $CtfTipRoot : category registered (" + ($categories -join ' ') + ")")

    # 2b. LanguageProfile: the entry that shows up under the language
    $lp = Join-Path $tip "LanguageProfile\$LangId\$Profile"
    New-Item -Path $lp -Force | Out-Null
    Set-RegValue -Path $lp -ValueName 'Description' -Value $Name
    Set-RegValue -Path $lp -ValueName 'Enable' -Value 1 -Type DWord
    # 图标统一取 64 位那份（两份 DLL 的资源一样，指同一路径少一个失效点）
    Set-RegValue -Path $lp -ValueName 'IconFile' -Value "$script:IconDll,-101" -Type ExpandString
    Set-RegValue -Path $lp -ValueName 'IconIndex' -Value 0 -Type DWord
}

$script:IconDll = $DllPath

Register-TipView -ClassesRoot 'HKLM:\SOFTWARE\Classes' `
                 -CtfTipRoot  'HKLM:\SOFTWARE\Microsoft\CTF\TIP' `
                 -Dll         $DllPath

if ($DllPath32) {
    Register-TipView -ClassesRoot 'HKLM:\SOFTWARE\Classes\Wow6432Node' `
                     -CtfTipRoot  'HKLM:\SOFTWARE\Wow6432Node\Microsoft\CTF\TIP' `
                     -Dll         $DllPath32
    Write-Host "[com] 32-bit host support enabled"
} else {
    Write-Host "[com] 未提供 32 位 DLL，32 位宿主（32 位 Office / IE 插件等）无法使用飞鹤" -ForegroundColor Yellow
}

# -- 3. Add to the current user's keyboard list -----------------------
# A stale per-user profile (Enable = 0) left behind by an older uninstall makes the
# entry invisible even after a successful install. Drop it first.
$hkcuTip = "HKCU:\SOFTWARE\Microsoft\CTF\TIP\$Clsid"
if (Test-Path $hkcuTip) {
    Remove-Item $hkcuTip -Recurse -Force
    Write-Host "[tip] removed stale HKCU TIP key (had Enable=0 leftovers)"
}

$sig = @"
using System;
using System.Runtime.InteropServices;
public static class FeiheInput {
    [DllImport("input.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern bool InstallLayoutOrTip(string psz, uint dwFlags);
}
"@
if (-not ('FeiheInput' -as [type])) { Add-Type -TypeDefinition $sig }

$psz = "0804:$Clsid$Profile"
$ok = [FeiheInput]::InstallLayoutOrTip($psz, 0)
if ($ok) {
    Write-Host "[tip] InstallLayoutOrTip($psz, 0) => OK"
} else {
    $err = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    Write-Host "[tip] InstallLayoutOrTip FAILED, err=$err (will use registry fallback)" -ForegroundColor Yellow
}

# -- 4. Verify + fallback ---------------------------------------------
$valueName = "0804:$Clsid$Profile"

# 4a. HKCU\Control Panel\International\User Profile\<lang> holds the real keyboard list
$upKey = 'HKCU:\Control Panel\International\User Profile\zh-Hans-CN'
$present = $false
if (Test-Path $upKey) {
    $k = Get-Item $upKey
    $present = @($k.GetValueNames() | Where-Object { $_ -ieq $valueName }).Count -gt 0
}
if ($present) {
    Write-Host "[ok] keyboard list contains Feihe"
} else {
    if (-not (Test-Path $upKey)) { New-Item -Path $upKey -Force | Out-Null }
    $k = Get-Item $upKey
    $next = 1
    foreach ($n in $k.GetValueNames()) {
        if ($n -like '0804:*' -and $k.GetValueKind($n) -eq 'DWord') {
            $v = [int]$k.GetValue($n)
            if ($v -ge $next) { $next = $v + 1 }
        }
    }
    Set-RegValue -Path $upKey -ValueName $valueName -Value $next -Type DWord
    Write-Host "[fix] wrote User Profile entry '$valueName' = $next"
}

# 4b. SortOrder controls the position in the input switcher
$soBase = "HKCU:\Software\Microsoft\CTF\SortOrder\AssemblyItem\$LangId\$KbdGuid"
$hasSort = $false
if (Test-Path $soBase) {
    foreach ($c in Get-ChildItem $soBase) {
        $p = (Get-ItemProperty -LiteralPath $c.PSPath -ErrorAction SilentlyContinue).Profile
        if ($p -eq $Profile) { $hasSort = $true; break }
    }
}
if ($hasSort) {
    Write-Host "[ok] SortOrder contains Feihe"
} else {
    New-Item -Path $soBase -Force | Out-Null
    $idx = 0
    $existing = @(Get-ChildItem $soBase | ForEach-Object { [int]$_.PSChildName })
    if ($existing.Count -gt 0) {
        $idx = ($existing | Measure-Object -Maximum).Maximum + 1
    }
    $leaf = Join-Path $soBase ('{0:D8}' -f $idx)
    New-Item -Path $leaf -Force | Out-Null
    Set-RegValue -Path $leaf -ValueName 'CLSID' -Value $Clsid
    Set-RegValue -Path $leaf -ValueName 'Profile' -Value $Profile
    Set-RegValue -Path $leaf -ValueName 'KeyboardLayout' -Value 0 -Type DWord
    Write-Host "[fix] wrote SortOrder entry $leaf"
}

Write-Host ""
Write-Host "[done] Feihe IME registered."
Write-Host "       Switch with Win+Space or Ctrl+Shift."
Write-Host "       If it does not show up yet, sign out and back in (the input list is cached by the shell)."
