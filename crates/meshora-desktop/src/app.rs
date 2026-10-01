//! Tauri 那一层：窗口、托盘、界面能调的命令、退出时收拾。决定都交给 [`Controller`]。

use std::time::Duration;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, State, WindowEvent};

use crate::controller::{AdminAction, Controller, CreateAt, Overview};
use crate::logs::LogBuffer;
use crate::update::UpdateView;

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
async fn set_prefer_broadcast(controller: State<'_, Controller>, on: bool) -> Result<(), String> {
    controller.set_prefer_broadcast(on).await
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
fn set_private_network(controller: State<'_, Controller>, on: bool) {
    controller.set_private_network(on);
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

/// 下载、核对新版本，运行安装程序，然后退出：安装程序装好会再把客户端打开
#[tauri::command]
async fn apply_update(app: AppHandle, controller: State<'_, Controller>) -> Result<(), String> {
    controller.apply_update().await?;
    // 退出时照常把节点停干净（RunEvent::Exit）；安装程序会等客户端退出再动文件
    app.exit(0);
    Ok(())
}

/// 托盘提示多久刷新一次
const TRAY_REFRESH: Duration = Duration::from_secs(2);

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<TrayIcon> {
    let show = MenuItem::with_id(app, "show", "打开 Meshora", true, None::<&str>)?;
    let disconnect = MenuItem::with_id(app, "disconnect", "断开连接", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出（断开连接）", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &disconnect, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("Meshora")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "disconnect" => {
                let controller = app.state::<Controller>().inner().clone();
                tauri::async_runtime::spawn(async move { controller.disconnect().await });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)
}

/// 托盘上的提示跟着状态走：鼠标移上去就知道连没连着、自己的地址是多少
fn tray_tooltip(controller: &Controller) -> String {
    let overview = controller.overview();
    match (overview.phase, overview.me) {
        ("connected", Some(me)) => {
            let online = overview.peers.iter().filter(|peer| peer.online).count();
            format!("Meshora · 已连接 {} · {online} 人在线", me.ip)
        }
        ("connecting", _) => "Meshora · 连接中".into(),
        ("failed", _) => "Meshora · 未连上".into(),
        _ => "Meshora · 未连接".into(),
    }
}

/// 启动客户端，直到窗口关掉。
pub fn run() {
    let buffer = LogBuffer::default();
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(buffer.clone())
        .init();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // 又双击了一次：把开着的窗口叫出来，不起第二份
            show_main(app);
        }))
        .setup(move |app| {
            let dir = app.path().app_local_data_dir()?;
            let controller = Controller::new(dir, buffer.clone())?;
            let starting = controller.clone();
            tauri::async_runtime::spawn(async move { starting.start_up().await });

            let tray = build_tray(app.handle())?;
            let watching = controller.clone();
            tauri::async_runtime::spawn(async move {
                let mut last = String::new();
                loop {
                    let tooltip = tray_tooltip(&watching);
                    if tooltip != last {
                        let _ = tray.set_tooltip(Some(&tooltip));
                        last = tooltip;
                    }
                    tokio::time::sleep(TRAY_REFRESH).await;
                }
            });

            app.manage(controller);
            Ok(())
        })
        .on_window_event(|window, event| {
            // 连着网的时候关窗口只是藏到托盘：游戏还在联机，关个窗口不该把它断了。
            // 没连着就真的退出
            if let WindowEvent::CloseRequested { api, .. } = event {
                let controller = window.state::<Controller>();
                if matches!(controller.overview().phase, "connected" | "connecting") {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            overview,
            connect,
            disconnect,
            forget,
            set_prefer_broadcast,
            set_auto_connect,
            set_private_network,
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
        .build(tauri::generate_context!())
        .expect("客户端启动失败");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            // 退出前把节点停干净：虚拟网卡删掉，别留在系统里
            let controller = app.state::<Controller>().inner().clone();
            tauri::async_runtime::block_on(controller.disconnect());
        }
    });
}
