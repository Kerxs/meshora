//! 数据面的核心：一个不做 I/O 的状态机。
//!
//! 思路和 boringtun 的 `Tunn` 一样：把报文喂进来，它告诉你"该往哪发什么、该往虚拟网卡写什么"。
//! socket、虚拟网卡、定时器都在外面，这里只管规矩 —— 契约的不变量都在这一层落地，
//! 所以这一层能被完整地单元测试，不需要真的网络。

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use boringtun::noise::handshake::parse_handshake_anon;
use boringtun::noise::rate_limiter::RateLimiter;
use boringtun::noise::{Packet, Tunn, TunnResult};
use meshora_types::{NodeKey, NodeSecret, Path};
use rand_core::{OsRng, RngCore};
use x25519_dalek::{PublicKey, StaticSecret};

use crate::{DataPlaneError, DatagramKind, Event, PeerSet, PeerStatus, WgMessage, classify};

/// 每秒最多处理多少个握手报文，超过就要求对端先用 cookie 证明自己的地址。
///
/// 和 boringtun 自带的 device 模块取同一个值。注意每个握手报文会被计两次（先在这里预检，
/// 再在 `Tunn::decapsulate` 里），所以实际触发 cookie 的门槛是每秒 50 个。
const HANDSHAKE_RATE_LIMIT: u64 = 100;

/// 暂存缓冲区：装得下最大的 UDP 报文，再留出 WireGuard 的 32 字节开销。
const BUF_LEN: usize = u16::MAX as usize + 32;

/// boringtun 把 peer 编号左移 8 位当会话索引，所以编号只有 24 位可用。
const INDEX_MASK: u32 = 0x00FF_FFFF;

/// 报文实际经过的链路：[`Path`] 再加上"经中继时，对端是谁"。
///
/// 收报文时，经中继来的报文由中继告诉我们发送方是谁（中继认证过它）；
/// 发报文时，中继要知道交给谁。直连时有地址就够了。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Link {
    /// 直连的 UDP 地址。
    Direct(SocketAddr),
    /// 经中继。
    Relay {
        /// 中继节点的身份。
        relay: NodeKey,
        /// 中继节点的地址。
        addr: SocketAddr,
        /// 链路另一头的节点：收报文时是发送方，发报文时是接收方。
        peer: NodeKey,
    },
}

impl Link {
    /// 去掉对端信息之后的路径。
    pub fn path(&self) -> Path {
        match *self {
            Self::Direct(addr) => Path::Direct(addr),
            Self::Relay { relay, addr, .. } => Path::Relay { relay, addr },
        }
    }

    /// 对 `peer` 走 `path` 时的链路。
    pub fn to_peer(path: Path, peer: NodeKey) -> Self {
        match path {
            Path::Direct(addr) => Self::Direct(addr),
            Path::Relay { relay, addr } => Self::Relay { relay, addr, peer },
        }
    }

    /// 这条链路另一头的 IP。WireGuard 的 cookie 机制按它来验证地址。
    fn ip(&self) -> IpAddr {
        match self {
            Self::Direct(addr) | Self::Relay { addr, .. } => addr.ip(),
        }
    }
}

/// 要发往网络的一个报文。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transmit {
    /// 走哪条链路。
    pub link: Link,
    /// WireGuard 报文。
    pub datagram: Vec<u8>,
}

/// 处理完一个收到的报文之后，外面该做的事。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// 往网络发一个报文。
    Transmit(Transmit),
    /// 往虚拟网卡写一个 IP 报文。
    WriteTun(Vec<u8>),
    /// 交给控制面的事件。
    Event(Event),
}

struct Peer {
    tunn: Tunn,
    /// 24 位的 peer 编号，收到的报文靠它找回是哪个 peer
    index: u32,
    path: Option<Path>,
    last_handshake: Option<Instant>,
    /// 最近一次往这个 peer 发报文的时刻，persistent keepalive 按它计时
    last_sent: Option<Instant>,
    rx_bytes: u64,
    tx_bytes: u64,
}

