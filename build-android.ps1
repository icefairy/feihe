# build-android.ps1 — 飞鹤输入法 Android 端构建（Windows，本机 JDK 17 + 独立 Gradle）
#
# 用法：
#   powershell -ExecutionPolicy Bypass -File build-android.ps1              # 编 release APK
#   powershell -ExecutionPolicy Bypass -File build-android.ps1 -Debug       # 编 debug APK
#   powershell -ExecutionPolicy Bypass -File build-android.ps1 -UnitTest    # 只跑 33 项 JVM 单测
#   powershell -ExecutionPolicy Bypass -File build-android.ps1 -ShellCheck  # 只查环境/预检
#   powershell -ExecutionPolicy Bypass -File build-android.ps1 -SkipRust    # 复用已有 .so
#   powershell -ExecutionPolicy Bypass -File build-android.ps1 -Install     # 编完 adb install
#   powershell -ExecutionPolicy Bypass -File build-android.ps1 -Jdk D:\x -Sdk D:\y -Gradle D:\z
#
# 为什么要这个脚本，而不是直接用 android/gradlew.bat：
#   本机默认 java 是 1.8（PATH 里那份），AGP 8.x 硬性要求 JDK 17，gradlew 会直接报错退出。
#   脚本把 JDK 17 / SDK / Gradle 三处路径显式钉死在进程环境里，**不依赖**机器上
#   的 JAVA_HOME / ANDROID_HOME 配没配对——换台机器只要这三处对得上就能编。
#
# 三个外部依赖（都在 D 盘，见 docs/Android-方案.md §12.5）：
#   SDK    D:\soft\asdk            compileSdk 34 / build-tools 34.0.0 / NDK 26.3.11579264
#   JDK    D:\soft\jdk17           AGP 8.5.2 要求
#   Gradle D:\soft\gradle-8.7      不用 gradlew（它要联网下 8.7 的发行包）
#
# 编出来的 APK 落在 android\app\build\outputs\apk\{debug,release}\。
#
# 签名：debug 和 release 用同一把自签名 key（android\keystore\），所以两者能互相
#       覆盖安装，升级不丢用户词库。key 跟着仓库走，不是机密，见 §12.6。

param(
    [switch]$Debug,
    [switch]$UnitTest,
    [switch]$ShellCheck,
    [switch]$SkipRust,
    [switch]$Install,
    [string]$Jdk = 'D:\soft\jdk17',
    [string]$Sdk = 'D:\soft\asdk',
    [string]$Gradle = 'D:\soft\gradle-8.7',
    [string]$Device = ''
)

$ErrorActionPreference = 'Stop'
$repo = $PSScriptRoot
$androidDir = Join-Path $repo 'android'
$jniDir = Join-Path $androidDir 'jni'

# ── 输出小工具 ─────────────────────────────────────────────────────
$script:failures = 0
function Step([string]$msg) { Write-Host ''; Write-Host "== $msg" -ForegroundColor Cyan }
function Ok([string]$msg)   { Write-Host "   [OK] $msg" -ForegroundColor Green }
function Warn([string]$msg) { Write-Host "   [!!] $msg" -ForegroundColor Yellow }
function Die([string]$msg)  { Write-Host "   [XX] $msg" -ForegroundColor Red; exit 1 }

# 原生命令的 stderr 在 $ErrorActionPreference='Stop' 下会被包成 NativeCommandError
# 直接终止脚本（cargo/gradle 的进度和 warning 全走 stderr）。必须临时降级到
# 'Continue' 再调，跑完恢复。build.ps1 的 Invoke-Cargo 已经踩过这个坑，照抄。
function Invoke-Native([string]$exe, [string[]]$argv) {
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $exe @argv 2>&1 | ForEach-Object { Write-Host $_ }
        return $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $prev
    }
}

function DirSizeMB([string]$path) {
    if (-not (Test-Path $path)) { return 0 }
    $bytes = (Get-ChildItem -Recurse -File $path | Measure-Object -Property Length -Sum).Sum
    return [math]::Round(($bytes / 1MB), 2)
}

# 单个文件的大小（MB）。DirSizeMB 是递归求和，拿来量一个 .so 会把旁边的
# .d/.rlib/增量产物全算进去——2026-11-06 就因此打出过 "181.51 MB" 这种假数字。
function FileSizeMB([string]$path) {
    if (-not (Test-Path $path)) { return 0 }
    return [math]::Round(((Get-Item $path).Length / 1MB), 2)
}

# ── 1. 环境预检 ────────────────────────────────────────────────────
Step '环境预检'

