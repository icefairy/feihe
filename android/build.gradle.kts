// 版本组合是被工具链钉死的，不是随便挑的：
//
// - compileSdk 34 —— SDK 里只装了 platforms/android-34
// - AGP 8.5.2     —— compileSdk 34 要求 AGP >= 8.1.1
// - Gradle 8.7    —— AGP 8.5 要求 Gradle >= 8.7
// - JDK 17        —— AGP 8.x 硬性要求（JDK 14 编不了，会直接报错退出）
//
// 任何一个往上跳都要连带动别的，改之前先把这四行对上。
plugins {
    id("com.android.application") version "8.5.2" apply false
    id("org.jetbrains.kotlin.android") version "2.0.21" apply false
}
