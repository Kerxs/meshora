//! Meshora 节点、协调服务、中继之间的线协议。
//!
//! | 通道 | 两端 | 传输 | 认证 |
//! | --- | --- | --- | --- |
//! | 控制通道 | 节点 ↔ 协调服务 | TCP | Noise IK（[`noise`]），消息见 [`control`] |
//! | 中继通道 | 节点 ↔ 中继 | TCP | Noise IK（[`noise`]） |
//! | 控制报文 | 节点 ↔ 节点、节点 ↔ 探测端点 | UDP，和 WireGuard 共用 socket | Noise K，每条一次（[`disco`]） |
//!
//! Noise 的原语和 WireGuard 同一套：Curve25519、ChaChaPoly、BLAKE2s。所有静态密钥都是节点的
//! WireGuard 密钥 —— 公钥即身份。同一把密钥用在几种协议里，靠各自不同的 Noise 模式和 prologue
//! 做域分离：一种协议里的消息，拿到另一种协议里解不开。
//!
//! 编解码是手写的（[`codec`]）：进来的都是不可信输入，每一次读都查边界。

pub mod codec;
pub mod control;
pub mod disco;
pub mod noise;
