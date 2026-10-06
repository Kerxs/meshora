//! Meshora 的桌面客户端：贴一个网络码，和朋友组成虚拟局域网。
//!
//! - [`controller`]：节点的启停和状态汇总。不依赖界面，在哪个平台都能编译、能测
//! - [`store`]：私钥和设置存在哪、怎么存
//! - [`logs`]：日志留在内存里给界面看（客户端没有控制台）
//! - `app`：Tauri 那一层，只在 Windows 上有。界面是 `ui/` 下的静态文件，由系统自带的 WebView2 显示
//!
//! 节点直接跑在客户端进程里，所以客户端要以管理员身份运行（建虚拟网卡需要）：
//! 可执行文件的清单里写了，双击时系统会弹 UAC。

#[cfg(windows)]
pub mod app;
pub mod controller;
pub mod host;
pub mod logs;
pub mod natpmp;
pub mod store;
pub mod update;
