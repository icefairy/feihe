<#
  Feihe IME - install / register (**必须管理员**，注册写 HKLM).

  为什么必须是 HKLM（2026-10-28 实测，用户报「设置里多出一个不可用的输入法，
  Win+Space 里没有飞鹤」）：
    之前一直是写 HKCU\SOFTWARE\Microsoft\CTF\TIP。注册表结构看起来完全正确，
    `CoCreateInstance(CLSID)` 也成功，但系统的输入法列表**读不到它**：
      - `ITfInputProcessorProfiles::GetLanguageProfileDescription` 只查 HKLM，
        HKCU 那份的 Description 拿不到 → 设置页显示成「不可用的输入法」；
      - Win+Space 的切换列表同样只来自 HKLM → 根本不列出飞鹤。
    机器上所有能用的第三方输入法（搜狗、小狼毫 weasel、微软拼音）无一例外都在
    HKLM\SOFTWARE\Microsoft\CTF\TIP，HKCU 下只有我们自己一条 —— 这就是铁证。
    以前「能用」是因为更早的管理员安装在 HKLM 留过一份残留，HKCU 缺了会回落到它；
    等脚本第 6 步把 HKLM 清干净，就彻底没了。

  What it does:
    1. COM registration (HKLM\SOFTWARE\Classes\CLSID\{...}) pointing at a STABLE dll path.
       32-bit host 走 Wow6432Node 那一份（见下方 DllPath32 说明）。
    2. HKLM CTF TIP definition (Category / Item / LanguageProfile)，32 位视图镜像一份。
    3. Adds Feihe to the CURRENT USER keyboard list via input.dll!InstallLayoutOrTip.
    4. Verifies the result and falls back to a direct registry write if needed.
    5. Restarts ctfmon so a freshly deployed DLL takes effect without sign-out.

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
    ⚠ SOFTWARE\Microsoft\CTF\TIP 是被 WOW64 重定向的（不在共享键列表里），
      32 位进程读的是 Wow6432Node 那份，所以 TIP 定义必须写两遍。

  用户级的部分仍然是 HKCU：键盘列表（User Profile）和 SortOrder 是「当前用户的
  偏好」，本来就该按用户存，不需要管理员。

  This file is UTF-8 WITH BOM on purpose: PowerShell 5.1 needs the BOM to read the
  Chinese display name correctly. Do not re-save as plain UTF-8 or ASCII.
