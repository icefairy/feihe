<#
  build_setup.ps1 — 一键产出安装包

  做四件事：
    1. 用「发布模式」重新构建两份 DLL（-NoUserDict：不并入个人词库）
    2. 校验 base.bin 里确实没有个人词库（防止个人打字记录随安装包发出去）
    3. 找 makensis 并编译 feihe.nsi
    4. 报告产物路径与大小

  用法：
    powershell -ExecutionPolicy Bypass -File installer\build_setup.ps1
    powershell -ExecutionPolicy Bypass -File installer\build_setup.ps1 -SkipBuild   # 只重打包
    powershell -ExecutionPolicy Bypass -File installer\build_setup.ps1 -NsisDir "D:\soft\nsis"
    powershell -ExecutionPolicy Bypass -File installer\build_setup.ps1 -SelfTest    # 真装一遍再校验

  产物：dist\Feihe-Setup-<版本>.exe
#>
param(
    [string]$NsisDir = 'D:\soft\nsis',
    [switch]$SkipBuild,
    [switch]$AllowPersonalDict,
    # 端到端自检：用 SELFTEST 宏额外打一个「不提权、装到 %TEMP%」的安装包，
    # 静默装一次，再把落地的两个 DLL 和 target\ 里的比对 SHA256。
    # 不加这个开关时，正式包的内容和以前完全一致。
    [switch]$SelfTest
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$version = '0.3.2'

function Write-Step($msg) { Write-Host "`n[setup] $msg" -ForegroundColor Cyan }

# 原生命令的统一调用方式。
# 两个坑叠在一起：$ErrorActionPreference = 'Stop' 时，PS 5.1 会把子进程的 stderr
# 包装成 NativeCommandError **立即抛出**，而且光写 `2>&1` 收进变量也拦不住——
# ErrorRecord 在产生的那一刻就触发了 'Stop'。所以必须临时降到 'Continue'。
# 不这么做的话，「cargo 打了条 cargo:warning」或「NSIS 输出一行提示」都会让脚本
# 以一个和真实原因毫无关系的红屏结束。
# 默认只在失败时回显输出（makensis -V3 太吵，成功时没必要刷屏）。
function Invoke-Native([string]$Exe, [string[]]$ArgList, [switch]$Always) {
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $out = & $Exe @ArgList 2>&1
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $prev
    }
    if ($Always -or $code -ne 0) {
        foreach ($line in $out) { Write-Host $line }
    }
    return $code
}

# ── 1. 发布构建 ─────────────────────────────────────────────────
if (-not $SkipBuild) {
    Write-Step '构建发布版 DLL（排除个人词库）'
    # 和 Invoke-Makensis 同样的写法：先 2>&1 收进变量再输出。
    # $ErrorActionPreference = 'Stop' 下原生命令的 stderr 会被包装成
    # NativeCommandError 直接终止脚本，连真正的失败原因都看不到。
    $code = Invoke-Native 'powershell' @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass',
        '-File', (Join-Path $repo 'build.ps1'), '-NoUserDict', '-Arch', 'both'
    ) -Always
    if ($code -ne 0) { throw "build.ps1 失败（$code），上面是构建输出" }
}

$dll64 = Join-Path $repo 'target\x86_64-pc-windows-gnullvm\release\feihe.dll'
$dll32 = Join-Path $repo 'target\i686-pc-windows-gnullvm\release\feihe.dll'
foreach ($d in @($dll64, $dll32)) {
    if (-not (Test-Path $d)) { throw "缺少 $d，请先运行 build.ps1 -NoUserDict" }
}

