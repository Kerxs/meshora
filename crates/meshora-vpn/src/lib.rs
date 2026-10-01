//! 安卓的 VpnService：安卓上 App 没有 root，建不了网卡，要请系统建。
//!
//! 1. [`Vpn::prepare`]：要 VPN 权限。第一次会弹系统对话框"Meshora 想要设置 VPN 连接"，等用户点
//! 2. `Vpn::establish`（只在安卓上有）：按协调服务分的地址建网卡，拿回文件描述符，交给 `meshora_tun::Tun::from_fd`
//!
//! 网卡只接管 overlay 网段（加上广播、组播），上网的流量不经过它；Meshora 自己的流量也排除在外。
//! 描述符关掉，网卡就没了。Kotlin 那一半在 `android/` 下。
//!
//! 只在 Windows 和安卓上有内容：Windows 上编得过（检查用），用不了。
#![cfg(any(windows, target_os = "android"))]

use serde::Serialize;
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime};

/// 建网卡要的参数。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Establish {
    /// 本机在 overlay 里的地址。
    pub address: String,
    /// 前缀长度：系统据此把整个网段路由进网卡。
    pub prefix: u8,
    /// MTU。
    pub mtu: u16,
    /// 额外交给网卡的路由（`地址/前缀`），比如广播、组播。系统不收的会跳过。
    pub routes: Vec<String>,
}

/// 插件的把手：`app.state::<Vpn<R>>()` 拿到。只有安卓上真的能用。
pub struct Vpn<R: Runtime> {
    #[cfg(target_os = "android")]
    handle: tauri::plugin::PluginHandle<R>,
    #[cfg(not(target_os = "android"))]
    _runtime: std::marker::PhantomData<fn() -> R>,
}

impl<R: Runtime> Clone for Vpn<R> {
    fn clone(&self) -> Self {
        Self {
            #[cfg(target_os = "android")]
            handle: self.handle.clone(),
            #[cfg(not(target_os = "android"))]
            _runtime: std::marker::PhantomData,
        }
    }
}

#[cfg(target_os = "android")]
#[derive(serde::Deserialize)]
struct Granted {
    granted: bool,
}

#[cfg(target_os = "android")]
#[derive(serde::Deserialize)]
struct Fd {
    fd: i32,
}

impl<R: Runtime> Vpn<R> {
    /// 要 VPN 权限。给过就马上返回真；没给过弹系统对话框，**阻塞到用户点了为止**。
    /// 用户拒绝返回假。
    pub fn prepare(&self) -> Result<bool, String> {
        #[cfg(target_os = "android")]
        {
            self.handle
                .run_mobile_plugin::<Granted>("prepare", ())
                .map(|answer| answer.granted)
                .map_err(|err| err.to_string())
        }
        #[cfg(not(target_os = "android"))]
        Err("只有安卓上有 VpnService".into())
    }

    /// 建网卡，交回它的文件描述符（归调用方所有，关掉网卡就没了）。要先 [`prepare`](Self::prepare) 过。
    #[cfg(target_os = "android")]
    pub fn establish(&self, request: &Establish) -> Result<std::os::fd::OwnedFd, String> {
        use std::os::fd::FromRawFd;
        let Fd { fd } = self
            .handle
            .run_mobile_plugin::<Fd>("establish", request.clone())
            .map_err(|err| err.to_string())?;
        if fd < 0 {
            return Err(format!("VpnService 交回了一个无效的描述符 {fd}"));
        }
        // SAFETY: Kotlin 那边用 ParcelFileDescriptor.detachFd() 交出了这个描述符的所有权，
        // 之后只有我们会关它
        #[allow(unsafe_code)]
        Ok(unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) })
    }

    /// 用浏览器（或者别的能打开它的 App）打开一个 https 地址：下载新版本的 APK 用。
    pub fn open_url(&self, url: &str) -> Result<(), String> {
        if !url.starts_with("https://") {
            return Err("只打开 https 地址".into());
        }
        #[cfg(target_os = "android")]
        {
            #[derive(Serialize)]
            struct Open<'a> {
                url: &'a str,
            }
            self.handle
                .run_mobile_plugin::<serde_json::Value>("open", Open { url })
                .map(drop)
                .map_err(|err| err.to_string())
        }
        #[cfg(not(target_os = "android"))]
        Err("只有安卓上能这样打开".into())
    }

    /// 停掉 VpnService（网卡已经随描述符关掉了，这里只是让服务退出）。
    pub fn stop(&self) {
        #[cfg(target_os = "android")]
        {
            let _ = self
                .handle
                .run_mobile_plugin::<serde_json::Value>("stop", ());
        }
    }
}

/// 插件本体。
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("meshora-vpn")
        .setup(|app, _api| {
            #[cfg(target_os = "android")]
            let vpn = Vpn {
                handle: _api.register_android_plugin("io.github.kerxs.meshora.vpn", "VpnPlugin")?,
            };
            #[cfg(not(target_os = "android"))]
            let vpn = Vpn::<R> {
                _runtime: std::marker::PhantomData,
            };
            app.manage(vpn);
            Ok(())
        })
        .build()
}
