//! 节点侧控制面：注册、发现、端点探测、打洞、链路探测、选路。
//!
//! 只依赖契约（meshora-dataplane），不依赖数据面的实现：数据面以 `Arc<dyn DataPlane>`
//! 交进来，事件从一个 channel 收。
//!
//! 一个节点的控制面做这些事：
//!
//! 1. 连上协调服务（Noise IK），拿到 overlay 地址和中继（[`Session::connect`]）
//! 2. 收到 NetMap 就把 peer 集合整个交给数据面（`apply`）
//! 3. **中继先行**：peer 一出现就让数据面走中继，同时在后台探测直连
//! 4. 端点探测：向协调服务的探测端点发 Ping，回来的 Pong 里是本机在公网上的地址
//! 5. 打洞：向 peer 的每个候选端点发 Ping，同时请协调服务转告对方也向我们发（CallMeMaybe）
//! 6. 选路：哪条直连回了 Pong 就切过去；直连不通了切回中继（选路规则见 [`paths`]）
//!
//! 控制连接断了会自己重连。断开期间数据面保持原样 —— 控制面抖一下，不该把已经通了的连接拆掉。

pub mod paths;

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::num::NonZeroU16;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ipnet::IpNet;
use meshora_dataplane::{DataPlane, Event, PeerConfig, PeerSet};
use meshora_proto::codec::DecodeError;
use meshora_proto::control::{ClientMessage, PeerInfo, RelayInfo, ServerMessage};
use meshora_proto::disco::{self, DiscoMessage, TxId};
use meshora_proto::noise::{Channel, NoiseError, NoiseStream, NoiseWriter};
use meshora_types::{NodeKey, NodeSecret, Path};
use rand_core::{OsRng, RngCore};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::paths::PeerPaths;

/// 控制面的节拍。
const TICK: Duration = Duration::from_secs(1);
/// 连接和握手的时限。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 多久向协调服务发一次保活。协调服务 90 秒收不到任何消息就断开。
const CONTROL_PING_INTERVAL: Duration = Duration::from_secs(30);
/// 多久重新探测一次本机的公网地址。
const PROBE_INTERVAL: Duration = Duration::from_secs(20);
/// 没有直连时，多久请对方打一次洞。
const CALL_ME_MAYBE_INTERVAL: Duration = Duration::from_secs(10);
/// 发出去的 Ping 多久没回应就不再等。
const PING_TIMEOUT: Duration = Duration::from_secs(10);
/// 重连的等待时间上限。
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// 控制面的配置。
pub struct Config {
    /// 本机私钥。
    pub secret: NodeSecret,
    /// 协调服务的地址。
    pub coord: SocketAddr,
    /// 协调服务的公钥。事先知道它，才能认证协调服务（IK 的 K）。
    pub coord_key: NodeKey,
    /// 数据面 socket 的本地端口，用来拼出本机的局域网端点。
    pub local_port: u16,
    /// 给每个 peer 的 persistent keepalive，NAT 后面的节点靠它维持映射。
    pub keepalive: Option<NonZeroU16>,
}

/// 协调服务在注册时告诉本机的东西。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Welcome {
    /// 本机的 overlay 地址。
    pub overlay_ip: Ipv4Addr,
    /// overlay 网段的前缀长度。
    pub prefix_len: u8,
    /// 端点探测地址。
    pub probe: Option<SocketAddr>,
    /// 可用的中继。
    pub relays: Vec<RelayInfo>,
}

/// 和协调服务的一条连接：读由单独的任务负责（Noise 的读不能在 select 里被中途取消），
/// 读到的消息经 channel 交出来
struct Connection {
    inbox: mpsc::Receiver<ServerMessage>,
    writer: NoiseWriter<TcpStream>,
    reader: JoinHandle<()>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

/// 连上协调服务、完成注册之后的控制面。
pub struct Session {
    config: Config,
    welcome: Welcome,
    connection: Connection,
}

impl Session {
    /// 连上协调服务并注册。本机不在协调服务的名单里时返回 [`ControlError::Rejected`]。
    pub async fn connect(config: Config) -> Result<Self, ControlError> {
        let (welcome, connection) = connect_once(&config).await?;
        info!(ip = %welcome.overlay_ip, "已注册到协调服务");
        Ok(Self {
            config,
            welcome,
            connection,
        })
    }

