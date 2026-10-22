# build.ps1 — 飞鹤输入法 Windows 构建（llvm-mingw 路线，无需 MSVC）
#
# 用法：
#   powershell -ExecutionPolicy Bypass -File build.ps1                 # 64 + 32 位都编（默认）
#   powershell -ExecutionPolicy Bypass -File build.ps1 -Arch x64       # 只编 64 位
#   powershell -ExecutionPolicy Bypass -File build.ps1 -Arch x86       # 只编 32 位
#   powershell -ExecutionPolicy Bypass -File build.ps1 -Debug          # debug 版
#   powershell -ExecutionPolicy Bypass -File build.ps1 -SkipTest       # 跳过加载冒烟测试
#   powershell -ExecutionPolicy Bypass -File build.ps1 -NoUserDict     # 发布构建（不含个人词库）
#
# 做了什么：
#   1. 定位并校验 llvm-mingw（默认 C:\llvm-mingw\llvm-mingw-*-ucrt-x86_64\）
#   2. 为每个架构生成 -lgcc/-lgcc_eh 兼容库 shim 并放进该架构的默认库目录（幂等）
#        x64: gnullvm，libgcc.a = compiler-rt builtins，libgcc_eh.a = libunwind
#        x86: gnu，   同上 + eh_stub.o（补 ___register_frame_info 等符号）
#   3. 禁用 libunwind.dll.a，避免产物依赖 libunwind.dll
#   4. 加入 PATH 后执行 cargo build
#
# 产物：
#   target\x86_64-pc-windows-gnullvm\release\feihe.dll   (64 位, Machine=0x8664)
#   target\i686-pc-windows-gnu\release\feihe.dll         (32 位, Machine=0x014c)
#
# 为什么要两个 DLL：in-proc COM 的位数必须和宿主进程一致。
#   64 位宿主（资源管理器、TextInputHost、64 位 Office）只加载 64 位 DLL；
#   32 位宿主（32 位 Office、IE 插件、老的 32 位应用）只加载 32 位 DLL。
#   两者用同一个 CLSID，但注册表位置不同（32 位在 Wow6432Node 下），
#   由 installer\install_tip.ps1 分别写入。

param(
    [switch]$Debug,
    [ValidateSet('x64', 'x86', 'both')]
    [string]$Arch = 'both',
    [switch]$SkipTest,
    # 发布构建：排除 src/user_dict.txt（本机搜狗导出的个人词库）。
    # 打安装包必须开这个，否则个人打字记录会被并进 base.bin 随 DLL 分发出去。
    [switch]$NoUserDict
)

$ErrorActionPreference = 'Stop'
$repo = $PSScriptRoot

if ($NoUserDict) {
    $env:FEIHE_NO_USER_DICT = '1'
    Write-Host "[build] 发布模式：已排除 src/user_dict.txt" -ForegroundColor Yellow
} else {
    Remove-Item Env:FEIHE_NO_USER_DICT -ErrorAction SilentlyContinue
}

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
$env:PATH = (Join-Path $mingw 'bin') + ';' + $env:PATH

$profile = if ($Debug) { 'debug' } else { 'release' }

# ─────────────────────────────────────────────────────────────
# 通用：装 shim + 禁用动态 libunwind
# ─────────────────────────────────────────────────────────────
function Install-Shim {
    param(
        [string]$Triple,        # x86_64-w64-mingw32 / i686-w64-mingw32
        [string]$BuiltinsName   # libclang_rt.builtins-x86_64.a / -i386.a
    )
    $libdir   = Join-Path $mingw "$Triple\lib"
    $builtins = Get-ChildItem (Join-Path $mingw 'lib\clang') -Recurse -Filter $BuiltinsName -ErrorAction SilentlyContinue |
                Select-Object -First 1
    $unwind   = Join-Path $libdir 'libunwind.a'
    if (-not $builtins) { throw "找不到 $BuiltinsName" }
    if (-not (Test-Path $unwind)) { throw "找不到 $Triple\lib\libunwind.a" }

    Copy-Item $builtins.FullName (Join-Path $libdir 'libgcc.a') -Force
    Copy-Item $unwind            (Join-Path $libdir 'libgcc_eh.a') -Force

    # 消除 libunwind.dll 动态依赖（关键）
    # llvm-mingw 同时提供 libunwind.a 与 libunwind.dll.a，gcc 默认优先 .dll.a，
    # 导致 feihe.dll 依赖 libunwind.dll。CTF 把 DLL 加载进 ctfmon.exe /
    # TextInputHost.exe，其搜索路径找不到 libunwind.dll -> 加载失败 -> 输入法不出现。
    # 将 .dll.a 重命名禁用，使 -lunwind 解析到静态库 libunwind.a。
    $unwindDllA    = Join-Path $libdir 'libunwind.dll.a'
    $unwindDisabled = Join-Path $libdir 'libunwind.dll.a.disabled'
    if (Test-Path $unwindDllA) {
        if (Test-Path $unwindDisabled) { Remove-Item $unwindDisabled -Force -ErrorAction SilentlyContinue }
        Move-Item $unwindDllA $unwindDisabled -Force
        Write-Host "[build] $Triple : libunwind.dll.a disabled -> static libunwind.a used"
    }
    Write-Host "[build] $Triple : shim installed (libgcc.a = compiler-rt, libgcc_eh.a = libunwind)"
}

