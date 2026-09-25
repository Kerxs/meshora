//! Meshora 各 crate 共用的词汇。
//!
//! 这里只放基础类型：不做网络 I/O，也不依赖任何其他 Meshora crate。控制面、数据面、
//! 协调服务、中继都依赖它，所以它必须是依赖图里最底下的那一层。
//!
//! 内容对应设计文档里三个核心概念中的两个：[`NodeKey`] 是
//! [Node](https://kerxs.github.io/meshora/guide/concepts#node) 的身份（[`NodeSecret`] 是它的私钥），
//! [`Path`] 是[路径](https://kerxs.github.io/meshora/guide/concepts#路径)。第三个概念"能力"
//! 属于 M3 的能力声明与授权模型，等那边的设计定下来再进来。

mod key;
mod path;
mod secret;

pub use key::{NodeKey, ParseNodeKeyError};
pub use path::Path;
pub use secret::NodeSecret;

/// Meshora 控制报文开头的魔数。
///
/// 数据面靠它把控制报文和 WireGuard 报文分开（见 meshora-dataplane 的 `classify`），
/// 控制面按它封装报文（见 meshora-proto 的 `disco`），连接前导也以它开头。
/// 放在这里是为了只有一个来源：两边各写一份，迟早会有一边改漏。
pub const CONTROL_MAGIC: [u8; 4] = *b"MSHR";
