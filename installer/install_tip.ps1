<#
  Feihe IME - install / register (per-user, NO Administrator required).

  What it does:
    1. COM registration (HKCU\SOFTWARE\Classes\CLSID\{...}) pointing at a STABLE dll path.
       HKCU 优先于 HKLM，所以用户级注册能覆盖旧版本留下的机器级条目。
       32-bit host 走 Wow6432Node 那一份（见下方 DllPath32 说明）。
    2. HKCU CTF TIP definition (Category / Item / LanguageProfile)，32 位视图镜像一份。
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
      HKCU\SOFTWARE\Classes\Wow6432Node\CLSID\{CLSID}\InprocServer32 -> feihe32.dll
      HKCU\SOFTWARE\Wow6432Node\Microsoft\CTF\TIP\{CLSID}            (镜像)
    ⚠ HKCU\SOFTWARE\Microsoft\CTF\TIP 是被 WOW64 重定向的（不在共享键列表里），
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
# 本脚本要同时写两个注册表视图：HKCU\SOFTWARE\Classes\CLSID（64 位视图）
# 和 HKCU\SOFTWARE\Classes\Wow6432Node\CLSID（32 位视图）。
# 如果自己就跑在 32 位 PowerShell 里，WOW64 会把 HKCU\SOFTWARE\Classes 重定向到
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
# 参数化的原因：WOW64 把 HKCU\SOFTWARE\Classes 和 HKCU\SOFTWARE\Microsoft\CTF\TIP
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

# -- 0. 清掉上一次留下的 TIP 定义（必须**在注册之前**）----------------
#
# 老版本卸载后可能留下 Enable=0 的 LanguageProfile，那条会让输入法在语言列表里
# 显示成灰色/不可用，所以每次安装都重建一遍。
#
# ⚠ 这段只能放在 Register-TipView **之前**。它原来被夹在「注册完 TIP」和「加键盘
# 列表」之间，等于把刚写好的 64 位 TIP 定义整棵删掉，然后照常往下走——CLSID 还在、
# TIP 没了，于是语言列表里根本看不到飞鹤，「添加键盘」也找不到它。32 位那份
# （Wow6432Node）因为写在后一步反而幸免，症状因此更迷惑：32 位宿主勉强能用，
# 64 位宿主（explorer / 记事本 / 浏览器 / 微信）全都看不见。
#
# 为什么长期没暴露：旧的管理员安装曾在 HKLM 留过一份 TIP，HKCU 缺了会回落到它；
# 等第 6 步把 HKLM 也清干净，输入法就彻底消失了。
foreach ($tipRoot in @(
    'HKCU:\SOFTWARE\Microsoft\CTF\TIP',
    'HKCU:\SOFTWARE\Wow6432Node\Microsoft\CTF\TIP'
)) {
    $stale = Join-Path $tipRoot $Clsid
    if (Test-Path $stale) {
        Remove-Item $stale -Recurse -Force
        Write-Host "[tip] removed stale TIP key: $stale"
    }
}

$script:IconDll = $DllPath

Register-TipView -ClassesRoot 'HKCU:\SOFTWARE\Classes' `
                 -CtfTipRoot  'HKCU:\SOFTWARE\Microsoft\CTF\TIP' `
                 -Dll         $DllPath

if ($DllPath32) {
    Register-TipView -ClassesRoot 'HKCU:\SOFTWARE\Classes\Wow6432Node' `
                     -CtfTipRoot  'HKCU:\SOFTWARE\Wow6432Node\Microsoft\CTF\TIP' `
                     -Dll         $DllPath32
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

# -- 6. 尽力清理旧版本留下的机器级条目 ------------------------------
# v0.3.0 之前是管理员安装，CLSID/TIP 写在 HKLM、DLL 在 Program Files。现在改成
# 用户级：HKCU 的条目优先，旧的 HKLM 条目不会再生效，但留着会指向一个已经不存在的
# 路径（升级安装时 Program Files 那份还在，反而可能被 32 位宿主加载到旧 DLL）。
# 没有管理员权限就删不掉 —— 静默跳过，绝不能因此让安装失败。
foreach ($view in @(
    @{ Name = '64'; Tip = 'HKLM:\SOFTWARE\Microsoft\CTF\TIP';             Cls = 'HKLM:\SOFTWARE\Classes\CLSID' },
    @{ Name = '32'; Tip = 'HKLM:\SOFTWARE\Wow6432Node\Microsoft\CTF\TIP'; Cls = 'HKLM:\SOFTWARE\Classes\Wow6432Node\CLSID' }
)) {
    foreach ($path in @((Join-Path $view.Tip $Clsid), (Join-Path $view.Cls $Clsid))) {
        if (Test-Path $path) {
            try {
                Remove-Item $path -Recurse -Force -ErrorAction Stop
                Write-Host "[legacy] removed machine-wide entry ($($view.Name)-bit): $path"
            } catch {
                Write-Host "[legacy] 保留旧机器级条目（无管理员权限）：$path" -ForegroundColor DarkGray
            }
        }
    }
}

# -- 6. 硬校验：注册项必须真的都在 ------------------------------------
# 安装器只看退出码。0.3.8 就是「TIP 被自己删掉、脚本照样 exit 0」，用户装完发现
# 语言列表里没有飞鹤、添加键盘也没有，却没有任何报错。这里把「定义在不在」变成
# 失败退出码，让安装器当场弹窗，而不是留给用户去语言设置里瞎找。
$required = @(
    "HKCU:\SOFTWARE\Classes\CLSID\$Clsid\InprocServer32",
    "HKCU:\SOFTWARE\Microsoft\CTF\TIP\$Clsid\Category\Item\$Clsid",
    "HKCU:\SOFTWARE\Microsoft\CTF\TIP\$Clsid\LanguageProfile\$LangId\$Profile"
)
if ($DllPath32) {
    $required += "HKCU:\SOFTWARE\Classes\Wow6432Node\CLSID\$Clsid\InprocServer32"
    $required += "HKCU:\SOFTWARE\Wow6432Node\Microsoft\CTF\TIP\$Clsid\LanguageProfile\$LangId\$Profile"
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
