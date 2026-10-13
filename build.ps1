# build.ps1 — 飞鹤输入法 Windows 构建（llvm-mingw / gnullvm 路线，无需 MSVC）
#
# 用法：  powershell -ExecutionPolicy Bypass -File build.ps1 [-Debug]
#
# 做了什么：
#   1. 定位并校验 llvm-mingw（默认 C:\llvm-mingw\llvm-mingw-*-ucrt-x86_64\）
#   2. 生成 -lgcc/-lgcc_eh 兼容库 shim（compiler-rt builtins + libunwind）
#      并复制进 llvm-mingw 的 x86_64-w64-mingw32\lib\（幂等）
#   3. 加入 PATH 后执行 cargo build --release（或 -Debug）
#
# 产物：target\x86_64-pc-windows-gnullvm\release\feihe.dll

param([switch]$Debug)

$ErrorActionPreference = 'Stop'
$repo = $PSScriptRoot

# --- 1. 定位 llvm-mingw ---
$roots = @(
    'C:\llvm-mingw',
    "$env:USERPROFILE\llvm-mingw",
    "$env:LOCALAPPDATA\llvm-mingw"
)
$mingw = $null
foreach ($r in $roots) {
    if (Test-Path $r) {
        $cand = Get-ChildItem $r -Directory -Filter 'llvm-mingw-*-ucrt-x86_64' -ErrorAction SilentlyContinue |
                Sort-Object Name -Descending | Select-Object -First 1
        if ($cand -and (Test-Path (Join-Path $cand.FullName 'bin\x86_64-w64-mingw32-gcc.exe'))) {
            $mingw = $cand.FullName; break
        }
    }
}
if (-not $mingw) {
    throw "找不到完整的 llvm-mingw。请解压 llvm-mingw-*-ucrt-x86_64.zip 到 C:\llvm-mingw（需含顶层 bin\ 目录）。"
}
Write-Host "[build] llvm-mingw: $mingw"

# --- 2. 生成/安装 libgcc shim（幂等） ---
$libdir = Join-Path $mingw 'x86_64-w64-mingw32\lib'
$builtins = Get-ChildItem (Join-Path $mingw 'lib\clang') -Recurse -Filter 'libclang_rt.builtins-x86_64.a' -ErrorAction SilentlyContinue |
            Select-Object -First 1
$unwind = Join-Path $mingw 'x86_64-w64-mingw32\lib\libunwind.a'
if (-not $builtins) { throw "找不到 libclang_rt.builtins-x86_64.a" }
if (-not (Test-Path $unwind)) { throw "找不到 libunwind.a" }

Copy-Item $builtins.FullName (Join-Path $libdir 'libgcc.a') -Force
Copy-Item $unwind          (Join-Path $libdir 'libgcc_eh.a') -Force
Write-Host "[build] shim installed: libgcc.a (compiler-rt), libgcc_eh.a (libunwind)"

# --- 3. 构建 ---
$env:PATH = (Join-Path $mingw 'bin') + ';' + $env:PATH
Push-Location $repo
try {
    $args = @('build')
    if (-not $Debug) { $args += '--release' }
    & cargo @args
    if ($LASTEXITCODE -ne 0) { throw "cargo build 失败 ($LASTEXITCODE)" }
} finally {
    Pop-Location
}

$profile = if ($Debug) { 'debug' } else { 'release' }
$dll = Join-Path $repo "target\x86_64-pc-windows-gnullvm\$profile\feihe.dll"
Write-Host ""
if (Test-Path $dll) {
    Write-Host "[build] OK -> $dll" -ForegroundColor Green
} else {
    throw "未生成 $dll"
}