impl Peer {
    /// 记一笔发送，并包装成要发出去的报文
    fn transmit(&mut self, link: Link, datagram: &[u8], now: Instant) -> Transmit {
        self.tx_bytes += datagram.len() as u64;
        self.last_sent = Some(now);
        Transmit {
            link,
            datagram: datagram.to_vec(),
        }
    }
}

/// 数据面的核心状态机。
///
/// 不是线程安全的，外面包一层锁。所有方法都不阻塞、不做 I/O。
pub struct Engine {
    secret: StaticSecret,
    public: PublicKey,
    local: NodeKey,
    rate_limiter: Arc<RateLimiter>,
    config: PeerSet,
    peers: HashMap<NodeKey, Peer>,
    by_index: HashMap<u32, NodeKey>,
    buf: Vec<u8>,
}

impl Engine {
    /// 用本机私钥创建。私钥交进来之后就拿不出去了（不变量 1）。
    pub fn new(secret: &NodeSecret) -> Self {
        let secret = secret.to_static_secret();
        let public = PublicKey::from(&secret);
        Self {
            rate_limiter: Arc::new(RateLimiter::new(&public, HANDSHAKE_RATE_LIMIT)),
            local: NodeKey::from_bytes(public.to_bytes()),
            secret,
            public,
            config: PeerSet::default(),
            peers: HashMap::new(),
            by_index: HashMap::new(),
            buf: vec![0; BUF_LEN],
        }
    }

    /// 本机身份。
    pub fn local_key(&self) -> NodeKey {
        self.local
    }

    /// 见 [`DataPlane::apply`](crate::DataPlane::apply)：声明式、原子，保留仍在集合里的 peer 的路径和会话。
    pub fn apply(&mut self, peers: &PeerSet) -> Result<(), DataPlaneError> {
        if peers.get(&self.local).is_some() {
            return Err(DataPlaneError::SelfPeer);
        }

        let by_index = &mut self.by_index;
        self.peers.retain(|key, peer| {
            let keep = peers.get(key).is_some();
            if !keep {
                by_index.remove(&peer.index);
            }
            keep
        });

        for config in peers.iter() {
            if self.peers.contains_key(&config.key) {
                continue;
            }
            let index = self.allocate_index();
            // keepalive 不交给 Tunn：它建好之后改不了，而契约要求改 keepalive 不能断会话。
            // persistent keepalive 在 timers() 里自己做
            let tunn = Tunn::new(
                self.secret.clone(),
                PublicKey::from(*config.key.as_bytes()),
                None,
                None,
                index,
                Some(Arc::clone(&self.rate_limiter)),
            );
            self.by_index.insert(index, config.key);
            self.peers.insert(
                config.key,
                Peer {
                    tunn,
                    index,
                    path: None,
                    last_handshake: None,
                    last_sent: None,
                    rx_bytes: 0,
                    tx_bytes: 0,
                },
            );
        }

        self.config = peers.clone();
        Ok(())
    }

    /// 见 [`DataPlane::set_path`](crate::DataPlane::set_path)。
    ///
    /// peer 第一次有了路径、又还没有会话时，立刻发起握手，不必等 WireGuard 5 秒一次的重传 ——
    /// "中继先行"要的就是马上能通。
    pub fn set_path(
        &mut self,
        key: &NodeKey,
        path: Path,
        now: Instant,
    ) -> Result<Vec<Transmit>, DataPlaneError> {
        let peer = self
            .peers
            .get_mut(key)
            .ok_or(DataPlaneError::UnknownPeer(*key))?;
        let had_path = peer.path.replace(path).is_some();

        let mut out = Vec::new();
        if !had_path
            && peer.tunn.time_since_last_handshake().is_none()
            && let TunnResult::WriteToNetwork(datagram) =
                peer.tunn.format_handshake_initiation(&mut self.buf, true)
        {
            out.push(peer.transmit(Link::to_peer(path, *key), datagram, now));
        }
        Ok(out)
    }

