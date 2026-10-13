# check_env.ps1 — 飞鹤输入法构建环境自检（llvm-mingw / gnullvm 路线）
# 用法： powershell -ExecutionPolicy Bypass -File check_env.ps1

$ErrorActionPreference = 'SilentlyContinue'
$ok = $true
function Check($name, $cond, $hint) {
    if ($cond) { Write-Host ("[ OK ] {0}" -f $name) -ForegroundColor Green }
    else { Write-Host ("[FAIL] {0}  -> {1}" -f $name, $hint) -ForegroundColor Red; $script:ok = $false }
}

Write-Host '===== 1. Rust 工具链 ====='
Check 'rustc' ([bool](Get-Command rustc.exe)) '安装 rustup'
Check 'cargo' ([bool](Get-Command cargo.exe)) '安装 rustup'
$toolchains = (rustup toolchain list) -join "`n"
Check 'toolchain: stable-x86_64-pc-windows-gnu' ($toolchains -match 'stable-x86_64-pc-windows-gnu') 'rustup toolchain install stable-x86_64-pc-windows-gnu'
$targets = (rustup +stable-x86_64-pc-windows-gnu target list --installed) -join "`n"
Check 'target: x86_64-pc-windows-gnullvm' ($targets -match 'x86_64-pc-windows-gnullvm') 'rustup target add x86_64-pc-windows-gnullvm'

Write-Host ''
Write-Host '===== 2. llvm-mingw ====='
$mingw = $null
foreach ($r in @('C:\llvm-mingw', "$env:USERPROFILE\llvm-mingw", "$env:LOCALAPPDATA\llvm-mingw")) {
    if (Test-Path $r) {
        $c = Get-ChildItem $r -Directory -Filter 'llvm-mingw-*-ucrt-x86_64' | Sort-Object Name -Descending | Select-Object -First 1
        if ($c) { $mingw = $c.FullName; break }
    }
}
Check 'llvm-mingw 解压目录' ([bool]$mingw) '解压 llvm-mingw-*-ucrt-x86_64.zip 到 C:\llvm-mingw'
if ($mingw) {
    Write-Host ("       目录: {0}" -f $mingw)
    Check '顶层 bin\x86_64-w64-mingw32-gcc.exe' (Test-Path (Join-Path $mingw 'bin\x86_64-w64-mingw32-gcc.exe')) '重新完整解压 zip（缺顶层 bin\ 说明解压不完整）'
    Check 'lib\clang\*\lib\windows\libclang_rt.builtins-x86_64.a' ([bool](Get-ChildItem (Join-Path $mingw 'lib\clang') -Recurse -Filter 'libclang_rt.builtins-x86_64.a')) 'llvm-mingw 不完整'
    Check 'x86_64-w64-mingw32\lib\libunwind.a' (Test-Path (Join-Path $mingw 'x86_64-w64-mingw32\lib\libunwind.a')) 'llvm-mingw 不完整'
    $shim = Join-Path $mingw 'x86_64-w64-mingw32\lib\libgcc.a'
    Check 'libgcc shim（build.ps1 生成）' (Test-Path $shim) '运行 build.ps1 自动生成'
}

Write-Host ''
Write-Host '===== 3. 工程配置 ====='
$repo = $PSScriptRoot
Check 'rust-toolchain.toml' (Test-Path (Join-Path $repo 'rust-toolchain.toml')) '缺失'
Check '.cargo\config.toml' (Test-Path (Join-Path $repo '.cargo\config.toml')) '缺失'
Check 'build.ps1' (Test-Path (Join-Path $repo 'build.ps1')) '缺失'

Write-Host ''
if ($ok) { Write-Host '环境检查通过，可运行: build.ps1' -ForegroundColor Green }
else { Write-Host '环境存在问题，见上方 [FAIL]' -ForegroundColor Yellow }
