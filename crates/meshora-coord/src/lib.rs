//! 协调服务：节点注册、密钥分发、端点探测、打洞对时。
//!
//! 它是整套设计里信任最集中的地方 —— 节点靠它知道"某某的公钥是什么"。
//! 被攻陷的后果见[威胁模型](https://kerxs.github.io/meshora/guide/threat-model#恶意或被攻陷的控制面)。
//! 它可以自建。
//!
//! M1 的取舍：
//!
//! - **节点名单就是白名单**，写在配置里。overlay 地址按名单顺序分配：第 n 个节点拿网段里
//!   第 n 个地址。名单顺序不变，协调服务重启后地址也不变，不需要持久化
//! - **NetMap 是整个名单**（声明式），带上每个节点最近上报的端点。节点的控制连接断了，
//!   它仍然在网里 —— 控制面抖一下，不该把已经通了的数据面也拆掉
//! - **收到第一条加密消息（Hello）之前不做任何有副作用的事**：IK 的首个握手包可以被重放（R1）

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use ipnet::Ipv4Net;
use meshora_proto::control::{ClientMessage, PeerInfo, RelayInfo, ServerMessage};
use meshora_proto::disco::{self, DiscoMessage};
use meshora_proto::noise::{Channel, NoiseStream};
use meshora_types::{NodeKey, NodeSecret};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{Notify, mpsc};
use tracing::{debug, info, warn};

/// 握手和第一条消息的时限：慢吞吞的连接不许一直占着。
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// 多久没收到节点的任何消息就断开。节点每 30 秒发一次保活。
const IDLE_TIMEOUT: Duration = Duration::from_secs(90);
/// 一个节点最多上报多少个端点。端点会被别的节点拿去发探测报文，不设上限就成了放大器。
const MAX_ENDPOINTS: usize = 16;
/// 发往一个连接的消息队列长度。满了说明对方根本不读，丢掉就是。
///
/// NetMap 不走这个队列：它是全量的，只需要把最新的一份送到，见 [`Conn::net_map`]。
const QUEUE: usize = 64;
/// 给同一个节点发 NetMap 的最小间隔，期间的变化合并成一份。有人频繁上报端点，
/// 别的节点也只是每秒多收一份 NetMap，协调服务不必为每次上报给每个节点都生成一份。
const NET_MAP_INTERVAL: Duration = Duration::from_secs(1);
/// 探测端点每秒最多回应多少条。
const PROBE_RATE: u32 = 200;

/// 协调服务的配置。
pub struct Config {
    /// 协调服务自己的身份。节点事先知道它的公钥，靠它认证协调服务。
    pub secret: NodeSecret,
    /// 允许加入的节点。overlay 地址按这个顺序分配。
    pub nodes: Vec<NodeKey>,
    /// overlay 网段，比如 `100.64.0.0/10`。
    pub overlay: Ipv4Net,
    /// 告诉节点的端点探测地址：本服务的探测 socket 在公网上的地址。
    pub probe: Option<SocketAddr>,
    /// 告诉节点的中继。
    pub relays: Vec<RelayInfo>,
}

struct Conn {
    id: u64,
    /// Welcome、转发的打洞请求、Pong
    tx: mpsc::Sender<ServerMessage>,
    /// "有新的 NetMap 了"。连接的写任务被唤醒时按当时的状态现生成一份，所以多次变化
    /// 自然合并，最新的一份也绝不会丢 —— 此前用队列发，队列满了就跳过，
    /// 跳过的偏偏是最后一份的话，这个节点就一直拿着过时的 NetMap
    net_map: Arc<Notify>,
}

#[derive(Default)]
struct State {
    online: HashMap<NodeKey, Conn>,
    endpoints: HashMap<NodeKey, Vec<SocketAddr>>,
}

struct Shared {
    secret: NodeSecret,
    /// 白名单，同时是地址表。顺序就是分配顺序
    members: Vec<(NodeKey, Ipv4Addr)>,
    overlay: Ipv4Net,
    probe: Option<SocketAddr>,
    relays: Vec<RelayInfo>,
    state: Mutex<State>,
    next_conn: AtomicU64,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("协调服务的状态在持锁时 panic 过")
    }

    fn address_of(&self, key: &NodeKey) -> Option<Ipv4Addr> {
        self.members
            .iter()
            .find(|(member, _)| member == key)
            .map(|(_, ip)| *ip)
    }

    fn is_member(&self, key: &NodeKey) -> bool {
        self.address_of(key).is_some()
    }

    /// 发给 `recipient` 的 NetMap：除它自己以外的所有成员
    fn net_map_for(&self, state: &State, recipient: &NodeKey) -> ServerMessage {
        let peers = self
            .members
            .iter()
            .filter(|(key, _)| key != recipient)
            .map(|(key, ip)| PeerInfo {
                key: *key,
                overlay_ip: *ip,
                endpoints: state.endpoints.get(key).cloned().unwrap_or_default(),
            })
            .collect();
        ServerMessage::NetMap { peers }
    }

    /// 网里有变化：通知每个在线节点的连接，发一份新的 NetMap
    fn broadcast_net_maps(&self, state: &State, except: Option<&NodeKey>) {
        for (key, conn) in &state.online {
            if Some(key) != except {
                conn.net_map.notify_one();
            }
        }
    }
}

