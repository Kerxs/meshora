//! 界面文件（和桌面端是同一份 `meshora-desktop/ui`）、图标、权限编进去（tauri-build 做）；
//! 编安卓时它还生成 Android 工程里引用插件的那几个 Gradle 文件。
//!
//! 构建脚本跑在编译机上：`#[cfg(target_os)]` 说的是编译机，要看目标平台得读 `CARGO_CFG_TARGET_OS`

fn main() {
    let target = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target == "android" || target == "windows" {
        tauri_build::build();
    }
}
