//! Tauri 那一层：一个窗口、几个命令，进度用事件报给界面。

use std::path::PathBuf;

use meshora_setup::{acl, location};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::DialogExt;

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
    /// 能不能改位置：没装过才能改；装着的升级、重装都装回原处
    movable: bool,
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
        dir: install::current_dir().display().to_string(),
        movable: install::installed_dir().is_none(),
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

/// 弹系统的选文件夹窗口，选好换算成安装目录、查一遍能不能装。用户取消是 `None`；不能装就说为什么
#[tauri::command]
async fn pick_dir(app: AppHandle) -> Result<Option<String>, String> {
    let chosen = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("选一个文件夹，Meshora 装在它下面")
            .blocking_pick_folder()
    })
    .await
    .map_err(|err| format!("选文件夹的窗口出错：{err}"))?;
    let Some(chosen) = chosen else {
        return Ok(None);
    };
    let chosen = chosen
        .into_path()
        .map_err(|_| "选的不是本机的文件夹".to_string())?;
    let target = location::target_dir(&chosen)?;
    acl::check_target(&target)?;
    Ok(Some(target.display().to_string()))
}

#[tauri::command]
async fn install(app: AppHandle, desktop: Option<bool>, dir: Option<String>) -> Result<(), String> {
    let update = has_flag("--update");
    // 装着的就装回原处；没装过用界面选的（install 里还会再查一遍）
    let dir = match install::installed_dir() {
        Some(existing) => existing,
        None => match dir {
            Some(dir) => location::target_dir(&PathBuf::from(dir))?,
            None => install::default_dir(),
        },
    };
    background(app, move |progress| {
        install::install(
            &install::Options {
                dir,
                desktop,
                update,
            },
            progress,
        )
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
    // 安装程序自己的界面缓存（WebView2 的数据目录）：装完、卸完都用不着了，退出之后删掉
    if let Ok(dir) = app.path().app_local_data_dir() {
        install::remove_after_exit(&dir);
    }
    app.exit(0);
}

/// 运行安装程序。
pub fn run() {
    if !install::webview2_installed() {
        no_webview2();
        return;
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            info, pick_dir, install, uninstall, launch, quit
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
