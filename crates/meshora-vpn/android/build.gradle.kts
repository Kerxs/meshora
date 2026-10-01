// Meshora 的 VpnService 插件（Android 库）。打包时由 Tauri 并进 App 的 Android 工程，
// Android Gradle 插件、Kotlin 的版本跟着 App 工程走
plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "io.github.kerxs.meshora.vpn"
    compileSdk = 34

    defaultConfig {
        minSdk = 24
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.9.0")
    implementation("androidx.activity:activity-ktx:1.7.2")
    implementation(project(":tauri-android"))
}