if (-not (Test-Path (Join-Path $Jdk 'bin\java.exe'))) { Die "找不到 JDK 17：$Jdk\bin\java.exe" }
if (-not (Test-Path (Join-Path $Sdk 'platforms')))    { Die "找不到 Android SDK：$Sdk" }
if (-not (Test-Path (Join-Path $Gradle 'bin\gradle.bat'))) { Die "找不到 Gradle：$Gradle\bin\gradle.bat" }

# java -version 把版本号打在 stderr 上；'Stop' 下那行会被当成错误抛出来。
# 所以就地临时降级取第一行，而不是走 Invoke-Native（那样会把整段刷屏）。
$prevEap = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
try { $javaVer = ((& (Join-Path $Jdk 'bin\java.exe') -version 2>&1) | Select-Object -First 1).ToString() }
finally { $ErrorActionPreference = $prevEap }
Ok "JDK   $Jdk  ($javaVer)"

# compileSdk 和 NDK 版本都写死在 android/app/build.gradle.kts 里，先对一眼，
# 免得 Gradle 跑到一半才报「找不到 platforms/android-34」。
$platforms = @(Get-ChildItem (Join-Path $Sdk 'platforms') -Directory | ForEach-Object { $_.Name })
if ($platforms -notcontains 'android-34') {
    Die "SDK 里没有 platforms/android-34（现有：$($platforms -join ', ')）。app/build.gradle.kts 写的是 compileSdk = 34"
}
$ndkVersion = '26.3.11579264'
if (-not (Test-Path (Join-Path $Sdk "ndk\$ndkVersion"))) {
    Die "SDK 里没有 NDK $ndkVersion（app/build.gradle.kts 钉的就是它）"
}
Ok "SDK   $Sdk（android-34, ndk $ndkVersion）"

# 进程级环境变量：Gradle 和它派生的 cargo 都从这里读。
# 只设进程变量，不写注册表/用户变量——脚本退出即失效，不污染这台机器。
$env:JAVA_HOME = $Jdk
$env:ANDROID_HOME = $Sdk
$env:ANDROID_SDK_ROOT = $Sdk
$env:PATH = "$Jdk\bin;$env:PATH"
$env:GRADLE_USER_HOME = Join-Path $env:USERPROFILE '.gradle'

# local.properties 不进版本库，缺了 AGP 会拿不到 sdk.dir。这里补一份（路径用正斜杠，
# Java properties 里反斜杠是转义符——写 D:\soft 会被吃成 D:soft）。
$localProps = Join-Path $androidDir 'local.properties'
$sdkForward = $Sdk -replace '\\', '/'
if (-not (Test-Path $localProps) -or (Get-Content $localProps -Raw) -notmatch [regex]::Escape($sdkForward)) {
    Set-Content -Path $localProps -Value "sdk.dir=$sdkForward" -Encoding ASCII
    Ok "写出 local.properties：sdk.dir=$sdkForward"
} else {
    Ok "local.properties 已指向 $sdkForward"
}

# 跨平台构建产物（base.bin）和字体/词库都在 ../src。Rust 侧要它，Android 侧不直接读。
$cargoExe = (Get-Command cargo.exe -ErrorAction SilentlyContinue).Source
if (-not $cargoExe) { $cargoExe = 'cargo.exe' }

# ── 2. 语音原生库 / 模型 ───────────────────────────────────────────
Step '语音原生库与模型（约 115 MB，不进版本库）'

$jniLibsDir = Join-Path $androidDir 'app\src\main\jniLibs\arm64-v8a'
$sherpaLibs = @(
    'libonnxruntime.so',
    'libsherpa-onnx-c-api.so',
    'libsherpa-onnx-cxx-api.so',
    'libsherpa-onnx-jni.so'
)
$missingLibs = @($sherpaLibs | Where-Object { -not (Test-Path (Join-Path $jniLibsDir $_)) })
if ($missingLibs.Count -gt 0) {
    Warn "缺 $($missingLibs -join ', ')，跑 tools\fetch_sherpa_libs.py 补"
    $py = (Get-Command python -ErrorAction SilentlyContinue)
    if ($py) {
        $rc = Invoke-Native $py.Source @((Join-Path $repo 'tools\fetch_sherpa_libs.py'))
        if ($rc -ne 0) { Die "fetch_sherpa_libs.py 失败（退出码 $rc）。来源见脚本头注释：从 D:\codes\xingyu 拷或 --from 指定目录" }
    } else {
        Die "没有 python，无法自动补原生库"
    }
} else {
    Ok "4 个 sherpa/onnx 原生库齐（$(DirSizeMB $jniLibsDir) MB）"
}