/// 能不能当端点。别的节点会往端点发探测报文，一眼就知道不对的不收：
/// 未指定地址（Linux 上发往 0.0.0.0 就是发给本机）、组播、广播、端口 0
fn usable_endpoint(addr: &SocketAddr) -> bool {
    let broadcast = matches!(addr.ip(), IpAddr::V4(ip) if ip.is_broadcast());
    addr.port() != 0 && !addr.ip().is_unspecified() && !addr.ip().is_multicast() && !broadcast
}

/// 按名单分配地址：第 n 个节点拿网段里第 n 个可用地址（从 .1 开始）
fn assign(nodes: &[NodeKey], overlay: Ipv4Net) -> io::Result<Vec<(NodeKey, Ipv4Addr)>> {
    let invalid = |message: String| io::Error::new(io::ErrorKind::InvalidInput, message);
    let mut hosts = overlay.hosts();
    let mut members: Vec<(NodeKey, Ipv4Addr)> = Vec::with_capacity(nodes.len());
    for key in nodes {
        if members.iter().any(|(member, _)| member == key) {
            return Err(invalid(format!("节点 {key} 在名单里出现了两次")));
        }
        let ip = hosts.next().ok_or_else(|| {
            invalid(format!(
                "overlay 网段 {overlay} 装不下 {} 个节点",
                nodes.len()
            ))
        })?;
        members.push((*key, ip));
    }
    Ok(members)
}

/// 运行协调服务，直到监听的 socket 出错。
///
/// `probe` 是端点探测用的 UDP socket；`config.probe` 是它在公网上的地址（告诉节点往哪发）。
pub async fn serve(
    config: Config,
    listener: TcpListener,
    probe: Option<UdpSocket>,
) -> io::Result<()> {
    let members = assign(&config.nodes, config.overlay)?;
    let shared = Arc::new(Shared {
        secret: config.secret,
        members,
        overlay: config.overlay,
        probe: config.probe,
        relays: config.relays,
        state: Mutex::new(State::default()),
        next_conn: AtomicU64::new(0),
    });
    info!(
        key = %shared.secret.public_key(),
        nodes = shared.members.len(),
        "协调服务启动"
    );

    if let Some(socket) = probe {
        tokio::spawn(probe_loop(Arc::clone(&shared), socket));
    }

    loop {
        let (tcp, from) = listener.accept().await?;
        tokio::spawn(handle(Arc::clone(&shared), tcp, from));
    }
}

