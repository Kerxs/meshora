//! 桌面客户端的入口。

// 不要控制台窗口：日志在界面里看
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    #[cfg(windows)]
    meshora_desktop::app::run();

    #[cfg(not(windows))]
    {
        eprintln!("Meshora 桌面客户端目前只支持 Windows。其他平台请用命令行的 meshorad。");
        std::process::exit(1);
    }
}