$asrDir = Join-Path $androidDir 'app\src\main\assets\asr'
$asrFiles = @('model.int8.onnx', 'tokens.txt')
$missingAsr = @($asrFiles | Where-Object { -not (Test-Path (Join-Path $asrDir $_)) })
$vadModel = Join-Path $androidDir 'app\src\main\assets\silero_vad.onnx'
if ($missingAsr.Count -gt 0 -or -not (Test-Path $vadModel)) {
    Warn "缺 ASR 模型（$($missingAsr -join ', ')）或 silero_vad.onnx，跑 tools\fetch_asr_model.py 补"
    $py = (Get-Command python -ErrorAction SilentlyContinue)
    if ($py) {
        # 只补 asr/ 那份；VAD 模型在 sherpa 那套里，fetch_asr_model.py 不管。
        $rc = Invoke-Native $py.Source @((Join-Path $repo 'tools\fetch_asr_model.py'))
        if ($rc -ne 0) { Die "fetch_asr_model.py 失败（退出码 $rc）" }
    } else {
        Die "没有 python，无法自动补 ASR 模型"
    }
    if (-not (Test-Path $vadModel)) {
        Die "仍缺 assets\silero_vad.onnx——这个从 tools\fetch_sherpa_libs.py 拷（同 VAD 那套）"
    }
} else {
    $asrMB = [math]::Round(((Get-Item (Join-Path $asrDir 'model.int8.onnx')).Length / 1MB), 1)
    Ok "ASR 模型齐（model.int8.onnx $asrMB MB + tokens.txt + silero_vad.onnx）"
}

# ── 3. Rust 引擎（libfeihe_android.so） ────────────────────────────
$rustTarget = 'aarch64-linux-android'
$rustSo = Join-Path $jniDir "target\$rustTarget\release\libfeihe_android.so"

Step "Rust 引擎 → $rustTarget"

if ($SkipRust -and (Test-Path $rustSo)) {
    Ok "跳过（-SkipRust，复用 $(FileSizeMB $rustSo) MB 的现有 .so）"
} else {
    if ($SkipRust) { Warn "-SkipRust 但 .so 不存在，还是编一次" }

    # rustup 目标装没装。没装的话 cargo 会报 "can't find crate for `std`"，
    # 那句话完全看不出是缺 target，所以提前查。
    $rustup = (Get-Command rustup.exe -ErrorAction SilentlyContinue).Source
    if ($rustup) {
        $targets = (& $rustup target list --installed 2>&1) -join "`n"
        if ($targets -notmatch [regex]::Escape($rustTarget)) {
            Warn "rustup 里没装 $rustTarget，自动补（约 1 分钟）"
            $rc = Invoke-Native $rustup @('target', 'add', $rustTarget)
            if ($rc -ne 0) { Die "rustup target add $rustTarget 失败" }
        }
        Ok "rustup target $rustTarget 就绪"
    } else {
        Warn '找不到 rustup，跳过着一步（假设 target 已装）'
    }

    # NDK 的 clang 当链接器。**不能用** aarch64-linux-android24-clang.cmd：
    # 那个 .cmd 在 Windows 上会把 -Wl,--version-script=... 弄坏（cmd 引号规则），
    # 报错还是 GBK 中文，见 docs/Android-方案.md §12.5 坑 1。
    $ndkBin = Join-Path $Sdk "ndk\$ndkVersion\toolchains\llvm\prebuilt"
    $hostDir = Get-ChildItem $ndkBin -Directory -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $hostDir) { Die "NDK 布局异常：$ndkBin 下没有 prebuilt\<host>" }
    $clang = Join-Path $hostDir.FullName 'bin\clang.exe'
    if (-not (Test-Path $clang)) { Die "NDK 里找不到 clang：$clang" }

    $env:CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER = $clang
    $env:RUSTFLAGS = '-Clink-arg=--target=aarch64-linux-android24'
    # 发布构建必须排除本机搜狗导出的个人词库，否则真实打字记录随 .so 分发出去。
    $env:FEIHE_NO_USER_DICT = '1'

    Ok "链接器 $clang"
    Push-Location $jniDir
    try {
        $rc = Invoke-Native $cargoExe @('build', '--release', '--target', $rustTarget)
    } finally {
        Pop-Location
    }
    if ($rc -ne 0) { Die "cargo build 失败（退出码 $rc）" }
    if (-not (Test-Path $rustSo)) { Die "cargo 说成功了但没找到 $rustSo" }
    Ok "libfeihe_android.so $(FileSizeMB $rustSo) MB"
}

# ── 4. Gradle 任务 ─────────────────────────────────────────────────
if ($ShellCheck) {
    Step 'ShellCheck：只做预检'
    Ok '全部预检通过（未编译）'
    exit 0
}

