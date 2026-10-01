//! Meshora 的安卓客户端。
//!
//! 和桌面端几乎是同一个东西：界面是同一份 `meshora-desktop/ui`（按平台藏掉手机上用不上的），
//! 节点的启停、状态汇总是同一个 [`Controller`]。不同的只有虚拟网卡：App 没有 root，
//! 网卡请系统的 VpnService 建（`tauri-plugin-meshora-vpn`），拿回描述符交给数据面。
//!
//! 只在安卓和 Windows 上编：Windows 上编得过，是为了在开发机上就能检查（用不了，网卡建不起来）。
#![cfg(any(windows, target_os = "android"))]

#[cfg(target_os = "android")]
#[allow(unsafe_code)]
mod logcat;

use meshora_desktop::controller::{AdminAction, Controller, CreateAt, Overview};
use meshora_desktop::logs::LogBuffer;
use meshora_desktop::update::UpdateView;
use tauri::{Manager, State};

#[tauri::command]
fn overview(controller: State<'_, Controller>) -> Overview {
    controller.overview()
}

#[tauri::command]
async fn connect(controller: State<'_, Controller>, code: Option<String>) -> Result<(), String> {
    controller.connect(code).await
}

#[tauri::command]
async fn create(
    controller: State<'_, Controller>,
    at: CreateAt,
    name: String,
) -> Result<(), String> {
    controller.create(at, name).await
}

#[tauri::command]
async fn admin(
    controller: State<'_, Controller>,
    action: AdminAction,
) -> Result<Option<String>, String> {
    controller.admin(action).await
}

#[tauri::command]
fn set_onboarded(controller: State<'_, Controller>) {
    controller.set_onboarded();
}

#[tauri::command]
fn add_server(controller: State<'_, Controller>, code: String) -> Result<String, String> {
    controller.add_server(&code)
}

#[tauri::command]
fn remove_server(controller: State<'_, Controller>, code: String) {
    controller.remove_server(&code);
}

#[tauri::command]
async fn disconnect(controller: State<'_, Controller>) -> Result<(), String> {
    controller.disconnect().await;
    Ok(())
}

#[tauri::command]
async fn forget(controller: State<'_, Controller>) -> Result<(), String> {
    controller.forget().await;
    Ok(())
}

#[tauri::command]
fn set_auto_connect(controller: State<'_, Controller>, on: bool) {
    controller.set_auto_connect(on);
}

#[tauri::command]
fn set_name(controller: State<'_, Controller>, name: String) -> String {
    controller.set_name(&name)
}

#[tauri::command]
fn logs(controller: State<'_, Controller>) -> Vec<String> {
    controller.logs()
}

#[tauri::command]
async fn check_update(controller: State<'_, Controller>) -> Result<UpdateView, String> {
    Ok(controller.check_update().await)
}

#[tauri::command]
fn set_check_updates(controller: State<'_, Controller>, on: bool) {
    controller.set_check_updates(on);
}

/// 安卓上 App 不能自己装别的 APK：把下载地址交给浏览器，下载完由系统安装器装
#[tauri::command]
async fn apply_update(
    controller: State<'_, Controller>,
    vpn: State<'_, tauri_plugin_meshora_vpn::Vpn<tauri::Wry>>,
) -> Result<(), String> {
    match controller.apply_update().await? {
        Some(url) => vpn.open_url(&url),
        None => Ok(()),
    }
}

/// 安卓上的虚拟网卡：先要 VPN 权限，再请 VpnService 建，描述符交给数据面
#[cfg(target_os = "android")]
fn tun_opener(vpn: tauri_plugin_meshora_vpn::Vpn<tauri::Wry>) -> meshorad::TunOpener {
    use std::io;

    use tauri_plugin_meshora_vpn::Establish;

    /// 除了 overlay 网段，广播和组播也进网卡：局域网游戏找房间靠它们。系统不收的会跳过
    const LAN_ROUTES: [&str; 2] = ["255.255.255.255/32", "224.0.0.0/4"];

    std::sync::Arc::new(move |config: &meshorad::TunConfig| {
        // 要权限时会弹系统对话框、等用户点：别占着异步运行时的线程干等
        tokio::task::block_in_place(|| {
            if !vpn.prepare().map_err(io::Error::other)? {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "没有允许 Meshora 建立 VPN 连接",
                ));
            }
            let fd = vpn
                .establish(&Establish {
                    address: config.address.to_string(),
                    prefix: config.prefix_len,
                    mtu: config.mtu,
                    routes: LAN_ROUTES.iter().map(|r| (*r).to_owned()).collect(),
                })
                .map_err(io::Error::other)?;
            meshorad::Tun::from_fd(config.name.clone(), fd)
        })
    })
}

/// 启动 App。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "android")]
    let buffer = LogBuffer::with_mirror(logcat::write);
    #[cfg(not(target_os = "android"))]
    let buffer = LogBuffer::default();
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_ansi(false)
        .with_writer(buffer.clone())
        .init();
    #[cfg(target_os = "android")]
    logcat::log_panics();

    tauri::Builder::default()
        .plugin(tauri_plugin_meshora_vpn::init())
        .setup(move |app| {
            let dir = app.path().app_local_data_dir()?;
            #[cfg(target_os = "android")]
            let open_tun = Some(tun_opener(
                app.state::<tauri_plugin_meshora_vpn::Vpn<tauri::Wry>>()
                    .inner()
                    .clone(),
            ));
            #[cfg(not(target_os = "android"))]
            let open_tun = None;
            let controller = Controller::with_tun_opener(dir, buffer.clone(), open_tun)?;
            let starting = controller.clone();
            tauri::async_runtime::spawn(async move { starting.start_up().await });
            app.manage(controller);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            overview,
            connect,
            disconnect,
            forget,
            set_auto_connect,
            set_name,
            create,
            admin,
            set_onboarded,
            add_server,
            remove_server,
            logs,
            check_update,
            set_check_updates,
            apply_update
        ])
        .run(tauri::generate_context!())
        .expect("Meshora 启动失败");
}
