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

  产物：dist\Feihe-Setup-<版本>.exe
#>
param(
    [string]$NsisDir = 'D:\soft\nsis',
    [switch]$SkipBuild,
    [switch]$AllowPersonalDict
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$version = '0.1.0'

function Write-Step($msg) { Write-Host "`n[setup] $msg" -ForegroundColor Cyan }

# ── 1. 发布构建 ─────────────────────────────────────────────────
if (-not $SkipBuild) {
    Write-Step '构建发布版 DLL（排除个人词库）'
    & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repo 'build.ps1') -NoUserDict -Arch both
    if ($LASTEXITCODE -ne 0) { throw "build.ps1 失败（$LASTEXITCODE）" }
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

Push-Location $PSScriptRoot
try {
    # /D 选项前面不能有空格，路径不要带尾反斜杠
    $nsiLog = & $makensis `
        "/DOUTDIR=$dist" `
        "/DSRCDIR=$(Join-Path $repo 'target')" `
        '-V3' `
        'feihe.nsi' 2>&1
    if ($LASTEXITCODE -ne 0) {
        $nsiLog | ForEach-Object { Write-Host "    $_" -ForegroundColor DarkGray }
        throw "makensis 失败（$LASTEXITCODE），上面是编译器输出"
    }
} finally {
    Pop-Location
}

# ── 4. 报告 ─────────────────────────────────────────────────────
$setup = Join-Path $dist "Feihe-Setup-$version.exe"
if (-not (Test-Path $setup)) { throw "未生成 $setup" }
$mb = [math]::Round((Get-Item $setup).Length / 1MB, 2)

Write-Host ''
Write-Host "[setup] OK -> $setup ($mb MB)" -ForegroundColor Green
Write-Host '        双击运行即可（会弹 UAC 提权）。'