    /// 协调服务分配的地址等信息。守护进程据此配置虚拟网卡。
    pub fn welcome(&self) -> &Welcome {
        &self.welcome
    }

    /// 运行控制面，直到事件 channel 关闭（数据面停了）或者出现无法恢复的错误。
    pub async fn run(
        self,
        dataplane: Arc<dyn DataPlane>,
        mut events: mpsc::UnboundedReceiver<Event>,
    ) -> Result<(), ControlError> {
        let Session {
            config,
            welcome,
            mut connection,
        } = self;
        let mut node = Node::new(&config, &welcome, dataplane);
        let mut tick = tokio::time::interval(TICK);

        loop {
            // 连着的时候
            loop {
                let now = Instant::now();
                let outgoing = tokio::select! {
                    message = connection.inbox.recv() => match message {
                        Some(message) => node.on_server_message(message, now),
                        None => break,
                    },
                    event = events.recv() => match event {
                        Some(event) => node.on_event(event, now),
                        None => return Ok(()),
                    },
                    _ = tick.tick() => node.on_tick(now),
                };
                for message in outgoing {
                    if let Err(err) = connection.writer.send(&message.encode()).await {
                        debug!(%err, "发往协调服务失败");
                    }
                }
            }

            // 断开了：边重连边照常处理事件和节拍，只是发不了消息给协调服务
            warn!("和协调服务的连接断了，重连中");
            let mut backoff = Duration::from_secs(1);
            connection = loop {
                let (delay, config) = (backoff, &config);
                let reconnect = async move {
                    tokio::time::sleep(delay).await;
                    connect_once(config).await
                };
                tokio::pin!(reconnect);
                let result = loop {
                    let now = Instant::now();
                    tokio::select! {
                        result = &mut reconnect => break result,
                        event = events.recv() => match event {
                            Some(event) => drop(node.on_event(event, now)),
                            None => return Ok(()),
                        },
                        _ = tick.tick() => drop(node.on_tick(now)),
                    }
                };
                match result {
                    Ok((again, connection)) => {
                        if again.overlay_ip != welcome.overlay_ip
                            || again.prefix_len != welcome.prefix_len
                        {
                            return Err(ControlError::AddressChanged);
                        }
                        info!("已重新连上协调服务");
                        node.on_reconnected(&again);
                        break connection;
                    }
                    Err(ControlError::Rejected(reason)) => {
                        return Err(ControlError::Rejected(reason));
                    }
                    Err(err) => {
                        debug!(%err, ?backoff, "重连失败");
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                    }
                }
            };
        }
    }
}

async fn connect_once(config: &Config) -> Result<(Welcome, Connection), ControlError> {
    let timeout = |what| move |_| ControlError::Timeout(what);
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(config.coord))
        .await
        .map_err(timeout("连接协调服务"))??;
    let _ = tcp.set_nodelay(true);
    let stream = tokio::time::timeout(
        CONNECT_TIMEOUT,
        NoiseStream::connect(tcp, Channel::Control, &config.secret, &config.coord_key),
    )
    .await
    .map_err(timeout("和协调服务握手"))??;
    let (mut reader, mut writer) = stream.into_split();
    writer.send(&ClientMessage::Hello.encode()).await?;

    let first = tokio::time::timeout(CONNECT_TIMEOUT, reader.recv())
        .await
        .map_err(timeout("等协调服务的 Welcome"))??;
    let welcome = match ServerMessage::decode(&first)? {
        ServerMessage::Welcome {
            overlay_ip,
            prefix_len,
            probe,
            relays,
        } => Welcome {
            overlay_ip,
            prefix_len,
            probe,
            relays,
        },
        ServerMessage::Rejected { reason } => return Err(ControlError::Rejected(reason)),
        _ => return Err(ControlError::Protocol("协调服务的第一条消息不是 Welcome")),
    };

