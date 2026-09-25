//! 数据面，以及控制面与数据面之间的接口契约。
//!
//! # 现在有什么
//!
//! M0 只有契约：[`DataPlane`] trait、它收发的类型，以及共享 socket 上的分流规则
//! [`classify`]。**还没有任何实现**，boringtun 数据面是 M1 的事。
//!
//! # 边界
//!
//! 数据面负责"加密、发出去"：WireGuard（boringtun 的 `Tunn`，一个自己不做 I/O 的状态机）、
//! UDP socket、虚拟网卡，以及把报文交给中继的那一步 —— 最后这步在每个报文的热路径上，
//! 所以归数据面。
//!
//! 控制面负责"和谁、走哪条路"：身份、发现、端点探测、穿透协调、链路探测、选路、ACL。
//!
//! 控制面通过 [`DataPlane`] 驱动数据面，数据面通过 [`EventSink`] 把事件交回控制面。
//! 两边的 crate 互不依赖，这里的契约是它们唯一的接触面。
//!
//! # 不变量
//!
//! 1. **私钥不出数据面。** trait 里没有任何读私钥的方法，私钥只在实现的构造函数里交进来。
//! 2. **[`DataPlane::apply`] 是声明式、原子的。** 交进来的是完整的期望状态，
//!    仍在集合里的 peer 保留路径和会话。
//! 3. **发送路径只能由 [`DataPlane::set_path`] 改变。** 数据面不做 WireGuard 式的漫游。
//! 4. **控制报文和 WireGuard 共用同一个 UDP socket。** 打洞凿出来的 NAT 映射必须就是
//!    WireGuard 用的那个。
//! 5. **[`Event::ControlDatagram`] 里的数据未经认证。** 数据面只按魔数分流。
//!
//! 每一条的理由写在对应方法的文档里，设计文档里也有一份：
//! <https://kerxs.github.io/meshora/guide/interfaces>

mod config;
mod datagram;

use std::error::Error;
use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::time::Instant;

use meshora_types::{NodeKey, Path};

pub use config::{ConfigError, PeerConfig, PeerSet};
pub use datagram::{CONTROL_MAGIC, DatagramKind, WgMessage, classify};

/// 控制面驱动数据面的接口。
///
/// 方法都是同步的、取 `&self`：契约不绑定任何 async 运行时，实现可以放进
/// `Arc<dyn DataPlane>` 在多处共享，并发安全由实现内部保证。但**配置的先后由调用方负责**：
/// 控制面里应当只有一处调用 [`apply`](Self::apply)。
///
/// 构造不在 trait 里。本机私钥和 [`EventSink`] 都在实现的构造函数里交进来 ——
/// 私钥交进去就不再出来。
pub trait DataPlane: Send + Sync {
    /// 用一份完整的 peer 集合替换当前配置。
    ///
    /// - 集合里没有的 peer 被删除，会话随之作废
    /// - 新出现的 peer 被加入，此时**没有路径**，发给它的报文被丢弃，直到
    ///   [`set_path`](Self::set_path)
    /// - 两边都有的 peer 更新配置，但**保留路径和 WireGuard 会话**：路径是运行时状态，
    ///   不是配置，改个 keepalive 不该让连接断一下
    ///
    /// 要么整体生效，要么保持原样。集合里出现本机公钥时返回
    /// [`DataPlaneError::SelfPeer`]。[`PeerSet`] 自身的合法性在构造时已经检查过。
    fn apply(&self, peers: &PeerSet) -> Result<(), DataPlaneError>;

    /// 切换一个 peer 的发送路径。
    ///
    /// 这是改变发送路径的**唯一**入口。数据面不做 WireGuard 式的漫游：收到来自新地址的
    /// 合法报文，也不会自动把发送路径切过去。否则一个被重放的旧报文就能诱导它把流量
    /// 发往别处，而且会和控制面的选路互相打架。
    ///
    /// 对收到报文的回应（握手响应、cookie 回复）原路返回，不受这里影响；数据面主动发出的
    /// 报文（数据、keepalive、握手发起）一律走这里设定的路径。
    ///
    /// peer 不在当前集合里时返回 [`DataPlaneError::UnknownPeer`]。
    fn set_path(&self, peer: &NodeKey, path: Path) -> Result<(), DataPlaneError>;

    /// 从 WireGuard 用的**同一个** UDP socket 发出一个控制报文。
    ///
    /// 必须是同一个 socket：打洞凿出来的 NAT 映射只对这个本地端口有效，
    /// 换一个 socket 发，凿出来的洞 WireGuard 用不上。
    ///
    /// `datagram` 必须以 [`CONTROL_MAGIC`] 开头，否则返回
    /// [`DataPlaneError::NotControlDatagram`]：控制面不能借这个口子，发出会被对端当成
    /// WireGuard 报文的东西。
    fn send_control(&self, to: SocketAddr, datagram: &[u8]) -> Result<(), DataPlaneError>;