# ── 2. 个人词库泄漏检查 ─────────────────────────────────────────
# 原理：build.rs 把 user_dict.txt 并进 base.bin。发布构建会跳过它，
# 但万一有人用普通构建的产物来打包，个人打字记录就随 DLL 发出去了，
# 而且是从二进制里抠不出来的。这里直接对比词库条数兜底。
Write-Step '检查是否混入个人词库'
$userDict = Join-Path $repo 'src\user_dict.txt'
$userCount = 0
if (Test-Path $userDict) {
    $userCount = (Get-Content $userDict -Encoding UTF8 | Where-Object { $_ -match "`t" }).Count
}
if ($userCount -gt 0) {
    $buildLog = Join-Path $repo 'target\build_mode.txt'
    $isRelease = (Test-Path $buildLog) -and ((Get-Content $buildLog -Raw).Trim() -eq 'release')
    # 标记可能过期：直接 `cargo build` 不会经过 build.ps1，也就不会重写这个文件。
    # 于是「上一次是发布构建」的旧标记会把这次的普通构建蒙过去，个人词库照样打包
    # 发出去（自测时就这么中过：标记说 release，DLL 里 86749 条）。
    # 兜底：DLL 比标记新 = 标记没覆盖这次构建，一律按非发布模式处理。
    if ($isRelease) {
        $markerTime = (Get-Item $buildLog).LastWriteTime
        $stale = @($dll64, $dll32) | Where-Object { (Get-Item $_).LastWriteTime -gt $markerTime }
        if ($stale) {
            $isRelease = $false
            Write-Host "  构建标记已过期（$(Split-Path $stale[0] -Leaf) 比它新）" -ForegroundColor Yellow
        }
    }
    Write-Host "  src\user_dict.txt 有 $userCount 条个人词条" -ForegroundColor Yellow
    if (-not $isRelease -and -not $AllowPersonalDict) {
        throw @"
检测到 src\user_dict.txt 非空（$userCount 条），但本次构建不是发布模式。
个人词库会被并进 base.bin，随安装包一起分发出去。
如果确实要带个人词库打包，加 -AllowPersonalDict 显式确认。
"@
    }
}
Write-Host '  OK：base.bin 不含个人词库' -ForegroundColor Green

# ── 3. 编译安装包 ───────────────────────────────────────────────
Write-Step '编译 NSIS 安装包'
$makensis = Join-Path $NsisDir 'Bin\makensis.exe'
if (-not (Test-Path $makensis)) {
    $makensis = Join-Path $NsisDir 'makensis.exe'
}
if (-not (Test-Path $makensis)) {
    throw "找不到 makensis.exe。请用 -NsisDir 指定 NSIS 安装目录（当前：$NsisDir）"
}
Write-Host "  makensis: $makensis"

$dist = Join-Path $repo 'dist'
New-Item -ItemType Directory -Force -Path $dist | Out-Null

# ── 3.0 NSIS 脚本必须带 UTF-8 BOM ───────────────────────────────
# NSIS 只有在文件开头有 BOM 时才按 UTF-8 解析；否则按系统 ACP（简中是 GBK）读。
# 脚本里有中文注释和中文界面文本，一旦丢了 BOM 就报
#   Bad text encoding: feihe.nsi:1
#   Error in script "feihe.nsi" on line 1
# 而且这个报错完全指不到真正的原因，很难查。这里每次编译前自动兜底。
$nsi = Join-Path $PSScriptRoot 'feihe.nsi'
$bytes = [System.IO.File]::ReadAllBytes($nsi)
$hasBom = $bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF
if (-not $hasBom) {
    Write-Host '  feihe.nsi 缺少 UTF-8 BOM，已自动补上（NSIS 靠 BOM 判断编码）' -ForegroundColor Yellow
    $text = [System.IO.File]::ReadAllText($nsi, [System.Text.Encoding]::UTF8)
    [System.IO.File]::WriteAllText($nsi, $text, (New-Object System.Text.UTF8Encoding($true)))
}

# makensis 调用抽成函数：正式包和自检包只差一个 /DSELFTEST 宏和输出目录
function Invoke-Makensis([string]$OutDir, [switch]$SelfTestMode) {
    # 变量别叫 $args —— 那是 PowerShell 的自动变量，容易被覆盖出怪问题
    $margs = @("/DOUTDIR=$OutDir", "/DSRCDIR=$(Join-Path $repo 'target')", '-V3')
    if ($SelfTestMode) { $margs += '/DSELFTEST' }
    $margs += 'feihe.nsi'
    Push-Location $PSScriptRoot
    try {
        # /D 选项前面不能有空格，路径不要带尾反斜杠
        $code = Invoke-Native $makensis $margs
        if ($code -ne 0) { throw "makensis 失败（$code），上面是编译器输出" }
    } finally {
        Pop-Location
    }
}

