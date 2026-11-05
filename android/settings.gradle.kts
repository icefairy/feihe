// 仓库优先走阿里云镜像：这台机器上 dl.google.com 完全不通（实测 http=000），
// 直接用 google() 会在第一步就卡死十几秒然后失败。aliyun 的 google 镜像有全量
// AGP / androidx，实测秒开。
//
// google() / mavenCentral() 留在后面不是因为需要它们，而是作为「镜像缺包」时的
// 兜底——真触发了会慢，但只要走了一次说明有包没被镜像，值得知道。
pluginManagement {
    repositories {
        maven("https://maven.aliyun.com/repository/gradle-plugin")
        maven("https://maven.aliyun.com/repository/google")
        maven("https://maven.aliyun.com/repository/public")
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        maven("https://maven.aliyun.com/repository/google")
        maven("https://maven.aliyun.com/repository/public")
        google()
        mavenCentral()
    }
}

rootProject.name = "FeiheIME"
include(":app")