    let (tx, inbox) = mpsc::channel(64);
    let reader = tokio::spawn(async move {
        loop {
            let message = match reader.recv().await {
                Ok(bytes) => ServerMessage::decode(&bytes),
                Err(err) => {
                    debug!(%err, "协调服务连接的读取结束");
                    return;
                }
            };
            match message {
                Ok(message) => {
                    if tx.send(message).await.is_err() {
                        return;
                    }
                }
                Err(err) => {
                    warn!(%err, "协调服务发来的消息解不开，断开");
                    return;
                }
            }
        }
    });
    Ok((
        welcome,
        Connection {
            inbox,
            writer,
            reader,
        },
    ))
}

struct PeerState {
    paths: PeerPaths,
    /// 已经告诉数据面的路径
    current: Option<Path>,
    last_call_me_maybe: Option<Instant>,
}

/// 发出去、还在等回应的 Ping
struct Pending {
    to: NodeKey,
    addr: SocketAddr,
    sent: Instant,
}

/// 控制面的状态和规则。不直接碰网络：对协调服务要说的话以返回值交出去，
/// 控制报文经数据面的 socket 发（不变量 4）
struct Node {
    secret: NodeSecret,
    coord: SocketAddr,
    coord_key: NodeKey,
    local_port: u16,
    keepalive: Option<NonZeroU16>,
    dataplane: Arc<dyn DataPlane>,
    relay: Option<Path>,
    probe: Option<SocketAddr>,
    peers: HashMap<NodeKey, PeerState>,
    pending: HashMap<TxId, Pending>,
    reflexive: Option<SocketAddr>,
    reported: Option<Vec<SocketAddr>>,
    next_probe: Instant,
    next_control_ping: Instant,
}

impl Node {
    fn new(config: &Config, welcome: &Welcome, dataplane: Arc<dyn DataPlane>) -> Self {
        let now = Instant::now();
        let mut node = Self {
            secret: config.secret.clone(),
            coord: config.coord,
            coord_key: config.coord_key,
            local_port: config.local_port,
            keepalive: config.keepalive,
            dataplane,
            relay: None,
            probe: None,
            peers: HashMap::new(),
            pending: HashMap::new(),
            reflexive: None,
            reported: None,
            next_probe: now,
            next_control_ping: now + CONTROL_PING_INTERVAL,
        };
        node.on_reconnected(welcome);
        node
    }

    fn on_reconnected(&mut self, welcome: &Welcome) {
        // M1 只用第一个中继
        self.relay = welcome.relays.first().map(|relay| Path::Relay {
            relay: relay.key,
            addr: relay.addr,
        });
        self.probe = welcome.probe;
        // 新连接上，协调服务可能不记得本机的端点了：下个节拍重新上报
        self.reported = None;
        self.next_probe = Instant::now();
    }

    fn on_server_message(&mut self, message: ServerMessage, now: Instant) -> Vec<ClientMessage> {
        match message {
            ServerMessage::NetMap { peers } => self.on_net_map(peers, now),
            ServerMessage::CallMeMaybe { peer, endpoints } => {
                if let Some(state) = self.peers.get_mut(&peer) {
                    // 对方此刻正在向我们发包：我们也马上向它发，两边同时凿洞
                    for addr in endpoints {
                        state.paths.learn(addr, now);
                        state.paths.ping_now(addr);
                    }
                    self.ping_due(now);
                }
                vec![]
            }
            ServerMessage::Pong => vec![],
            ServerMessage::Welcome { .. } | ServerMessage::Rejected { .. } => {
                debug!("注册之后又收到了 Welcome 或 Rejected，忽略");
                vec![]
            }
        }
    }