#>
param(
    [Parameter(Mandatory = $true)][string]$DllPath,
    [string]$DllPath32,
    # 只体检、不写任何东西。注册正常 exit 0，需要修复 exit 3。
    # Feihe-Repair.exe 用它来决定「要不要真的动一遍」——用户双击修复工具时
    # 大多数情况注册是好的，白写一遍只会让他以为出过问题。
    [switch]$CheckOnly
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
        # -CheckOnly 必须一起带过去：漏了的话 32 位入口会起一个**正常注册**的
        # 子进程，「体检」变成「悄悄改一遍」，比不做体检更糟。
        if ($CheckOnly) { $a += '-CheckOnly' }
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

# ── 只体检（-CheckOnly）：一个字都不写，只看「系统现在能不能看到我们」──
#
# 判定项与末尾的硬校验同源，另外多三条最容易漏的：
#   · `InprocServer32` 指的**是不是当前这份 DLL**。换过安装目录、或者把目录搬走
#     之后，键都在、路径是旧的，宿主加载失败——症状和「没注册」一模一样。
#   · 当前用户的键盘列表里有没有我们。注册全对但键盘列表没加上，Win+Space 就是
#     不列出来，用户照样用不了。
#   · `HKCU\...\CTF\TIP\{CLSID}` 有没有残留。它是「设置里多出一个不可用的输入法」
#     的来源，而且 `HKCU\Software\Classes` 优先级比 HKLM 高，留着会盖住新路径。
#
# 退出码：0 = 正常，3 = 需要修复。1/2 留给真正的失败，别混用。
# 不需要管理员：读 HKLM 谁都能读，这样「只是想看看正不正常」的人不用点 UAC。
if ($CheckOnly) {
    $need = New-Object System.Collections.Generic.List[string]

    $inproc64 = "HKLM:\SOFTWARE\Classes\CLSID\$Clsid\InprocServer32"
    $cur64 = $null
    if (Test-Path -LiteralPath $inproc64) {
        $cur64 = (Get-ItemProperty -LiteralPath $inproc64 -ErrorAction SilentlyContinue).'(default)'
    }
    if (-not $cur64) {
        $need.Add("64 位 COM 注册缺失：$inproc64")
    } elseif ($cur64 -ne $DllPath) {
        $need.Add("64 位 COM 注册指向别的 DLL：注册表里是 `"$cur64`"，应为 `"$DllPath`"")
    }

    $tip64 = "HKLM:\SOFTWARE\Microsoft\CTF\TIP\$Clsid"
    if (-not (Test-Path -LiteralPath "$tip64\Category\Item\$Clsid")) {
        $need.Add("TIP 定义缺失（64 位，Category\Item）：$tip64")
    }
    $lp64 = "$tip64\LanguageProfile\$LangId\$Profile"
    if (-not (Test-Path -LiteralPath $lp64)) {
        $need.Add("LanguageProfile 缺失（64 位）：$lp64")
    } else {
        $en = (Get-ItemProperty -LiteralPath $lp64 -ErrorAction SilentlyContinue).Enable
        if ($null -eq $en -or [int]$en -ne 1) { $need.Add("LanguageProfile 的 Enable 不是 1（64 位）") }
    }

    if ($DllPath32) {
        $inproc32 = "HKLM:\SOFTWARE\Classes\Wow6432Node\CLSID\$Clsid\InprocServer32"
        $cur32 = $null
        if (Test-Path -LiteralPath $inproc32) {
            $cur32 = (Get-ItemProperty -LiteralPath $inproc32 -ErrorAction SilentlyContinue).'(default)'
        }
        if (-not $cur32) {
            $need.Add("32 位 COM 注册缺失：$inproc32")
        } elseif ($cur32 -ne $DllPath32) {
            $need.Add("32 位 COM 注册指向别的 DLL：注册表里是 `"$cur32`"，应为 `"$DllPath32`"")
        }
        $lp32 = "HKLM:\SOFTWARE\Wow6432Node\Microsoft\CTF\TIP\$Clsid\LanguageProfile\$LangId\$Profile"
        if (-not (Test-Path -LiteralPath $lp32)) {
            $need.Add("LanguageProfile 缺失（32 位）：$lp32")
        }
    }

    if (Test-Path -LiteralPath "HKCU:\SOFTWARE\Microsoft\CTF\TIP\$Clsid") {
        $need.Add("HKCU 还留着旧的 TIP 定义（设置里会多出一个「不可用的输入法」）")
    }

    $upKey = 'HKCU:\Control Panel\International\User Profile\zh-Hans-CN'
    $tipValue = "0804:$Clsid$Profile"
    $inList = $false
    if (Test-Path -LiteralPath $upKey) {
        $inList = (@((Get-Item -LiteralPath $upKey).GetValueNames()) -contains $tipValue)
    }
    if (-not $inList) {
        $need.Add("当前用户的键盘列表里没有飞鹤（Win+Space 不会列出它）")
    }

    if ($need.Count -eq 0) {
        Write-Host "[ok] 注册状态正常，无需修复（CLSID / TIP 定义 / LanguageProfile / 键盘列表都在，且指向当前 DLL）"
        exit 0
    }
    Write-Host "[need] 需要修复，共 $($need.Count) 项：" -ForegroundColor Yellow
    foreach ($n in $need) { Write-Host "       $n" -ForegroundColor Yellow }
    exit 3
}

# ── 必须管理员 ──────────────────────────────────────────────────
# 注册项写到 HKLM（见文件头说明：HKCU 的话系统读不到，输入法显示成「不可用」）。
# 非管理员时不要硬往下走——半套注册比不注册更难查。
$isAdmin = ([Security.Principal.WindowsPrincipal] `
    [Security.Principal.WindowsIdentity]::GetCurrent()
).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    Write-Host "[FAIL] 需要管理员权限（注册项要写 HKLM，否则系统读不到，输入法会显示成「不可用」）。" -ForegroundColor Red
    Write-Host "       请右键「以管理员身份运行」安装程序，或右键 Feihe-Repair.exe 选择「以管理员身份运行」。" -ForegroundColor Red
    exit 2
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
function Register-Com {
    param(
        [Parameter(Mandatory = $true)][string]$ClassesRoot,
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
}

# -- 1b. CTF TIP 定义（Category + LanguageProfile）------------------------
#
# ⚠ **必须在 InstallLayoutOrTip 之后调用，不能放在前面。**
#
# 实测（2026-10-28，本机）：`input.dll!InstallLayoutOrTip` 会把它不认可的 TIP 定义
# 整棵删掉——调用前 64 位那份 `Category\Item` 存在，调用后就没了，而 32 位那份它
# 不管。它清的是「当前进程视图」里的孤儿 CLSID 键：只要这个 profile 没走过 TSF 的
# 正式注册（ITfInputProcessorProfileMgr::RegisterProfile），就被当成垃圾。
#
# 而它又是「把输入法加进当前用户键盘列表」唯一好用的 API（0.3.x 当初为修
# 「Ctrl+Shift 切不到飞鹤」才加的）。两边都要，只能排顺序：
#
#   清旧定义 → 写 CLSID → InstallLayoutOrTip → **再写 TIP 定义** → 补 User Profile
#
# 定义最后写，谁也删不掉；安装器末尾的硬校验会确认它真的在。
function Register-TipDef {
    param([Parameter(Mandatory = $true)][string]$CtfTipRoot)

    # 2. CTF TIP definition
    $tip = Join-Path $CtfTipRoot $Clsid

    # 2a. Category: mark this TIP as a keyboard input method
    # 只注册 KEYBOARD 是不够的。Windows 8 以后引入了 capability 分类，
    # 特别是 IMMERSIVESUPPORT —— 缺它的话 UWP / WinUI3 应用（例如 Windows 11
    # 记事本）不会把按键转给 TIP：Activate 照常调用、key sink 也挂上了，
    # 但一个 OnTestKeyDown 都收不到，表现为「切到输入法后字母直接进输入框」。
    $ImmGuid = '{13A016DF-560B-46CD-947A-4C3AF1E0E35D}'   # GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT

    # UIELEMENTENABLED：宿主用 `ITfThreadMgrEx::ActivateEx(TF_TMAE_UIELEMENTENABLEDONLY)`
    # 激活 TSF 时（独占全屏游戏就是这么干的），MSCTF **只加载**注册了这个分类的 TIP。
    # 缺了它，游戏里的症状不是「候选看不见」，而是**输入法整个不出现**：ActivateEx
    # 照样返回 S_OK，一个错都不报，UILess 那段代码永远没机会跑。
    # 这条是实测出来的——见 tools/tsfhost。
    $UiElemGuid = '{49D2F9CF-1F5E-11D7-A6D3-00065B84435C}'  # GUID_TFCAT_TIPCAP_UIELEMENTENABLED

    $categories = @($KbdGuid, $ImmGuid, $UiElemGuid)
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

# -- 0. 清掉上一次留下的注册（必须**在最前面**）------------------------
#
# 老版本卸载后可能留下 Enable=0 的 LanguageProfile，那条会让输入法在语言列表里
# 显示成灰色/不可用，所以每次安装都重建一遍。
#
# ⚠ 这段必须放在**所有写入之前**。它一度被夹在「注册完 TIP」和「加键盘列表」之间，
# 等于把刚写好的 64 位 TIP 定义整棵删掉，然后照常往下走——CLSID 还在、TIP 没了，
# 于是语言列表里根本看不到飞鹤，「添加键盘」也找不到它。
#
# 现在清四个位置：HKLM/HKCU × 64/32 位视图。
#   - HKLM 两份：上一次安装（管理员）留下的旧路径，必须换成当前 DLL 路径；
#   - HKCU 两份：**0.3.8 及以前版本写在这里**，而 HKCU\Software\Classes 的优先级
#     **高于** HKLM —— 不清掉的话，旧版写的 HKCU CLSID 会盖住我们刚写的 HKLM 那条，
#     而且 HKCU 的 TIP 定义就是「设置里那个不可用的输入法」的来源。
$staleRoots = @(
    'HKLM:\SOFTWARE\Microsoft\CTF\TIP',
    'HKLM:\SOFTWARE\Wow6432Node\Microsoft\CTF\TIP',
    'HKCU:\SOFTWARE\Microsoft\CTF\TIP',
    'HKCU:\SOFTWARE\Wow6432Node\Microsoft\CTF\TIP'
)
foreach ($tipRoot in $staleRoots) {
    $stale = Join-Path $tipRoot $Clsid
    if (Test-Path -LiteralPath $stale) {
        Remove-Item -LiteralPath $stale -Recurse -Force
        Write-Host "[tip] removed stale TIP key: $stale"
    }
}
foreach ($clsRoot in @(
    'HKCU:\SOFTWARE\Classes\CLSID',
    'HKCU:\SOFTWARE\Classes\Wow6432Node\CLSID'
)) {
    $stale = Join-Path $clsRoot $Clsid
    if (Test-Path -LiteralPath $stale) {
        Remove-Item -LiteralPath $stale -Recurse -Force
        Write-Host "[com] removed stale HKCU CLSID: $stale"
    }
}

$script:IconDll = $DllPath

Register-Com -ClassesRoot 'HKLM:\SOFTWARE\Classes' -Dll $DllPath

if ($DllPath32) {
    Register-Com -ClassesRoot 'HKLM:\SOFTWARE\Classes\Wow6432Node' -Dll $DllPath32
    Write-Host "[com] 32-bit host support enabled"
} else {
    Write-Host "[com] 未提供 32 位 DLL，32 位宿主（32 位 Office / IE 插件等）无法使用飞鹤" -ForegroundColor Yellow
}

# -- 3. Add to the current user's keyboard list -----------------------
# 这里不再删 TIP 键——清理在注册**之前**（第 0 步）做完了。0.3.8 及以前这段写在这，
# 把刚注册好的 TIP 定义整个删掉，是「装完语言列表里没有飞鹤」的直接原因。

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

# -- 3b. 写 TIP 定义（**必须在上面那步之后**，见 Register-TipDef 的注释）-----
Register-TipDef -CtfTipRoot 'HKLM:\SOFTWARE\Microsoft\CTF\TIP'
if ($DllPath32) {
    Register-TipDef -CtfTipRoot 'HKLM:\SOFTWARE\Wow6432Node\Microsoft\CTF\TIP'
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

# -- 5. 重启 ctfmon，让新 DLL 真正生效 ---------------------------------
# 为什么需要这一步：安装器 / register.cmd 换 DLL 的手法是「把被占用的旧文件改名成
# .old，再写新文件」（Windows 允许重命名已映射的文件，不允许直接覆盖）。但改名
# 只动了磁盘——ctfmon.exe 进程里映射的仍是旧代码，它不会因为磁盘上换了文件就
# 重新加载。不重启 ctfmon，覆盖安装后新版本要等到下次注销才生效，
# 「更新不用注销」就落了空。
#
# ctfmon 被杀后 Windows 会按需自动重启（点一下任意输入框即拉起，中等完整性，
# 和登录时一样），重启时重新 CoCreateInstance(CLSID) → 按注册表路径加载新 DLL。
# 所以只杀、不手动拉：从提权的 PowerShell 里 Start-Process ctfmon 拿到的是高
# 完整性令牌，UIPI 会挡住它和普通应用之间的窗口消息，输入法直接瘫痪。
# 宁可让系统自己拉。
try {
    Stop-Process -Name ctfmon -Force -ErrorAction Stop
    Write-Host "[tip] ctfmon 已重启：下次输入时加载新 DLL，不用注销"
} catch {
    # 没在跑（或杀不了）都不是错误——系统会在需要时自己拉起
    Write-Host "[tip] ctfmon 未在运行，跳过重启"
}

# 顺手清理改名残留：ctfmon 一死就没人映射 .old 了，能删就删；删不掉
# （用户立刻点了输入框，ctfmon 又起来了）就留给卸载程序。
Start-Sleep -Milliseconds 500
$dlls = @($DllPath)
if ($DllPath32) { $dlls += $DllPath32 }
foreach ($dll in $dlls) {
    $old = "$dll.old"
    if (Test-Path -LiteralPath $old) {
        Remove-Item -LiteralPath $old -Force -ErrorAction SilentlyContinue
        if (-not (Test-Path -LiteralPath $old)) { Write-Host "[tip] removed $old" }
    }
}

# -- 5.5 报告：还有哪些进程映射着**别的路径**的 feihe.dll ---------------
# 换 DLL 只对「之后才加载它」的进程有效。浏览器、资源管理器、编辑器这类常驻程序
# 在安装前就把旧 DLL 映射进自己的地址空间了，不重启它们，里面跑的一直是旧代码。
#
# 症状极具误导性：同一个版本，记事本里新功能好用、Chrome 里却是老样子，用户会
# 以为「设置没保存」。实测过一台机器上三个不同目录的 feihe.dll 同时活着
# （旧管理员安装的 C://soft//Feihe、更早的 ProgramData、新装的 LOCALAPPDATA）。
#
# 只报告、不杀进程：这里面可能有用户正在写的东西（Word、IDE），替用户关掉窗口
# 比让他自己重开一次糟糕得多。信息进安装日志，也打一行到控制台。
$loaded = @{}
try {
    foreach ($proc in Get-Process) {
        try {
            foreach ($m in $proc.Modules) {
                if ($m.ModuleName -like 'feihe*.dll' -and $m.FileName) {
                    if ($m.FileName -ne $DllPath -and $m.FileName -ne $DllPath32) {
                        $loaded[$proc.ProcessName] = $m.FileName
                    }
                }
            }
        } catch { }   # 系统进程 / 别的用户的进程读不到模块列表，跳过
    }
} catch { }
if ($loaded.Count -gt 0) {
    Write-Host "[stale] 下列程序仍在使用旧版 DLL，请关闭后重开一次才会用上新版本："
    foreach ($k in ($loaded.Keys | Sort-Object)) {
        Write-Host ("         {0}  ->  {1}" -f $k, $loaded[$k])
    }
} else {
    Write-Host "[stale] 没有进程映射旧路径的 feihe.dll"
}

# -- 6. 硬校验：注册项必须真的都在 ------------------------------------
# 安装器只看退出码。0.3.8 就是「TIP 被自己删掉、脚本照样 exit 0」，用户装完发现
# 语言列表里没有飞鹤、添加键盘也没有，却没有任何报错。这里把「定义在不在」变成
# 失败退出码，让安装器当场弹窗，而不是留给用户去语言设置里瞎找。
#
# 验的是 HKLM —— 系统只认这一份。HKCU 那份即使存在也不会有用（见文件头）。
$required = @(
    "HKLM:\SOFTWARE\Classes\CLSID\$Clsid\InprocServer32",
    "HKLM:\SOFTWARE\Microsoft\CTF\TIP\$Clsid\Category\Item\$Clsid",
    "HKLM:\SOFTWARE\Microsoft\CTF\TIP\$Clsid\LanguageProfile\$LangId\$Profile"
)
if ($DllPath32) {
    $required += "HKLM:\SOFTWARE\Classes\Wow6432Node\CLSID\$Clsid\InprocServer32"
    $required += "HKLM:\SOFTWARE\Wow6432Node\Microsoft\CTF\TIP\$Clsid\LanguageProfile\$LangId\$Profile"
}
$missing = @($required | Where-Object { -not (Test-Path -LiteralPath $_) })
if ($missing.Count -gt 0) {
    Write-Host "[FAIL] 注册项没有写全，输入法不会出现在语言列表里：" -ForegroundColor Red
    foreach ($m in $missing) { Write-Host "       $m" -ForegroundColor Red }
    exit 1
}
Write-Host "[ok] CLSID / TIP 定义 / LanguageProfile 全部就位"

Write-Host ""
Write-Host "[done] Feihe IME registered."
Write-Host "       Switch with Win+Space or Ctrl+Shift."
Write-Host "       If it does not show up yet, sign out and back in (the input list is cached by the shell)."