Invoke-Makensis -OutDir $dist

# ── 4. 报告 ─────────────────────────────────────────────────────
$setup = Join-Path $dist "Feihe-Setup-$version.exe"
if (-not (Test-Path $setup)) { throw "未生成 $setup" }
$mb = [math]::Round((Get-Item $setup).Length / 1MB, 2)

Write-Host ''
Write-Host "[setup] OK -> $setup ($mb MB)" -ForegroundColor Green
Write-Host '        双击运行即可（会弹 UAC 提权）。'

# ── 5. 端到端自检（可选）────────────────────────────────────────
# 为什么需要：安装包曾经因为用 CopyFiles 部署（运行时拷贝、源路径是构建机路径）
# 而打出一个 192 KB 的空壳，但 makensis 完全没报错。光看大小、光看编译日志
# 都发现不了，必须真装一次再比对字节。
if ($SelfTest) {
    Write-Step '端到端自检：静默安装到 %TEMP% 并比对 SHA256'
    $tmpDist = Join-Path $env:TEMP 'feihe-selftest-dist'
    New-Item -ItemType Directory -Force -Path $tmpDist | Out-Null
    Invoke-Makensis -OutDir $tmpDist -SelfTestMode

    $probe = Join-Path $tmpDist "Feihe-Setup-$version.exe"
    if (-not (Test-Path $probe)) { throw "自检包未生成：$probe" }

    # 自检包用 RequestExecutionLevel user + InstallDir $TEMP\feihe-selftest，
    # 所以 /S 能在非交互 shell 里跑完，不弹 UAC、不碰 HKLM。
    $installDir = Join-Path $env:TEMP 'feihe-selftest'
    if (Test-Path $installDir) { Remove-Item $installDir -Recurse -Force }
    $p = Start-Process -FilePath $probe -ArgumentList '/S' -Wait -PassThru
    if ($p.ExitCode -ne 0) { throw "自检安装返回 $($p.ExitCode)" }

    $pairs = @(
        @{ Name = '64 位'; Deployed = (Join-Path $installDir 'feihe.dll');   Source = $dll64 },
        @{ Name = '32 位'; Deployed = (Join-Path $installDir 'feihe32.dll'); Source = $dll32 }
    )
    foreach ($pr in $pairs) {
        if (-not (Test-Path $pr.Deployed)) { throw "$($pr.Name) DLL 没落地：$($pr.Deployed)" }
        $h1 = (Get-FileHash $pr.Deployed -Algorithm SHA256).Hash
        $h2 = (Get-FileHash $pr.Source   -Algorithm SHA256).Hash
        if ($h1 -ne $h2) {
            throw "$($pr.Name) DLL 校验不一致`n  装机: $h1`n  构建: $h2"
        }
        Write-Host ("  {0} feihe{1}.dll  {2}  OK" -f $pr.Name, $(if ($pr.Name -eq '64 位') { '' } else { '32' }), $h1.Substring(0, 16)) -ForegroundColor Green
    }

    # 用户手册也要落地。中文文件名 + NSIS 的 File 指令是最容易静默失败的一环：
    # 打漏了、名字乱码了，装完都没人会发现。
    $helpName = '使用帮助.md'
    $helpDst = Join-Path $installDir $helpName
    if (-not (Test-Path -LiteralPath $helpDst)) { throw "用户手册没落地：$helpDst" }
    $h1 = (Get-FileHash -LiteralPath $helpDst -Algorithm SHA256).Hash
    $h2 = (Get-FileHash -LiteralPath (Join-Path $repo "docs\$helpName") -Algorithm SHA256).Hash
    if ($h1 -ne $h2) {
        throw "用户手册校验不一致`n  装机: $h1`n  构建: $h2"
    }
    Write-Host ("  用户手册      {0}  OK" -f $h1.Substring(0, 16)) -ForegroundColor Green

    Remove-Item $installDir -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item $tmpDist -Recurse -Force -ErrorAction SilentlyContinue
    Write-Host '[setup] 自检通过：安装包内容与构建产物逐字节一致' -ForegroundColor Green
}
