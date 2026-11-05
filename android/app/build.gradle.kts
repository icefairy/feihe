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

    kotlinOptions {
        jvmTarget = "17"
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
}