    fn on_net_map(&mut self, peers: Vec<PeerInfo>, now: Instant) -> Vec<ClientMessage> {
        let configs = peers.iter().map(|peer| PeerConfig {
            key: peer.key,
            allowed_ips: vec![IpNet::from(IpAddr::V4(peer.overlay_ip))],
            keepalive: self.keepalive,
        });
        let set = match PeerSet::new(configs) {
            Ok(set) => set,
            Err(err) => {
                warn!(%err, "协调服务发来的 NetMap 不合法，忽略");
                return vec![];
            }
        };
        if let Err(err) = self.dataplane.apply(&set) {
            warn!(%err, "数据面拒绝了新的 peer 集合");
            return vec![];
        }

        self.peers.retain(|key, _| set.get(key).is_some());
        for peer in &peers {
            let state = self.peers.entry(peer.key).or_insert_with(|| PeerState {
                paths: PeerPaths::default(),
                current: None,
                last_call_me_maybe: None,
            });
            state.paths.set_advertised(&peer.endpoints);
        }
        self.update_paths(now);
        self.ping_due(now);
        vec![]
    }

    fn on_event(&mut self, event: Event, now: Instant) -> Vec<ClientMessage> {
        match event {
            Event::ControlDatagram { from, datagram } => self.on_disco(from, &datagram, now),
            Event::HandshakeCompleted { peer, via } => {
                debug!(%peer, ?via, "WireGuard 握手完成");
            }
        }
        vec![]
    }

    fn on_disco(&mut self, from: SocketAddr, datagram: &[u8], now: Instant) {
        let coord_key = self.coord_key;
        let peers = &self.peers;
        let opened = disco::open(&self.secret, datagram, |key| {
            *key == coord_key || peers.contains_key(key)
        });
        let (sender, message) = match opened {
            Ok(opened) => opened,
            Err(err) => {
                debug!(%from, %err, "丢弃一条控制报文");
                return;
            }
        };
        match message {
            DiscoMessage::Ping { tx } => {
                let Some(state) = self.peers.get_mut(&sender) else {
                    return;
                };
                let pong = disco::seal(
                    &self.secret,
                    &sender,
                    &DiscoMessage::Pong { tx, observed: from },
                );
                if let Err(err) = self.dataplane.send_control(from, &pong) {
                    debug!(%from, %err, "发 Pong 失败");
                }
                // 对方能从这个地址找到我们，反过来也值得一试
                if state.paths.learn(from, now) {
                    self.ping_due(now);
                }
            }
            DiscoMessage::Pong { tx, observed } => {
                let Some(pending) = self.pending.remove(&tx) else {
                    return;
                };
                if pending.to != sender {
                    return;
                }
                let rtt = now.duration_since(pending.sent);
                if sender == self.coord_key {
                    if self.reflexive != Some(observed) {
                        info!(%observed, "探测到本机的公网端点");
                        self.reflexive = Some(observed);
                    }
                } else if let Some(state) = self.peers.get_mut(&sender) {
                    state.paths.on_pong(pending.addr, rtt, now);
                    self.update_paths(now);
                }
            }
        }
    }

    fn on_tick(&mut self, now: Instant) -> Vec<ClientMessage> {
        let mut outgoing = Vec::new();
        self.pending
            .retain(|_, pending| now.duration_since(pending.sent) < PING_TIMEOUT);

        if let Some(probe) = self.probe
            && now >= self.next_probe
        {
            self.ping(self.coord_key, probe, now);
            self.next_probe = now + PROBE_INTERVAL;
        }

        let endpoints = self.endpoints();
        if self.reported.as_ref() != Some(&endpoints) {
            outgoing.push(ClientMessage::Endpoints(endpoints.clone()));
            self.reported = Some(endpoints);
        }

        self.ping_due(now);
        for (key, state) in &mut self.peers {
            let waited = state
                .last_call_me_maybe
                .is_none_or(|last| now.duration_since(last) >= CALL_ME_MAYBE_INTERVAL);
            if !state.paths.has_fresh(now) && waited {
                outgoing.push(ClientMessage::CallMeMaybe { peer: *key });
                state.last_call_me_maybe = Some(now);
            }
        }
        self.update_paths(now);

        if now >= self.next_control_ping {
            outgoing.push(ClientMessage::Ping);
            self.next_control_ping = now + CONTROL_PING_INTERVAL;
        }
        outgoing
    }