# ─────────────────────────────────────────────────────────────
# 64 位：x86_64-pc-windows-gnullvm
# ─────────────────────────────────────────────────────────────
function Build-X64 {
    Install-Shim -Triple 'x86_64-w64-mingw32' -BuiltinsName 'libclang_rt.builtins-x86_64.a'

    Push-Location $repo
    try {
        $cargoArgs = @('build')
        if (-not $Debug) { $cargoArgs += '--release' }
        & cargo @cargoArgs
        if ($LASTEXITCODE -ne 0) {
            # Windows 允许重命名被占用的文件（如已注册后被 ctfmon/explorer 加载），
            # 此处自动重命名旧 DLL 后重试一次。
            $dll0 = Join-Path $repo "target\x86_64-pc-windows-gnullvm\$profile\feihe.dll"
            if (Test-Path $dll0) {
                Write-Host "[build] feihe.dll 被占用，重命名后重试..." -ForegroundColor Yellow
                Move-Item $dll0 "$dll0.old-$([DateTime]::Now.ToString('HHmmss'))" -Force
                & cargo @cargoArgs
            }
            if ($LASTEXITCODE -ne 0) { throw "cargo build (x64) 失败 ($LASTEXITCODE)" }
        }
    } finally {
        Pop-Location
    }

    $dll = Join-Path $repo "target\x86_64-pc-windows-gnullvm\$profile\feihe.dll"
    if (-not (Test-Path $dll)) { throw "未生成 $dll" }
    return $dll
}

