use std::net::SocketAddr;

use crate::NodeKey;

/// 两个节点之间流量实际走的那条路。
///
/// 同一对节点之间的路径会随时间变化：由控制面根据链路探测的结果选定，数据面只负责照着走。
/// 见设计文档[核心概念 · 路径](https://kerxs.github.io/meshora/guide/concepts#路径)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Path {
    /// 直连：对端在这个 UDP 地址上可达 —— 打洞成功了，或者两边本来就在同一个局域网里。
    Direct(SocketAddr),
    /// 经中继：报文交给中继节点，由它转交对端。
    ///
    /// 中继看得到报文要交给谁，看不到内容：它转发的是已经被 WireGuard 加密过的报文。
    Relay {
        /// 中继节点的身份。连接中继时用它认证对方。
        relay: NodeKey,
        /// 中继节点的地址。和中继之间走什么传输（UDP 还是 TCP/TLS）是 M1 的决定。
        addr: SocketAddr,
    },
}