    /// 所有 peer 的当前状态，顺序不做保证。
    fn status(&self) -> Vec<PeerStatus>;
}

/// 一个 peer 的运行时状态快照。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerStatus {
    /// peer 的身份。
    pub key: NodeKey,
    /// 当前的发送路径。还没 [`set_path`](DataPlane::set_path) 过的 peer 为 `None`。
    pub path: Option<Path>,
    /// 最近一次握手完成的时刻，从未握手成功为 `None`。
    ///
    /// 用单调时钟：判断会话是否还活着，不该受系统时间被调整的影响。
    pub last_handshake: Option<Instant>,
    /// 从这个 peer 收到的字节数，按 WireGuard 报文计（含 WireGuard 自身的开销，不含中继的封装）。
    pub rx_bytes: u64,
    /// 发给这个 peer 的字节数，计法同 [`rx_bytes`](Self::rx_bytes)。
    pub tx_bytes: u64,
}

/// 数据面交回控制面的事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// 和 `peer` 完成了一次 WireGuard 握手。
    ///
    /// `via` 是握手报文实际走的路径，不一定是当前的发送路径 —— 对端可能是从一个
    /// 刚打通的地址发起的握手。要不要切过去由控制面决定，见 [`DataPlane::set_path`]。
    HandshakeCompleted {
        /// 完成握手的 peer。
        peer: NodeKey,
        /// 握手报文实际走的路径。
        via: Path,
    },
    /// 共享 socket 上收到一个控制报文（以 [`CONTROL_MAGIC`] 开头）。
    ///
    /// 数据面只按魔数分流，**不做任何认证**：`from` 可以伪造，内容可以是任意字节。
    /// 控制面必须先验证，再相信它。
    ControlDatagram {
        /// 报文的来源地址。
        from: SocketAddr,
        /// 整个报文，包括开头的魔数。
        datagram: Vec<u8>,
    },
}

/// 数据面把 [`Event`] 交回控制面的出口。由控制面实现，在构造数据面时交给它。
///
/// [`emit`](Self::emit) 在数据面的收发循环里被调用，**不能阻塞** —— 这里卡一下，
/// 所有流量都跟着停。控制面处理不过来时可以丢弃 [`Event::ControlDatagram`]：UDP 本来就
/// 不保证送达，而且这是未认证的外部输入，被人灌满时必须能丢。但不能丢
/// [`Event::HandshakeCompleted`]。
///
/// 闭包可以直接当 `EventSink` 用。
pub trait EventSink: Send + Sync {
    /// 投递一个事件。不能阻塞。
    fn emit(&self, event: Event);
}

impl<F> EventSink for F
where
    F: Fn(Event) + Send + Sync,
{
    fn emit(&self, event: Event) {
        self(event)
    }
}

/// [`DataPlane`] 的操作失败。
#[derive(Debug)]
pub enum DataPlaneError {
    /// 交给 [`apply`](DataPlane::apply) 的集合里有本机自己的公钥。
    SelfPeer,
    /// 这个 peer 不在当前集合里。
    UnknownPeer(NodeKey),
    /// 交给 [`send_control`](DataPlane::send_control) 的报文不以 [`CONTROL_MAGIC`] 开头。
    NotControlDatagram,
    /// 底层 I/O 出错。
    Io(io::Error),
}

impl fmt::Display for DataPlaneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SelfPeer => f.write_str("peer 集合里有本机自己的公钥"),
            Self::UnknownPeer(key) => write!(f, "peer {key} 不在当前集合里"),
            Self::NotControlDatagram => f.write_str("控制报文必须以 CONTROL_MAGIC 开头"),
            Self::Io(err) => write!(f, "数据面 I/O 出错：{err}"),
        }
    }
}

impl Error for DataPlaneError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for DataPlaneError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[test]
    fn data_plane_can_be_shared_as_trait_object() {
        // 平台相关的实现要能藏在 Arc<dyn DataPlane> 后面，交给控制面的多个任务共用
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn DataPlane>();
        let _: Option<Arc<dyn DataPlane>> = None;
    }

    #[test]
    fn closure_works_as_event_sink() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink: Arc<dyn EventSink> = Arc::new({
            let seen = Arc::clone(&seen);
            move |event| seen.lock().unwrap().push(event)
        });

        let event = Event::ControlDatagram {
            from: "192.0.2.1:41641".parse().unwrap(),
            datagram: CONTROL_MAGIC.to_vec(),
        };
        sink.emit(event.clone());
        assert_eq!(*seen.lock().unwrap(), [event]);
    }

    #[test]
    fn io_error_is_kept_as_source() {
        let err = DataPlaneError::from(io::Error::other("socket 关了"));
        assert!(err.source().is_some());
        assert!(err.to_string().contains("socket 关了"));
        assert!(DataPlaneError::SelfPeer.source().is_none());
    }
}
