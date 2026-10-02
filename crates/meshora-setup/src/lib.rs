//! Meshora 安装程序里平台无关、能测的部分：编进来的载荷。
//!
//! 可执行文件内嵌了"要求管理员权限"的清单，它的测试程序在普通用户下起不来，所以要测的放在库里。

pub mod location;
pub mod payload;

/// Windows 防火墙里放行 Meshora 收 UDP。
#[cfg(windows)]
pub mod firewall;

/// 读、收紧文件夹的权限（Windows）。
#[cfg(windows)]
#[allow(unsafe_code)]
pub mod acl;
