plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
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

    kotlinOptions {
        jvmTarget = "17"
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")

    // 句末标点这类纯逻辑不碰 Android API，跑普通 JVM 单测就够了，
    // 不需要 Robolectric / 真机。
    testImplementation("junit:junit:4.13.2")
}