async fn handle(shared: Arc<Shared>, tcp: TcpStream, from: SocketAddr) {
    let _ = tcp.set_nodelay(true);
    let stream = match tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        NoiseStream::accept(tcp, Channel::Control, &shared.secret),
    )
    .await
    {
        Ok(Ok(stream)) => stream,
        Ok(Err(err)) => {
            debug!(%from, %err, "握手失败");
            return;
        }
        Err(_) => {
            debug!(%from, "握手超时");
            return;
        }
    };
    let key = stream.remote();
    let (mut reader, mut writer) = stream.into_split();

    // R1：第一条加密消息到了，才说明对面真在线，而不是一个被重放的握手包
    let hello = match tokio::time::timeout(HANDSHAKE_TIMEOUT, reader.recv()).await {
        Ok(Ok(bytes)) => ClientMessage::decode(&bytes),
        _ => {
            debug!(%from, node = %key, "握手后没等到第一条消息");
            return;
        }
    };
    if hello != Ok(ClientMessage::Hello) {
        debug!(%from, node = %key, "握手后第一条不是 Hello");
        return;
    }
    let Some(overlay_ip) = shared.address_of(&key) else {
        info!(%from, node = %key, "拒绝不在名单里的节点");
        let reason = "这把公钥不在协调服务的节点名单里".to_string();
        let _ = writer
            .send(&ServerMessage::Rejected { reason }.encode())
            .await;
        return;
    };

    let (tx, mut rx) = mpsc::channel(QUEUE);
    let net_map = Arc::new(Notify::new());
    let id = shared.next_conn.fetch_add(1, Ordering::Relaxed);
    {
        let mut state = shared.state();
        let welcome = ServerMessage::Welcome {
            overlay_ip,
            prefix_len: shared.overlay.prefix_len(),
            probe: shared.probe,
            relays: shared.relays.clone(),
        };
        let _ = tx.try_send(welcome);
        // 第一份 NetMap 紧跟在 Welcome 后面：写任务优先发队列里的
        net_map.notify_one();
        state.online.insert(
            key,
            Conn {
                id,
                tx: tx.clone(),
                net_map: Arc::clone(&net_map),
            },
        );
    }
    info!(%from, node = %key, ip = %overlay_ip, "节点上线");

    let writer_shared = Arc::clone(&shared);
    let writer_task = tokio::spawn(async move {
        let mut pending = false;
        let mut earliest = tokio::time::Instant::now();
        loop {
            tokio::select! {
                biased;
                message = rx.recv() => {
                    let Some(message) = message else { break };
                    if writer.send(&message.encode()).await.is_err() {
                        break;
                    }
                }
                () = net_map.notified(), if !pending => pending = true,
                () = tokio::time::sleep_until(earliest), if pending => {
                    let message = writer_shared.net_map_for(&writer_shared.state(), &key);
                    if writer.send(&message.encode()).await.is_err() {
                        break;
                    }
                    pending = false;
                    earliest = tokio::time::Instant::now() + NET_MAP_INTERVAL;
                }
            }
        }
    });

    loop {
        let bytes = match tokio::time::timeout(IDLE_TIMEOUT, reader.recv()).await {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(err)) => {
                debug!(node = %key, %err, "连接断开");
                break;
            }
            Err(_) => {
                debug!(node = %key, "太久没有消息，断开");
                break;
            }
        };
        match ClientMessage::decode(&bytes) {
            Ok(ClientMessage::Endpoints(mut endpoints)) => {
                endpoints.retain(usable_endpoint);
                endpoints.truncate(MAX_ENDPOINTS);
                let state = &mut *shared.state();
                if state.endpoints.get(&key) != Some(&endpoints) {
                    state.endpoints.insert(key, endpoints);
                    shared.broadcast_net_maps(state, Some(&key));
                }
            }
            Ok(ClientMessage::CallMeMaybe { peer }) => {
                let state = shared.state();
                if let Some(conn) = state.online.get(&peer) {
                    let endpoints = state.endpoints.get(&key).cloned().unwrap_or_default();
                    let _ = conn.tx.try_send(ServerMessage::CallMeMaybe {
                        peer: key,
                        endpoints,
                    });
                }
            }
            Ok(ClientMessage::Ping) => {
                let _ = tx.try_send(ServerMessage::Pong);
            }
            Ok(ClientMessage::Hello) => {}
            Err(err) => {
                debug!(node = %key, %err, "消息解不开，断开");
                break;
            }
        }
    }

    {
        let mut state = shared.state();
        // 同一个节点可能已经重连上来了：只移除属于这条连接的登记
        if state.online.get(&key).is_some_and(|conn| conn.id == id) {
            state.online.remove(&key);
        }
    }
    writer_task.abort();
    info!(node = %key, "节点下线");
}

/// 简单的令牌桶：每秒补满
struct TokenBucket {
    tokens: u32,
    refilled: Instant,
}

impl TokenBucket {
    fn take(&mut self) -> bool {
        if self.refilled.elapsed() >= Duration::from_secs(1) {
            self.tokens = PROBE_RATE;
            self.refilled = Instant::now();
        }
        if self.tokens == 0 {
            return false;
        }
        self.tokens -= 1;
        true
    }
}

/// 端点探测：节点发来 Ping，回一个 Pong，带上"看到你从哪来"
async fn probe_loop(shared: Arc<Shared>, socket: UdpSocket) {
    let mut buf = vec![0u8; 2048];
    let mut bucket = TokenBucket {
        tokens: PROBE_RATE,
        refilled: Instant::now(),
    };
    loop {
        let (len, from) = match socket.recv_from(&mut buf).await {
            Ok(received) => received,
            Err(err) => {
                debug!(%err, "探测 socket 接收出错，继续");
                continue;
            }
        };
        // 限速放在任何密码学运算之前（R3）
        if !bucket.take() {
            continue;
        }
        let opened = disco::open(&shared.secret, &buf[..len], |key| shared.is_member(key));
        let Ok((sender, DiscoMessage::Ping { tx })) = opened else {
            continue;
        };
        let reply = disco::seal(
            &shared.secret,
            &sender,
            &DiscoMessage::Pong { tx, observed: from },
        );
        if let Err(err) = socket.send_to(&reply, from).await {
            warn!(%from, %err, "探测回应发送失败");
        }
    }
}

#[cfg(test)]
mod tests;