$gradleExe = Join-Path $Gradle 'bin\gradle.bat'

# 统一关掉 Gradle 守护进程和配置缓存之外的额外噪音；--no-daemon 让每次构建
# 都干净（本机内存 2 GB 的 jvmargs 已经写死在 gradle.properties）。
$gradleCommon = @('--project-dir', $androidDir, '--console=plain', '--no-daemon')

$tasks = @()
if ($UnitTest) {
    $tasks += 'testDebugUnitTest'
} elseif ($Debug) {
    $tasks += 'assembleDebug'
} else {
    $tasks += 'assembleRelease'
}

Step "Gradle $($tasks -join ' ')"
Write-Host "   JAVA_HOME=$env:JAVA_HOME"
Write-Host "   ANDROID_HOME=$env:ANDROID_HOME"
Write-Host "   GRADLE_USER_HOME=$env:GRADLE_USER_HOME"

$netCheck = $null
try { $netCheck = (Invoke-WebRequest -Uri 'https://maven.aliyun.com/repository/google' -Method Head -TimeoutSec 8 -UseBasicParsing).StatusCode } catch { }
if ($netCheck -eq 200) {
    Ok '阿里云 Maven 镜像可达'
} else {
    # 依赖都进缓存时离线也能编；只有缺包才会真失败。提醒一句而不是中止。
    Warn '阿里云 Maven 镜像不可达（首次构建/换版本会失败；缓存齐的话仍能编）'
}

# 首次构建要从阿里云拉 AGP 8.5.2 / Kotlin 2.0.21，冷缓存十分钟起步，
# 给足 45 分钟；正常增量构建 1~3 分钟。
$prev = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
try {
    & $gradleExe @gradleCommon @tasks 2>&1 | ForEach-Object { Write-Host $_ }
    $rc = $LASTEXITCODE
} finally {
    $ErrorActionPreference = $prev
}
if ($rc -ne 0) {
    Write-Host ''
    Die "Gradle 失败（退出码 $rc）。常见原因见 docs/Android-方案.md §12：JDK 版本、SDK 缺组件、首次拉依赖超时"
}

# ── 5. 产物汇总 ────────────────────────────────────────────────────
Step '产物'

if ($UnitTest) {
    $report = Join-Path $androidDir 'app\build\reports\tests\testDebugUnitTest\index.html'
    $xmlDir = Join-Path $androidDir 'app\build\test-results\testDebugUnitTest'
    $total = 0
    $failed = 0
    $xmlCount = 0
    if (Test-Path $xmlDir) {
        # 不用 `([xml]$x).testsuite.tests`：`[xml]` 转成 XmlDocument 之后那个 `.`
        # 是成员访问而不是 XPath，取不到属性。也不用 XmlDocument.Load + GetAttribute
        # ——2026-11-06 实测那条路在 PS 5.1 里恒返回 0（属性和方法都在，就是不进值）。
        # 这里直接正则抠属性：JUnit XML 的 `<testsuite>` 标签是 Gradle 自己按固定
        # 顺序吐的，抠三个整数比调试 PowerShell 的 XML 绑定可靠得多。
        foreach ($f in (Get-ChildItem $xmlDir -Filter 'TEST-*.xml')) {
            $xmlCount++
            $head = (Get-Content $f.FullName -TotalCount 8) -join "`n"
            $m = [regex]::Match($head, '<testsuite\b[^>]*>')
            if ($m.Success) {
                $tag = $m.Value
                foreach ($attr in @('tests', 'failures', 'errors')) {
                    $a = [regex]::Match($tag, $attr + '="(\d+)"')
                    if ($a.Success) {
                        $v = [int]$a.Groups[1].Value
                        if ($attr -eq 'tests') { $total = $total + $v } else { $failed = $failed + $v }
                    }
                }
            }
        }
    }
    if ($xmlCount -eq 0) { Die "没找到单测结果 XML：$xmlDir。是不是 -UnitTest 和别的开关一起给了？" }
    if ($failed -gt 0) { Die "单测有 $failed 项失败，报告 $report" }
    Ok ("单测完成：{0} 项全部通过（{1} 个测试类，报告 {2}）" -f $total, $xmlCount, $report)
    exit 0
}

$variant = if ($Debug) { 'debug' } else { 'release' }
$apkDir = Join-Path $androidDir "app\build\outputs\apk\$variant"
$apks = @(Get-ChildItem $apkDir -Filter *.apk -ErrorAction SilentlyContinue)
if ($apks.Count -eq 0) { Die "没找到 APK：$apkDir" }

