//! Meshora 的 WireGuard 数据面：基于 boringtun 实现 [`meshora_dataplane`] 的契约。
//!
//! 分两层：
//!
//! - [`engine::Engine`]：不做 I/O 的状态机。喂进报文，吐出"往哪条链路发什么、往虚拟网卡写什么、
//!   交给控制面什么事件"。契约的不变量在这一层落地，所以不需要真的网络就能完整测试
//! - [`userspace::UserspaceDataPlane`]：把引擎接到 tokio 的 UDP socket 上。虚拟网卡那一侧
//!   只是一对 channel，真正的网卡由 meshora-tun 接上
//!
//! 还没有：中继客户端。经中继的报文目前被丢弃。

pub mod engine;
#[cfg(test)]
mod testutil;
pub mod userspace;

pub use userspace::{TunChannels, UserspaceDataPlane};
