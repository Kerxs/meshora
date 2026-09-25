//! Meshora 各 crate 共用的词汇。
//!
//! 这里只放纯数据：没有 I/O，也不依赖任何其他 Meshora crate。控制面、数据面、
//! 协调服务、中继都依赖它，所以它必须是依赖图里最底下的那一层。
//!
//! 内容对应设计文档里三个核心概念中的两个：[`NodeKey`] 是
//! [Node](https://kerxs.github.io/meshora/guide/concepts#node) 的身份，[`Path`] 是
//! [路径](https://kerxs.github.io/meshora/guide/concepts#路径)。第三个概念"能力"
//! 属于 M3 的能力声明与授权模型，等那边的设计定下来再进来。

mod key;
mod path;

pub use key::{NodeKey, ParseNodeKeyError};
pub use path::Path;
