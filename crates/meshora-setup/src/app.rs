//! Tauri 那一层：一个窗口、几个命令，进度用事件报给界面。

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::install;

/// 界面一打开要知道的
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Info {
    /// `install`、`update`（客户端发起的更新：不用点，装完打开）或 `uninstall`
    mode: &'static str,
    /// 要装的版本
    version: &'static str,
    /// 装到哪
    dir: String,
    /// 已经装着的版本
    installed: Option<String>,
}

#[derive(Clone, Serialize)]
struct Progress {
    step: String,
    percent: u8,
}

fn has_flag(flag: &str) -> bool {
    std::env::args().any(|arg| arg == flag)
}

#[tauri::command]
fn info() -> Info {
    Info {
        mode: if has_flag("--uninstall") {
            "uninstall"
        } else if has_flag("--update") {
            "update"
        } else {
            "install"
        },
        version: env!("CARGO_PKG_VERSION"),
        dir: install::install_dir().display().to_string(),
        installed: install::installed_version(),
    }
}

/// 在后台线程里做（复制文件、等进程退出都是阻塞的），界面靠事件看进度
async fn background(
    app: AppHandle,
    work: impl FnOnce(&dyn Fn(&str, u8)) -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let report = |step: &str, percent: u8| {
            let _ = app.emit(
                "progress",
                Progress {
                    step: step.to_owned(),
                    percent,
                },
            );
        };
        work(&report)
    })
    .await
    .map_err(|err| format!("安装程序出错：{err}"))?
}

#[tauri::command]
async fn install(app: AppHandle, desktop: Option<bool>) -> Result<(), String> {
    let update = has_flag("--update");
    background(app, move |progress| {
        install::install(&install::Options { desktop, update }, progress)
    })
    .await
}

#[tauri::command]
async fn uninstall(app: AppHandle, purge: bool) -> Result<(), String> {
    background(app, move |progress| install::uninstall(purge, progress)).await
}

#[tauri::command]
fn launch() -> Result<(), String> {
    install::launch()
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

/// 运行安装程序。
pub fn run() {
    if !install::webview2_installed() {
        no_webview2();
        return;
    }
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            info, install, uninstall, launch, quit
        ])
        .run(tauri::generate_context!())
        .expect("安装程序启动失败");
}

/// 没有 WebView2：界面画不出来，用系统对话框说一声
#[allow(unsafe_code)]
fn no_webview2() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONWARNING, MB_OK, MessageBoxW};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let text = wide(
        "这台电脑上没有 Microsoft Edge WebView2 运行时，Meshora 的界面要靠它。\n\n\
         到 https://go.microsoft.com/fwlink/p/?LinkId=2124703 下载安装之后，再运行这个安装程序。",
    );
    let title = wide("安装 Meshora");
    // SAFETY: 两个都是以 0 结尾的 UTF-16，活到函数返回
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONWARNING,
        );
    }
}