    /// 从虚拟网卡读到一个 IP 报文：找到该发给哪个 peer，加密。
    ///
    /// 没有 peer 的网段覆盖目的地址、或者那个 peer 还没有路径时，报文被丢弃。
    pub fn outbound(&mut self, packet: &[u8], now: Instant) -> Option<Transmit> {
        let dst = Tunn::dst_address(packet)?;
        let key = *self.config.route(dst)?;
        let peer = self.peers.get_mut(&key)?;
        match peer.tunn.encapsulate(packet, &mut self.buf) {
            // 没有路径时，Tunn 可能已经把报文排进队列、生成了握手发起。
            // 发起丢掉没关系：set_path 时会重新发，报文随后跟上
            TunnResult::WriteToNetwork(datagram) => {
                let path = peer.path?;
                Some(peer.transmit(Link::to_peer(path, key), datagram, now))
            }
            _ => None,
        }
    }

    /// 从网络收到一个报文。
    pub fn inbound(&mut self, datagram: &[u8], link: Link, now: Instant) -> Vec<Action> {
        let mut actions = Vec::new();
        match classify(datagram) {
            DatagramKind::WireGuard(kind) => {
                self.inbound_wireguard(datagram, kind, link, now, &mut actions)
            }
            // 经中继来的控制报文走什么路还没定（见接口契约的"还没定的"），先丢掉
            DatagramKind::Control => {
                if let Link::Direct(from) = link {
                    actions.push(Action::Event(Event::ControlDatagram {
                        from,
                        datagram: datagram.to_vec(),
                    }));
                }
            }
            DatagramKind::Unknown => {}
        }
        actions
    }

    fn inbound_wireguard(
        &mut self,
        datagram: &[u8],
        kind: WgMessage,
        link: Link,
        now: Instant,
        actions: &mut Vec<Action>,
    ) {
        let src_ip = Some(link.ip());

        // 先过一遍限速器：验证 mac1，负载高时要求 cookie。
        // 必须在认出发送方之前做 —— 认发送方要做一次 DH，不能让没验证过的报文随便触发
        let packet = match self
            .rate_limiter
            .verify_packet(src_ip, datagram, &mut self.buf)
        {
            Ok(packet) => packet,
            Err(TunnResult::WriteToNetwork(cookie)) => {
                actions.push(Action::Transmit(Transmit {
                    link,
                    datagram: cookie.to_vec(),
                }));
                return;
            }
            Err(_) => return,
        };

        let Some(key) = self.identify(&packet) else {
            return;
        };
        // 中继报告的发送方必须和密码学上认出来的一致，否则回应会被送到别处
        if let Link::Relay { peer, .. } = link
            && peer != key
        {
            return;
        }
        let Some(peer) = self.peers.get_mut(&key) else {
            return;
        };
        let Some(config) = self.config.get(&key) else {
            return;
        };
        peer.rx_bytes += datagram.len() as u64;

        let mut input = datagram;
        let mut first = true;
        loop {
            match peer.tunn.decapsulate(src_ip, input, &mut self.buf) {
                TunnResult::WriteToNetwork(out) => {
                    let out_kind = classify(out);
                    let completed = first
                        && matches!(
                            (kind, out_kind),
                            (
                                WgMessage::HandshakeInitiation,
                                DatagramKind::WireGuard(WgMessage::HandshakeResponse)
                            ) | (
                                WgMessage::HandshakeResponse,
                                DatagramKind::WireGuard(WgMessage::TransportData)
                            )
                        );
                    if completed {
                        peer.last_handshake = Some(now);
                        actions.push(Action::Event(Event::HandshakeCompleted {
                            peer: key,
                            via: link.path(),
                        }));
                    }

                    // 回应原路返回；主动发出的（数据、keepalive）走控制面设定的路径（不变量 3）
                    let reply_link = match out_kind {
                        DatagramKind::WireGuard(
                            WgMessage::HandshakeResponse | WgMessage::CookieReply,
                        ) => Some(link),
                        _ => peer.path.map(|path| Link::to_peer(path, key)),
                    };
                    if let Some(reply_link) = reply_link {
                        actions.push(Action::Transmit(peer.transmit(reply_link, out, now)));
                    }
                }
                TunnResult::WriteToTunnelV4(packet, src) => {
                    Self::deliver(config, packet, src.into(), actions);
                    break;
                }
                TunnResult::WriteToTunnelV6(packet, src) => {
                    Self::deliver(config, packet, src.into(), actions);
                    break;
                }
                TunnResult::Done | TunnResult::Err(_) => break,
            }
            // boringtun 的约定：返回 WriteToNetwork 之后要用空报文再调一次，把排队的报文放出来
            input = &[];
            first = false;
        }
    }