foreach ($apk in $apks) {
    Ok ("{0}  {1:N2} MB" -f $apk.Name, ($apk.Length / 1MB))
}

# APK 里三条必须成立的事实，失败就是白打包：$so 在不在、模型是否 STORED、
# 签名是不是那把 key。用 build-tools 里的工具反查，不靠"编译成功"下结论。
$buildTools = @(Get-ChildItem (Join-Path $Sdk 'build-tools') -Directory | Sort-Object Name -Descending | Select-Object -First 1)
if ($buildTools.Count -gt 0) {
    $bt = $buildTools[0].FullName
    $apk = $apks[0].FullName

    # 直接 java -jar 调 apksigner.jar，不走 apksigner.bat。
    # 那个 .bat 是 cmd 批处理，从 PowerShell 里调没事，但它的输出是英文
    # （"Verified using v2 scheme ...: true"），和设计文档里写的 "scheme v2: true"
    # 对不上——2026-11-06 第一次跑就因此误报「v2 未开启」。这里对齐真实输出。
    $apksignerJar = Join-Path $bt 'lib\apksigner.jar'
    if (Test-Path $apksignerJar) {
        $prev = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try {
            $sig = (& (Join-Path $Jdk 'bin\java.exe') -jar $apksignerJar verify -v --print-certs $apk 2>&1) -join "`n"
        } finally { $ErrorActionPreference = $prev }
        if ($sig -match 'Signer #1 certificate DN:\s*(.+)') { Ok "签名 DN $($Matches[1].Trim())" }
        else { Warn "apksigner 没读出签名证书——是不是打出了 unsigned 包？" }
        if ($sig -match 'v2 scheme \(APK Signature Scheme v2\):\s*true') { Ok 'v2 签名开启' }
        elseif ($sig -match 'Verified using v2 scheme') { Warn 'v2 签名未开启（Android 7+ 装不上）' }
    } else {
        Warn "找不到 $apksignerJar，跳过签名核验"
    }

    # 用 .NET 的 ZipFile 反查压缩方式：STORED 的 onnx 才能被 mmap。
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [System.IO.Compression.ZipFile]::OpenRead($apk)
    try {
        $onnxEntry = $zip.Entries | Where-Object { $_.FullName -like '*assets/asr/model.int8.onnx' } | Select-Object -First 1
        if ($onnxEntry) {
            $method = $onnxEntry.CompressedLength -eq $onnxEntry.Length
            if ($method) { Ok "model.int8.onnx 在包里是 STORED（$(('{0:N1}' -f ($onnxEntry.Length/1MB))) MB）" }
            else { Warn "model.int8.onnx 被压缩了（$($onnxEntry.CompressedLength) / $($onnxEntry.Length)）——noCompress 没生效，冷启动会翻倍" }
        } else { Warn 'APK 里没有 assets/asr/model.int8.onnx' }
        $soEntry = $zip.Entries | Where-Object { $_.FullName -like 'lib/arm64-v8a/libfeihe_android.so' } | Select-Object -First 1
        if ($soEntry) { Ok "引擎 .so 在包里（$(('{0:N0}' -f ($soEntry.Length/1KB))) KB）" }
        else { Die 'APK 里没有 lib/arm64-v8a/libfeihe_android.so——preBuild 的 syncRustLib 没跑' }
    } finally { $zip.Dispose() }
}

# ── 6. 可选：装到手机 ─────────────────────────────────────────────
if ($Install) {
    Step 'adb install'
    $adb = Join-Path $Sdk 'platform-tools\adb.exe'
    if (-not (Test-Path $adb)) { Die "找不到 adb：$adb" }
    $devices = (& $adb devices 2>&1 | Select-String 'device$') -join ''
    if (-not $devices) { Die '没有连接的设备（adb devices 为空）。插上 USB 并允许调试' }
    Warn '84 MB 的包直接 adb install 容易超时（设备侧解压慢）；'
    Warn '失败了走 push + pm install，步骤见 handoff.md（2026-11-05 记的那套）'
    # $args 是 PowerShell 自动变量（当前参数列表），别拿来当自己的数组。
    $adbArgv = @()
    if ($Device) { $adbArgv += @('-s', $Device) }
    $adbArgv += @('install', '-r', $apks[0].FullName)
    $rc = Invoke-Native $adb $adbArgv
    if ($rc -ne 0) { Die "adb install 失败（$rc）。试试：adb push <apk> /data/local/tmp/a.apk && adb shell pm install -r /data/local/tmp/a.apk" }
    Ok '安装成功。别忘了在系统设置里启用输入法并切换'
}

Write-Host ''
Ok '构建完成'