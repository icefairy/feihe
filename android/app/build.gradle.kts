import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// ── Rust 引擎（feihe-core）的交叉编译 ─────────────────────────────────
//
// 引擎是 Rust 写的 `android/jni`（feihe-android），产物是一个 arm64-v8a 的 .so。
// 它**不**走 AGP 的 externalNativeBuild（那是 CMake 的地盘），而是直接调 cargo，
// 再把产物拷成 jniLibs 期望的目录形状。
//
// 三处环境依赖，缺任何一处都会在构建时明确报错（而不是悄悄出一个没有引擎的包）：
//   - cargo（Rust 工具链）
//   - aarch64-linux-android 目标（`rustup target add aarch64-linux-android`）
//   - NDK 里的 clang（当链接器用）
val localProps = Properties().apply {
    val f = rootProject.file("local.properties")
    if (f.exists()) f.inputStream().use { load(it) }
}

val sdkDir: String = localProps.getProperty("sdk.dir")
    ?: System.getenv("ANDROID_HOME")
    ?: System.getenv("ANDROID_SDK_ROOT")
    ?: ""

/** NDK 版本。和 SDK 里 `ndk/` 下的目录名一致，换 NDK 要同步改这里。 */
val ndkVersion = "26.3.11579264"

val rustTarget = "aarch64-linux-android"

/**
 * NDK 的 host 目录。Windows 是 `windows-x86_64`，Linux/macOS 各自不同，
 * 所以这里现场看一眼而不是写死。
 */
fun ndkHostBin(): File {
    val ndk = File(sdkDir, "ndk/$ndkVersion")
    val prebuilt = File(ndk, "toolchains/llvm/prebuilt")
    val host = prebuilt.listFiles()?.firstOrNull()
        ?: throw GradleException("找不到 NDK：$ndk。检查 local.properties 的 sdk.dir，或先装 NDK $ndkVersion")
    return File(host, "bin")
}

/**
 * 链接器用 `clang.exe` 而不是 `aarch64-linux-android24-clang.cmd`。
 *
 * 那个 `.cmd` 包装脚本在 Windows 上会把 `-Wl,--version-script=...` 这类参数搞坏
 * （cmd 的引号规则），报错还是 GBK 编码的中文「此时不应有 --version-script」。
 * 直接用 clang 并显式给 `--target`，行为和 NDK 的包装脚本一致，但不经过 cmd。
 */
fun ndkClang(): File {
    val bin = ndkHostBin()
    return bin.listFiles()?.firstOrNull { it.name == "clang.exe" || it.name == "clang" }
        ?: throw GradleException("NDK 里找不到 clang：$bin")
}

val rustJniLibs = layout.buildDirectory.dir("rustJniLibs")
val rustSo = rootProject.file("jni/target/$rustTarget/release/libfeihe_android.so")

val buildRust by tasks.registering(Exec::class) {
    group = "build"
    description = "交叉编译 feihe-core 到 $rustTarget（arm64-v8a）"
    workingDir = rootProject.file("jni")

    // 只在源码真的变了才重编。core 那边一改，这里就必须跟着重跑，
    // 否则会出现「改了 Rust，装上去行为没变」这种最难查的情况。
    inputs.dir(rootProject.file("jni/src"))
    inputs.file(rootProject.file("jni/Cargo.toml"))
    inputs.dir(rootProject.file("../core/src"))
    inputs.file(rootProject.file("../core/Cargo.toml"))
    outputs.file(rustSo)

    environment(
        "CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER",
        ndkClang().absolutePath,
    )
    environment("RUSTFLAGS", "-Clink-arg=--target=aarch64-linux-android24")
    // 和 Windows 侧的发布构建同一个开关：**绝不能**把本机的个人词库
    // （`src/user_dict.txt`，从搜狗导出的真实打字记录）编进要发出去的包里。
    // 不设它 base.bin 会从 46k 条涨到 98k 条，而且那些词是用户自己的。
    environment("FEIHE_NO_USER_DICT", "1")

    commandLine("cargo", "build", "--release", "--target", rustTarget)
}

val syncRustLib by tasks.registering(Copy::class) {
    group = "build"
    description = "把编译好的 .so 拷成 jniLibs 期望的目录结构"
    dependsOn(buildRust)
    from(rootProject.file("jni/target/$rustTarget/release")) { include("libfeihe_android.so") }
    into(rustJniLibs.map { it.dir("arm64-v8a") })
}

android {
    namespace = "com.feihe.ime"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.feihe.ime"
        // 24 是「能跑 InputMethodService 的新 API」和「还值得支持的老机器」的折中。
        // Android 7.0 以下的份额已经不值得为它牺牲任何 API。
        minSdk = 24
        targetSdk = 34
        versionCode = 1
        versionName = "0.1.0-skeleton"

        // Rust 侧只编了 arm64。写成显式的：不写的话 AGP 会为每个 ABI 找 .so，
        // 缺了就报「找不到 libfeihe_android.so」，看起来像是打包出错。
        ndk { abiFilters += listOf("arm64-v8a") }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    // AGP 默认会把 assets 下的文件再压缩一遍（zip deflate）。ONNX / 词典二进制一旦被
    // 压缩，AssetManager 就拿不到「连续的文件描述符」，sherpa-onnx 的原生层只能把
    // 它整个解压到堆内存再读——79 MB 的模型冷启动会慢一倍，还会多占一份内存。
    // 加进这里之后是 stored（不压缩），原生层可以直接 mmap。
    //
    // `.bin` 是给飞鹤自己的词库（base.bin）留的，同一个道理。
    androidResources {
        noCompress += listOf("onnx", "bin", "fst")
    }

    sourceSets.getByName("main").jniLibs.srcDir(rustJniLibs)

    kotlinOptions {
        jvmTarget = "17"
    }
}

// 每个变体在打包前都要先把 .so 准备好。
tasks.named("preBuild").configure { dependsOn(syncRustLib) }

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")

    // 句末标点这类纯逻辑不碰 Android API，跑普通 JVM 单测就够了，
    // 不需要 Robolectric / 真机。
    testImplementation("junit:junit:4.13.2")
}