    /// 解密出来的 IP 报文，源地址必须落在这个 peer 的网段里（cryptokey routing 的另一半）
    fn deliver(config: &crate::PeerConfig, packet: &[u8], src: IpAddr, actions: &mut Vec<Action>) {
        if config.allowed_ips.iter().any(|net| net.contains(&src)) {
            actions.push(Action::WriteTun(packet.to_vec()));
        }
    }

    /// 认出一个报文属于哪个 peer：握手发起靠解出里面加密的静态公钥，其余靠接收方索引
    fn identify(&self, packet: &Packet) -> Option<NodeKey> {
        let index = match packet {
            Packet::HandshakeInit(init) => {
                let half = parse_handshake_anon(&self.secret, &self.public, init).ok()?;
                let key = NodeKey::from_bytes(half.peer_static_public);
                return self.peers.contains_key(&key).then_some(key);
            }
            Packet::HandshakeResponse(p) => p.receiver_idx,
            Packet::PacketCookieReply(p) => p.receiver_idx,
            Packet::PacketData(p) => p.receiver_idx,
        };
        self.by_index.get(&(index >> 8)).copied()
    }

    /// 定时器，外面每 250 毫秒左右调一次：握手重传、会话过期、keepalive。
    pub fn timers(&mut self, now: Instant) -> Vec<Transmit> {
        // 限速器是所有 Tunn 共用的，得由我们来清零计数
        self.rate_limiter.reset_count();

        let mut out = Vec::new();
        for (key, peer) in &mut self.peers {
            let Some(path) = peer.path else {
                continue;
            };
            let link = Link::to_peer(path, *key);
            if let TunnResult::WriteToNetwork(datagram) = peer.tunn.update_timers(&mut self.buf) {
                out.push(peer.transmit(link, datagram, now));
            }

            let keepalive = self.config.get(key).and_then(|config| config.keepalive);
            if let Some(interval) = keepalive {
                let interval = Duration::from_secs(interval.get().into());
                let due = peer
                    .last_sent
                    .is_none_or(|sent| now.duration_since(sent) >= interval);
                if due
                    && let TunnResult::WriteToNetwork(datagram) =
                        peer.tunn.encapsulate(&[], &mut self.buf)
                {
                    out.push(peer.transmit(link, datagram, now));
                }
            }
        }
        out
    }

    /// 所有 peer 的当前状态。
    pub fn status(&self) -> Vec<PeerStatus> {
        self.peers
            .iter()
            .map(|(key, peer)| PeerStatus {
                key: *key,
                path: peer.path,
                last_handshake: peer.last_handshake,
                rx_bytes: peer.rx_bytes,
                tx_bytes: peer.tx_bytes,
            })
            .collect()
    }

