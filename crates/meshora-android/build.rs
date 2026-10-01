//! 界面文件（和桌面端是同一份 `meshora-desktop/ui`）、图标、权限编进去（tauri-build 做）。

fn main() {
    #[cfg(any(windows, target_os = "android"))]
    tauri_build::build();
}
