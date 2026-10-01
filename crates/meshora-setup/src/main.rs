//! Meshora 的安装程序（也是卸载程序：装好后复制成 `uninstall.exe`，带 `--uninstall` 运行）。

// 不要控制台窗口
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod app;
#[cfg(windows)]
#[allow(unsafe_code)]
mod install;

fn main() {
    #[cfg(windows)]
    app::run();

    #[cfg(not(windows))]
    {
        eprintln!("Meshora 的安装程序只在 Windows 上用。");
        std::process::exit(1);
    }
}