# ─────────────────────────────────────────────────────────────
# 32 位：i686-pc-windows-gnullvm
#
# 和 64 位同一条 gnullvm 路线，所以不需要 rsbegin.o 那套垫片。
#
# 曾经走过 i686-pc-windows-gnu（当时误以为官方没有 i686 gnullvm），
# 那条路有两个额外麻烦，记在这里免得有人重踩：
#   1. *-windows-gnu 会带 rsbegin.o，它引用 GCC 老式 frame-info 接口
#      ___register_frame_info / ___deregister_frame_info。llvm-mingw 用
#      libunwind，没有这两个符号，链接直接失败，得自己写空实现垫上。
#   2. 需要 `rustup target add i686-pc-windows-gnu`（国内用中科大镜像，
#      清华/阿里不保留历史日期目录会 404）。
# 换成 gnullvm 之后这两条都消失了。
# ─────────────────────────────────────────────────────────────
function Build-X86 {
    $installed = (& rustup target list --installed) -join "`n"
    if ($installed -notmatch 'i686-pc-windows-gnullvm') {
        throw @"
缺少 32 位标准库。请先执行：

    `$env:RUSTUP_DIST_SERVER='https://mirrors.ustc.edu.cn/rust-static'
    rustup target add i686-pc-windows-gnullvm

（若走官方源：rustup target add i686-pc-windows-gnullvm）
"@
    }

    Install-Shim -Triple 'i686-w64-mingw32' -BuiltinsName 'libclang_rt.builtins-i386.a'

    Push-Location $repo
    try {
        $cargoArgs = @('build', '--target', 'i686-pc-windows-gnullvm')
        if (-not $Debug) { $cargoArgs += '--release' }
        & cargo @cargoArgs
        if ($LASTEXITCODE -ne 0) {
            $dll0 = Join-Path $repo "target\i686-pc-windows-gnullvm\$profile\feihe.dll"
            if (Test-Path $dll0) {
                Write-Host "[build] feihe.dll (x86) 被占用，重命名后重试..." -ForegroundColor Yellow
                Move-Item $dll0 "$dll0.old-$([DateTime]::Now.ToString('HHmmss'))" -Force
                & cargo @cargoArgs
            }
            if ($LASTEXITCODE -ne 0) { throw "cargo build (x86) 失败 ($LASTEXITCODE)" }
        }
    } finally {
        Pop-Location
    }

    $dll = Join-Path $repo "target\i686-pc-windows-gnullvm\$profile\feihe.dll"
    if (-not (Test-Path $dll)) { throw "未生成 $dll" }
    return $dll
}

# ─────────────────────────────────────────────────────────────
# 校验 PE 头，防止「编出来是 64 位却当 32 位用」这种低级事故
# ─────────────────────────────────────────────────────────────
function Assert-PE {
    param([string]$Path, [uint16]$WantMachine)
    $fs = [IO.File]::OpenRead($Path)
    try {
        $br = New-Object IO.BinaryReader($fs)
        $fs.Position = 0x3C; $pe = $br.ReadUInt32()
        $fs.Position = $pe;   $null = $br.ReadUInt32()
        $machine = $br.ReadUInt16()
        if ($machine -ne $WantMachine) {
            throw ("{0}: Machine=0x{1:X4}，期望 0x{2:X4}" -f $Path, $machine, $WantMachine)
        }
        return $machine
    } finally { $fs.Close() }
}

# ─────────────────────────────────────────────────────────────
# 冒烟测试：LoadLibrary + DllGetClassObject + CreateInstance
#
# 为什么值得做：编译通过 ≠ 能被宿主加载。依赖 DLL 缺失（例如以前那个
# libunwind.dll）、DllMain 崩溃、导出名写错，全都只在 LoadLibrary 那一刻才暴露。
# 用对应位数的测试程序去加载，能提前发现「64 位能跑、32 位一加载就失败」这类问题。
# ─────────────────────────────────────────────────────────────
function Test-Dll {
    param([string]$DllPath, [string]$Triple, [string]$Tag)
    $src = Join-Path $repo 'shim\loadtest.c'
    if (-not (Test-Path $src)) { Write-Host "[test] 跳过（找不到 shim\loadtest.c）" -ForegroundColor Yellow; return }
    $exe = Join-Path $repo "target\shim\loadtest-$Tag.exe"
    New-Item -ItemType Directory -Force -Path (Split-Path $exe) | Out-Null
    & "$Triple-gcc" -O2 -o $exe $src -lole32 -luuid 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "编译 loadtest ($Tag) 失败" }

    $out = & $exe $DllPath clsid 2>&1
    $code = $LASTEXITCODE
    $out | ForEach-Object { Write-Host "        $_" }
    if ($code -ne 0) { throw "DLL 冒烟测试失败 ($Tag, exit=$code)" }
    Write-Host "[test] $Tag : PASS" -ForegroundColor Green
}

# ─────────────────────────────────────────────────────────────
# 主流程
# ─────────────────────────────────────────────────────────────
$results = @()
if ($Arch -eq 'x64' -or $Arch -eq 'both') { $results += , @{ Name = 'x64'; Tag = '64'; Triple = 'x86_64-w64-mingw32'; Path = (Build-X64); Machine = 0x8664 } }
if ($Arch -eq 'x86' -or $Arch -eq 'both') { $results += , @{ Name = 'x86'; Tag = '32'; Triple = 'i686-w64-mingw32';   Path = (Build-X86); Machine = 0x014C } }

Write-Host ""
foreach ($r in $results) {
    $m = Assert-PE -Path $r.Path -WantMachine $r.Machine
    $bits = if ($m -eq 0x8664) { '64 位' } else { '32 位' }
    $size = [math]::Round((Get-Item $r.Path).Length / 1KB)
    Write-Host ("[build] OK ({0}, Machine=0x{1:X4}, {2} KB) -> {3}" -f $bits, $m, $size, $r.Path) -ForegroundColor Green
}

if (-not $SkipTest) {
    foreach ($r in $results) { Test-Dll -DllPath $r.Path -Triple $r.Triple -Tag $r.Tag }
}

# 记下本次构建模式，给 installer\build_setup.ps1 做「个人词库有没有混进去」的校验。
# 这是个纯标记文件，不参与编译。
$marker = Join-Path $repo 'target\build_mode.txt'
$mode = if ($NoUserDict) { 'release' } else { 'dev' }
Set-Content -LiteralPath $marker -Value $mode -Encoding ASCII -NoNewline
Write-Host "[build] 构建模式: $mode（标记写入 target\build_mode.txt）"
