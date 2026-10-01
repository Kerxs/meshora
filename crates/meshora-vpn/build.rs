//! 告诉 Tauri 这个插件的 Kotlin 代码在 `android/`，打包时并进 App 的 Android 工程。
//! 没有给界面直接调的命令：命令都由 Rust 这边调用。

fn main() {
    tauri_plugin::Builder::new(&[])
        .android_path("android")
        .build();
}