    /// 随机分配一个没用过的 24 位编号。顺序编号会暴露这台机器有多少个 peer
    fn allocate_index(&self) -> u32 {
        loop {
            let index = OsRng.next_u32() & INDEX_MASK;
            if !self.by_index.contains_key(&index) {
                return index;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU16;

    use super::*;
    use crate::{CONTROL_MAGIC, PeerConfig};

    /// 一个节点：引擎、它的 overlay 地址、它在测试网络里的 UDP 地址
    struct Node {
        engine: Engine,
        key: NodeKey,
        ip: IpAddr,
        addr: SocketAddr,
    }

    fn node(last_octet: u8) -> Node {
        let secret = NodeSecret::generate();
        Node {
            key: secret.public_key(),
            engine: Engine::new(&secret),
            ip: IpAddr::from([100, 64, 0, last_octet]),
            addr: SocketAddr::from(([192, 0, 2, last_octet], 41641)),
        }
    }

    fn config_for(peer: &Node) -> PeerConfig {
        PeerConfig {
            key: peer.key,
            allowed_ips: vec![ipnet::IpNet::from(peer.ip)],
            keepalive: None,
        }
    }

    /// 一个最小的 IPv4 报文：20 字节头 + 载荷。boringtun 只看版本、长度和地址
    fn ipv4(src: IpAddr, dst: IpAddr, payload: &[u8]) -> Vec<u8> {
        let (IpAddr::V4(src), IpAddr::V4(dst)) = (src, dst) else {
            panic!("只造 IPv4 报文");
        };
        let total = 20 + payload.len();
        let mut packet = vec![0u8; total];
        packet[0] = 0x45;
        packet[2..4].copy_from_slice(&(total as u16).to_be_bytes());
        packet[8] = 64;
        packet[9] = 1; // ICMP
        packet[12..16].copy_from_slice(&src.octets());
        packet[16..20].copy_from_slice(&dst.octets());
        packet[20..].copy_from_slice(payload);
        packet
    }

    fn transmits(actions: &[Action]) -> Vec<Transmit> {
        actions
            .iter()
            .filter_map(|action| match action {
                Action::Transmit(t) => Some(t.clone()),
                _ => None,
            })
            .collect()
    }

    fn written(actions: &[Action]) -> Vec<Vec<u8>> {
        actions
            .iter()
            .filter_map(|action| match action {
                Action::WriteTun(packet) => Some(packet.clone()),
                _ => None,
            })
            .collect()
    }

    fn events(actions: &[Action]) -> Vec<Event> {
        actions
            .iter()
            .filter_map(|action| match action {
                Action::Event(event) => Some(event.clone()),
                _ => None,
            })
            .collect()
    }

    /// 两个互为 peer 的节点，A 设好了到 B 的直连路径，握手已经完成
    fn connected_pair() -> (Node, Node) {
        let mut a = node(1);
        let mut b = node(2);
        let now = Instant::now();
        a.engine
            .apply(&PeerSet::new([config_for(&b)]).unwrap())
            .unwrap();
        b.engine
            .apply(&PeerSet::new([config_for(&a)]).unwrap())
            .unwrap();

        let init = a
            .engine
            .set_path(&b.key, Path::Direct(b.addr), now)
            .unwrap();
        assert_eq!(init.len(), 1, "第一次有路径就该立刻发起握手");
        let response = b
            .engine
            .inbound(&init[0].datagram, Link::Direct(a.addr), now);
        let response = transmits(&response);
        assert_eq!(response.len(), 1);
        let keepalive = a
            .engine
            .inbound(&response[0].datagram, Link::Direct(b.addr), now);
        for t in transmits(&keepalive) {
            b.engine.inbound(&t.datagram, Link::Direct(a.addr), now);
        }
        (a, b)
    }

    #[test]
    fn handshake_then_data_reaches_the_other_side() {
        let mut a = node(1);
        let mut b = node(2);
        let now = Instant::now();
        a.engine
            .apply(&PeerSet::new([config_for(&b)]).unwrap())
            .unwrap();
        b.engine
            .apply(&PeerSet::new([config_for(&a)]).unwrap())
            .unwrap();

        // 路径还没设：报文进了 Tunn 的队列，但什么也发不出去
        let packet = ipv4(a.ip, b.ip, b"ping");
        assert_eq!(a.engine.outbound(&packet, now), None);

        // 设路径：立刻发起握手
        let init = a
            .engine
            .set_path(&b.key, Path::Direct(b.addr), now)
            .unwrap();
        assert_eq!(init.len(), 1);
        assert_eq!(init[0].link, Link::Direct(b.addr));
        assert_eq!(
            classify(&init[0].datagram),
            DatagramKind::WireGuard(WgMessage::HandshakeInitiation)
        );

        // B 回应。B 还没有到 A 的路径，但握手响应按原路返回
        let actions = b
            .engine
            .inbound(&init[0].datagram, Link::Direct(a.addr), now);
        assert_eq!(
            events(&actions),
            [Event::HandshakeCompleted {
                peer: a.key,
                via: Path::Direct(a.addr),
            }]
        );
        let response = transmits(&actions);
        assert_eq!(response.len(), 1);
        assert_eq!(response[0].link, Link::Direct(a.addr));

        // A 收到响应：握手完成，排队的报文跟着发出去
        let actions = a
            .engine
            .inbound(&response[0].datagram, Link::Direct(b.addr), now);
        assert_eq!(
            events(&actions),
            [Event::HandshakeCompleted {
                peer: b.key,
                via: Path::Direct(b.addr),
            }]
        );
        let mut delivered = Vec::new();
        for t in transmits(&actions) {
            assert_eq!(t.link, Link::Direct(b.addr));
            delivered.extend(written(&b.engine.inbound(
                &t.datagram,
                Link::Direct(a.addr),
                now,
            )));
        }
        assert_eq!(delivered, [packet]);
    }

    #[test]
    fn replies_are_dropped_until_the_control_plane_sets_a_path() {
        let (mut a, mut b) = connected_pair();
        let now = Instant::now();
        // B 没有到 A 的路径（不变量 3：不因为收到过 A 的报文就自己选一条）
        let reply = ipv4(b.ip, a.ip, b"pong");
        assert_eq!(b.engine.outbound(&reply, now), None);

        // 会话已经有了，设路径不会再发起握手
        let out = b
            .engine
            .set_path(&a.key, Path::Direct(a.addr), now)
            .unwrap();
        assert!(out.is_empty());
        let t = b.engine.outbound(&reply, now).unwrap();
        assert_eq!(t.link, Link::Direct(a.addr));
        let actions = a.engine.inbound(&t.datagram, Link::Direct(b.addr), now);
        assert_eq!(written(&actions), [reply]);
    }

    #[test]
    fn does_not_roam_to_a_new_source_address() {
        let (mut a, mut b) = connected_pair();
        let now = Instant::now();
        b.engine
            .set_path(&a.key, Path::Direct(a.addr), now)
            .unwrap();

        // A 的报文从另一个地址到达：照常解密交付，但 B 的发送路径不变
        let elsewhere = SocketAddr::from(([198, 51, 100, 7], 5555));
        let t = a.engine.outbound(&ipv4(a.ip, b.ip, b"x"), now).unwrap();
        let actions = b.engine.inbound(&t.datagram, Link::Direct(elsewhere), now);
        assert_eq!(written(&actions).len(), 1);

        let reply = b.engine.outbound(&ipv4(b.ip, a.ip, b"y"), now).unwrap();
        assert_eq!(reply.link, Link::Direct(a.addr));
        let status = b.engine.status();
        assert_eq!(status[0].path, Some(Path::Direct(a.addr)));
    }

    #[test]
    fn drops_packets_from_outside_the_peers_allowed_ips() {
        let (mut a, mut b) = connected_pair();
        let now = Instant::now();
        let spoofed = ipv4(IpAddr::from([100, 64, 0, 99]), b.ip, b"spoof");
        let t = a.engine.outbound(&spoofed, now).unwrap();
        let actions = b.engine.inbound(&t.datagram, Link::Direct(a.addr), now);
        assert!(written(&actions).is_empty());
    }

    #[test]
    fn apply_keeps_sessions_of_remaining_peers() {
        let (mut a, mut b) = connected_pair();
        let now = Instant::now();
        // 改 keepalive、加一个新 peer：A 的会话必须还在
        let c = node(3);
        let mut config = config_for(&a);
        config.keepalive = NonZeroU16::new(25);
        b.engine
            .apply(&PeerSet::new([config, config_for(&c)]).unwrap())
            .unwrap();

        let packet = ipv4(a.ip, b.ip, b"still here");
        let t = a.engine.outbound(&packet, now).unwrap();
        assert_eq!(
            classify(&t.datagram),
            DatagramKind::WireGuard(WgMessage::TransportData)
        );
        let actions = b.engine.inbound(&t.datagram, Link::Direct(a.addr), now);
        assert_eq!(written(&actions), [packet]);
    }

    #[test]
    fn removed_peers_are_forgotten() {
        let (mut a, mut b) = connected_pair();
        let now = Instant::now();
        b.engine.apply(&PeerSet::default()).unwrap();
        assert!(b.engine.status().is_empty());

        let t = a.engine.outbound(&ipv4(a.ip, b.ip, b"gone"), now).unwrap();
        assert!(
            b.engine
                .inbound(&t.datagram, Link::Direct(a.addr), now)
                .is_empty()
        );
    }

    #[test]
    fn handshake_from_unknown_node_gets_no_answer() {
        let mut a = node(1);
        let mut b = node(2);
        let now = Instant::now();
        // A 认识 B，B 不认识 A
        a.engine
            .apply(&PeerSet::new([config_for(&b)]).unwrap())
            .unwrap();
        let init = a
            .engine
            .set_path(&b.key, Path::Direct(b.addr), now)
            .unwrap();
        assert!(
            b.engine
                .inbound(&init[0].datagram, Link::Direct(a.addr), now)
                .is_empty()
        );
    }

    #[test]
    fn relay_must_report_the_real_sender() {
        let mut a = node(1);
        let mut b = node(2);
        let now = Instant::now();
        a.engine
            .apply(&PeerSet::new([config_for(&b)]).unwrap())
            .unwrap();
        b.engine
            .apply(&PeerSet::new([config_for(&a)]).unwrap())
            .unwrap();

        let relay = NodeKey::from_bytes([7; 32]);
        let relay_addr = SocketAddr::from(([203, 0, 113, 1], 443));
        let init = a
            .engine
            .set_path(
                &b.key,
                Path::Relay {
                    relay,
                    addr: relay_addr,
                },
                now,
            )
            .unwrap();
        assert_eq!(
            init[0].link,
            Link::Relay {
                relay,
                addr: relay_addr,
                peer: b.key
            }
        );

        // 中继谎报发送方：丢弃
        let liar = Link::Relay {
            relay,
            addr: relay_addr,
            peer: NodeKey::from_bytes([9; 32]),
        };
        assert!(b.engine.inbound(&init[0].datagram, liar, now).is_empty());

        // 如实报告：回应经同一个中继交给 A
        let honest = Link::Relay {
            relay,
            addr: relay_addr,
            peer: a.key,
        };
        let actions = b.engine.inbound(&init[0].datagram, honest, now);
        let response = transmits(&actions);
        assert_eq!(response[0].link, honest);
        assert_eq!(
            events(&actions),
            [Event::HandshakeCompleted {
                peer: a.key,
                via: Path::Relay {
                    relay,
                    addr: relay_addr
                },
            }]
        );
    }

    #[test]
    fn self_as_peer_is_rejected() {
        let mut a = node(1);
        let me = PeerConfig {
            key: a.key,
            allowed_ips: vec![],
            keepalive: None,
        };
        let result = a.engine.apply(&PeerSet::new([me]).unwrap());
        assert!(matches!(result, Err(DataPlaneError::SelfPeer)));
    }

    #[test]
    fn set_path_on_unknown_peer_is_an_error() {
        let mut a = node(1);
        let stranger = NodeKey::from_bytes([5; 32]);
        let result = a
            .engine
            .set_path(&stranger, Path::Direct(a.addr), Instant::now());
        assert!(matches!(result, Err(DataPlaneError::UnknownPeer(k)) if k == stranger));
    }

    #[test]
    fn control_datagrams_are_handed_over_untouched() {
        let mut a = node(1);
        let mut datagram = CONTROL_MAGIC.to_vec();
        datagram.extend_from_slice(b"hello");
        let from = SocketAddr::from(([192, 0, 2, 9], 1234));
        let actions = a
            .engine
            .inbound(&datagram, Link::Direct(from), Instant::now());
        assert_eq!(
            actions,
            [Action::Event(Event::ControlDatagram { from, datagram })]
        );
    }

    #[test]
    fn garbage_and_unroutable_packets_are_dropped() {
        let (mut a, _b) = connected_pair();
        let now = Instant::now();
        let from = SocketAddr::from(([192, 0, 2, 9], 1234));
        assert!(
            a.engine
                .inbound(b"not wireguard", Link::Direct(from), now)
                .is_empty()
        );
        let mut fake_data = vec![0u8; 64];
        fake_data[0] = 4;
        assert!(
            a.engine
                .inbound(&fake_data, Link::Direct(from), now)
                .is_empty()
        );

        // 没有网段覆盖的目的地址
        let packet = ipv4(a.ip, IpAddr::from([10, 9, 9, 9]), b"?");
        assert_eq!(a.engine.outbound(&packet, now), None);
        assert_eq!(a.engine.outbound(b"", now), None);
    }

    #[test]
    fn byte_counters_count_wireguard_datagrams() {
        let (mut a, mut b) = connected_pair();
        let now = Instant::now();
        let before = a.engine.status()[0].tx_bytes;
        let t = a
            .engine
            .outbound(&ipv4(a.ip, b.ip, &[0; 100]), now)
            .unwrap();
        // 120 字节的 IP 报文加上 WireGuard 的 32 字节开销。boringtun 不做规范里的 16 字节填充
        assert_eq!(t.datagram.len(), 120 + 32);
        assert_eq!(
            a.engine.status()[0].tx_bytes - before,
            t.datagram.len() as u64
        );

        let rx_before = b.engine.status()[0].rx_bytes;
        b.engine.inbound(&t.datagram, Link::Direct(a.addr), now);
        assert_eq!(
            b.engine.status()[0].rx_bytes - rx_before,
            t.datagram.len() as u64
        );
    }

    #[test]
    fn persistent_keepalive_is_sent_on_schedule() {
        let (mut a, b) = connected_pair();
        let now = Instant::now();
        let mut config = config_for(&b);
        config.keepalive = NonZeroU16::new(25);
        a.engine.apply(&PeerSet::new([config]).unwrap()).unwrap();

        // 刚发过报文，还不到时候
        let quiet: Vec<_> = a
            .engine
            .timers(now)
            .into_iter()
            .filter(|t| classify(&t.datagram) == DatagramKind::WireGuard(WgMessage::TransportData))
            .collect();
        assert!(quiet.is_empty());

        // 25 秒后该发了：一个空的数据报文，正好 32 字节
        let later = now + Duration::from_secs(25);
        let out = a.engine.timers(later);
        let keepalives: Vec<_> = out.iter().filter(|t| t.datagram.len() == 32).collect();
        assert_eq!(keepalives.len(), 1);
        assert_eq!(keepalives[0].link, Link::Direct(b.addr));
    }
}