    /// 本机的候选端点：局域网地址，加上探测到的公网地址
    fn endpoints(&self) -> Vec<SocketAddr> {
        let mut endpoints = Vec::new();
        if let Some(local) = local_endpoint(self.coord, self.local_port) {
            endpoints.push(local);
        }
        if let Some(reflexive) = self.reflexive
            && !endpoints.contains(&reflexive)
        {
            endpoints.push(reflexive);
        }
        endpoints
    }

    /// 探测所有到时候的候选
    fn ping_due(&mut self, now: Instant) {
        let due: Vec<(NodeKey, SocketAddr)> = self
            .peers
            .iter_mut()
            .flat_map(|(key, state)| {
                state
                    .paths
                    .due_pings(now)
                    .into_iter()
                    .map(move |addr| (*key, addr))
            })
            .collect();
        for (key, addr) in due {
            self.ping(key, addr, now);
        }
    }

    fn ping(&mut self, to: NodeKey, addr: SocketAddr, now: Instant) {
        let mut tx = TxId::default();
        OsRng.fill_bytes(&mut tx);
        let datagram = disco::seal(&self.secret, &to, &DiscoMessage::Ping { tx });
        if let Err(err) = self.dataplane.send_control(addr, &datagram) {
            debug!(%addr, %err, "发 Ping 失败");
            return;
        }
        self.pending.insert(
            tx,
            Pending {
                to,
                addr,
                sent: now,
            },
        );
    }

    /// 按选路规则算出每个 peer 该走的路，变了就告诉数据面
    fn update_paths(&mut self, now: Instant) {
        for (key, state) in &mut self.peers {
            let Some(desired) = state.paths.choose(state.current, self.relay, now) else {
                continue;
            };
            if state.current == Some(desired) {
                continue;
            }
            match self.dataplane.set_path(key, desired) {
                Ok(()) => {
                    info!(peer = %key, path = ?desired, "切换路径");
                    state.current = Some(desired);
                }
                Err(err) => debug!(peer = %key, %err, "设置路径失败"),
            }
        }
    }
}

/// 本机的局域网端点：对协调服务的地址做一次 UDP connect（不发包），
/// 看系统选了哪个本地地址，再配上数据面 socket 的端口
fn local_endpoint(coord: SocketAddr, port: u16) -> Option<SocketAddr> {
    let bind: SocketAddr = match coord {
        SocketAddr::V4(_) => SocketAddr::from(([0, 0, 0, 0], 0)),
        SocketAddr::V6(_) => SocketAddr::from(([0u16; 8], 0)),
    };
    let socket = UdpSocket::bind(bind).ok()?;
    socket.connect(coord).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified()).then_some(SocketAddr::new(ip, port))
}

/// 控制面出错的原因。
#[derive(Debug)]
pub enum ControlError {
    /// 协调服务拒绝了本机（多半是公钥不在它的名单里）。
    Rejected(String),
    /// 重连后协调服务分配的地址变了。虚拟网卡已经按旧地址配好，只能重启守护进程。
    AddressChanged,
    /// 某一步超时了。
    Timeout(&'static str),
    /// 协调服务说的话不符合协议。
    Protocol(&'static str),
    /// 消息解不开。
    Decode(DecodeError),
    /// 连接出错。
    Noise(NoiseError),
    /// I/O 出错。
    Io(io::Error),
}

impl From<NoiseError> for ControlError {
    fn from(err: NoiseError) -> Self {
        Self::Noise(err)
    }
}

impl From<io::Error> for ControlError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<DecodeError> for ControlError {
    fn from(err: DecodeError) -> Self {
        Self::Decode(err)
    }
}

impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(reason) => write!(f, "协调服务拒绝了本机：{reason}"),
            Self::AddressChanged => f.write_str("重连后协调服务分配的地址变了，需要重启"),
            Self::Timeout(what) => write!(f, "{what}超时"),
            Self::Protocol(what) => f.write_str(what),
            Self::Decode(err) => write!(f, "协调服务的消息解不开：{err}"),
            Self::Noise(err) => write!(f, "和协调服务的连接出错：{err}"),
            Self::Io(err) => write!(f, "I/O 出错：{err}"),
        }
    }
}

impl std::error::Error for ControlError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode(err) => Some(err),
            Self::Noise(err) => Some(err),
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}
