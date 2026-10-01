//! 协调服务：节点注册、密钥分发、端点探测、打洞对时。
//!
//! 它是整套设计里信任最集中的地方 —— 节点靠它知道"某某的公钥是什么"。
//! 被攻陷的后果见[威胁模型](https://kerxs.github.io/meshora/guide/threat-model#恶意或被攻陷的控制面)。
//! 它可以自建。
//!
//! 两种模式，可以同时开：
//!
//! - **单网络**（`--node` / `--state`）：整个协调服务就是一个网络。网络码是 `公钥@地址:端口#邀请码`，
//!   节点的第一条消息是 `Hello`
//! - **hub**（`--hub <数据目录>`）：托管很多网络，谁都能来建一个、自己当网主，见 [`hub`](HubConfig)。
//!   网络码是 `公钥@地址:端口/网络ID#邀请码`，第一条消息是 `Join` 或 `Create`
//!
//! 取舍：
//!
//! - **成员有两种来源**。一是配置里的名单（`--node`），overlay 地址按名单顺序分配：第 n 个节点拿网段里
//!   第 n 个地址，名单顺序不变地址就不变。二是**凭邀请码加入**的：核对无误就分给它最小的空闲地址，
//!   写进状态文件，重启后地址不变。见[状态文件](#状态文件)
//! - **单网络模式的状态文件改了就生效**：协调服务每 [`STATE_POLL`] 看一次它变没变。删掉一个成员，
//!   它马上被断开、从所有人的 NetMap 里消失；换了邀请码，旧的网络码马上作废。
//!   文件改坏了（格式不对、地址冲突）就记一条警告、保持原样。hub 模式由网主在客户端里管，不盯文件
//! - **NetMap 是整个名单**（声明式），带上每个节点最近上报的端点。节点的控制连接断了，
//!   它仍然在网里 —— 控制面抖一下，不该把已经通了的数据面也拆掉
//! - **收到第一条加密消息之前不做任何有副作用的事**：IK 的首个握手包可以被重放（R1）
//!
//! # 状态文件
//!
//! 纯文本，一行一项，`#` 开头的是注释。人可以直接改，改完不用重启：
//!
//! ```text
//! invite 3q2-7wEYkQ6n0Cf8Hs5VYA
//! member mTe0q8vN3kRZp1u5yXcW7bLdF2gH9jK4sA6eQoIiUtY= 100.64.0.3
//! ```
//!
//! - `invite`：邀请码。不带限制的那一行是长期有效的，删掉它会生成一个新的写回去 ——
//!   旧的网络码随之失效，已经加入的成员不受影响
//! - `invite <码> uses=1 expires=2026-10-01T12:00:00Z`：带限制的邀请码，可以有很多行。
//!   `uses` 是还能用几次，`expires` 是 UTC 的过期时间，两个都可以只写一个。用完、过期的
//!   自动从文件里清掉。`meshora-coord invite` 帮你生成这样一行
//! - `member`：凭邀请码加入的成员和它的地址。删掉一行，这个成员就不在网里了
//! - hub 模式的文件还有 `owner`（网主）、`name`（网络名）、`seen`（最近有人在线的时刻）

mod hub;
mod network;
mod state_file;

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

use ipnet::Ipv4Net;
use meshora_proto::admission::Admission;
use meshora_proto::control::{AdminRequest, ClientMessage, RelayInfo, ServerMessage, clean_name};
use meshora_proto::disco::{self, DiscoMessage};
use meshora_proto::noise::{Channel, NoiseStream};
use meshora_types::{Invite, NodeKey, NodeSecret};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{Notify, mpsc};
use tracing::{debug, info, warn};

pub use hub::{HubConfig, HubLimits};
pub use state_file::limited_invite;

use hub::Hub;
use network::{Conn, Kick, Network, State};
use state_file::{assign, load_state, merge_joined};
#[cfg(test)]
use state_file::{format_utc, parse_utc};

/// 握手和第一条消息的时限：慢吞吞的连接不许一直占着。
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// 第一条消息（Hello / Join / Create）的长度上限。这时还不知道对方是谁 ——
/// 谁都能现生成一把密钥完成握手 —— 不能按它自称的长度分配内存
const FIRST_MESSAGE_MAX: usize = 192;
/// 接受连接出错（比如文件描述符用完）之后，等多久再接。
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);
/// 同一个来源同时最多几条连接在握手，见 [`Admission`]。
const PENDING_PER_SOURCE: usize = 16;
/// 多久没收到节点的任何消息就断开。节点每 30 秒发一次保活。
const IDLE_TIMEOUT: Duration = Duration::from_secs(90);
/// 一个节点最多上报多少个端点。端点会被别的节点拿去发探测报文，不设上限就成了放大器。
const MAX_ENDPOINTS: usize = 16;
/// 发往一个连接的消息队列长度。满了说明对方根本不读，丢掉就是。
///
/// NetMap 不走这个队列：它是全量的，只需要把最新的一份送到，见 `Conn::net_map`。
const QUEUE: usize = 64;
/// 给同一个节点发 NetMap（和成员清单）的最小间隔，期间的变化合并成一份。有人频繁上报端点，
/// 别的节点也只是每秒多收一份 NetMap，协调服务不必为每次上报给每个节点都生成一份。
const NET_MAP_INTERVAL: Duration = Duration::from_secs(1);
/// 探测端点每秒最多回应多少条。
const PROBE_RATE: u32 = 200;
/// 单网络模式下网里最多多少个成员（名单里的加上凭邀请码加入的）。NetMap 要发给每个人，
/// 成员数没有上限的话，一份邀请码泄漏出去就能把协调服务拖垮。hub 模式另有更小的限额
pub const MAX_MEMBERS: usize = 1024;
/// 多久看一次状态文件变没变。
pub const STATE_POLL: Duration = Duration::from_secs(2);
/// 被移出的成员：给它的连接多少时间把"你被移出了"送出去，然后断开
const KICK_FLUSH: Duration = Duration::from_secs(1);
/// hub 模式多久清一次闲置的网络
const CLEANUP_INTERVAL: Duration = Duration::from_secs(3600);

/// 协调服务的配置。
pub struct Config {
    /// 协调服务自己的身份。节点事先知道它的公钥，靠它认证协调服务。
    pub secret: NodeSecret,
    /// 单网络模式：名单里的节点。overlay 地址按这个顺序分配。
    pub nodes: Vec<NodeKey>,
    /// 单网络模式：状态文件，邀请码和凭它加入的成员存在这里。`None` 表示不接受邀请加入，只认名单。
    pub state: Option<PathBuf>,
    /// overlay 网段，比如 `100.64.0.0/10`。hub 模式下每个网络各用一份。
    pub overlay: Ipv4Net,
    /// 告诉节点的端点探测地址：本服务的探测 socket 在公网上的地址。
    pub probe: Option<SocketAddr>,
    /// 告诉节点的中继。
    pub relays: Vec<RelayInfo>,
    /// hub 模式。和单网络模式可以同时开。
    pub hub: Option<HubConfig>,
}

struct Shared {
    secret: NodeSecret,
    probe: Option<SocketAddr>,
    relays: Vec<RelayInfo>,
    /// 单网络模式的那个网络
    legacy: Option<Arc<Network>>,
    hub: Option<Mutex<Hub>>,
    next_conn: AtomicU64,
    admission: Arc<Admission>,
}

impl Shared {
    fn hub(&self) -> Option<MutexGuard<'_, Hub>> {
        self.hub
            .as_ref()
            .map(|hub| hub.lock().expect("网络表在持锁时 panic 过"))
    }

    /// 所有网络。先把列表取出来再一个个看：不在拿着网络表的锁时去拿网络的锁以外的东西
    fn networks(&self) -> Vec<Arc<Network>> {
        let mut all: Vec<Arc<Network>> = self.legacy.iter().cloned().collect();
        if let Some(hub) = self.hub() {
            all.extend(hub.networks().cloned());
        }
        all
    }

    /// 是不是任何一个网络的成员：探测端点、中继用它
    fn is_member(&self, key: &NodeKey) -> bool {
        self.networks().iter().any(|network| network.is_member(key))
    }

    /// 两个节点在不在同一个网络里：中继只在同一个网络的成员之间转发
    fn same_network(&self, a: &NodeKey, b: &NodeKey) -> bool {
        self.networks().iter().any(|network| {
            let state = network.state();
            state.address_of(a).is_some() && state.address_of(b).is_some()
        })
    }
}

/// 一个准备好的协调服务：成员表、邀请码都已经就位，还没开始接受连接。
pub struct Coordinator {
    shared: Arc<Shared>,
}

impl Coordinator {
    /// 按配置建好成员表。单网络模式有状态文件时读它，里面还没有邀请码就生成一个写回去；
    /// hub 模式读数据目录里已有的网络。
    pub fn new(config: Config) -> io::Result<Self> {
        let invalid = |message: String| io::Error::new(io::ErrorKind::InvalidInput, message);
        let legacy = if !config.nodes.is_empty() || config.state.is_some() {
            Some(Arc::new(legacy_network(&config)?))
        } else {
            None
        };
        let hub = match config.hub {
            Some(hub) => Some(Mutex::new(Hub::load(hub, config.overlay)?)),
            None => None,
        };
        if legacy.is_none() && hub.is_none() {
            return Err(invalid(
                "既没有名单也没有状态文件，也没开 hub 模式：谁都加入不了".to_string(),
            ));
        }
        Ok(Self {
            shared: Arc::new(Shared {
                secret: config.secret,
                probe: config.probe,
                relays: config.relays,
                legacy,
                hub,
                next_conn: AtomicU64::new(0),
                admission: Admission::new(PENDING_PER_SOURCE),
            }),
        })
    }

    /// 单网络模式的邀请码。没有状态文件（或者只开了 hub 模式）时是 `None`。
    pub fn invite(&self) -> Option<Invite> {
        self.shared.legacy.as_ref().and_then(|n| n.state().invite)
    }

    /// 判断一把公钥是不是成员（任何一个网络的）。同一个进程里的中继用它：
    /// 凭邀请码新加入的成员也马上能用中继。
    pub fn members(&self) -> meshora_relay::Allow {
        let shared = Arc::clone(&self.shared);
        Arc::new(move |key| shared.is_member(key))
    }

    /// 判断两个节点在不在同一个网络里。同一个进程里的中继用它：hub 模式下，
    /// 一个网络的成员不能经中继给别的网络的成员发东西。
    pub fn links(&self) -> meshora_relay::Links {
        let shared = Arc::clone(&self.shared);
        Arc::new(move |a, b| shared.same_network(a, b))
    }

    /// 运行，直到监听的 socket 出错。
    ///
    /// `probe` 是端点探测用的 UDP socket；[`Config::probe`] 是它在公网上的地址（告诉节点往哪发）。
    pub async fn serve(self, listener: TcpListener, probe: Option<UdpSocket>) -> io::Result<()> {
        run(self.shared, listener, probe).await
    }
}

/// 单网络模式的网络：名单里的成员，加上状态文件里凭邀请码加入的
fn legacy_network(config: &Config) -> io::Result<Network> {
    let invalid = |message: String| io::Error::new(io::ErrorKind::InvalidInput, message);
    let mut members = assign(&config.nodes, config.overlay)?;
    let mut invite = None;
    let mut limited = Vec::new();
    if let Some(path) = &config.state {
        let stored = load_state(path)?;
        members = merge_joined(members, stored.joined, config.overlay).map_err(invalid)?;
        invite = Some(stored.invite.unwrap_or_else(Invite::generate));
        limited = stored.limited;
    }
    if members.len() > MAX_MEMBERS {
        return Err(invalid(format!("成员超过了上限 {MAX_MEMBERS}")));
    }
    let network = Network::new(
        None,
        config.overlay,
        MAX_MEMBERS,
        config.state.clone(),
        State {
            members,
            invite,
            limited,
            online: HashMap::new(),
            endpoints: HashMap::new(),
            names: HashMap::new(),
            owner: None,
            name: String::new(),
            seen: SystemTime::now(),
            deleted: false,
        },
    );
    // 写回去：新生成的邀请码要存下来；已经写进名单的成员、过期的邀请码也顺带清掉
    network.save(&network.state())?;
    Ok(network)
}

/// 按配置建好协调服务并运行，直到监听的 socket 出错。见 [`Coordinator`]。
pub async fn serve(
    config: Config,
    listener: TcpListener,
    probe: Option<UdpSocket>,
) -> io::Result<()> {
    Coordinator::new(config)?.serve(listener, probe).await
}

async fn run(
    shared: Arc<Shared>,
    listener: TcpListener,
    probe: Option<UdpSocket>,
) -> io::Result<()> {
    // 先取出来再打日志：同一条语句里两次加锁会在同一个线程上卡死
    let (nodes, invite) = shared.legacy.as_ref().map_or((0, false), |legacy| {
        let state = legacy.state();
        (state.members.len(), state.invite.is_some())
    });
    let networks = shared.hub().map(|hub| hub.len());
    info!(
        key = %shared.secret.public_key(),
        nodes,
        invite,
        hub = ?networks,
        "协调服务启动"
    );

    if let Some(socket) = probe {
        tokio::spawn(probe_loop(Arc::clone(&shared), socket));
    }
    if let Some(legacy) = &shared.legacy
        && legacy.state_file.is_some()
    {
        tokio::spawn(watch_state(Arc::clone(legacy)));
    }
    if shared.hub.is_some() {
        tokio::spawn(cleanup_loop(Arc::clone(&shared)));
    }

    loop {
        match listener.accept().await {
            Ok((tcp, from)) => {
                tokio::spawn(handle(Arc::clone(&shared), tcp, from));
            }
            // 这类错误是暂时的（文件描述符用完之类），不能因此退出 ——
            // 否则谁都能靠开一大堆连接把协调服务打挂。稍等再接，免得空转
            Err(err) => {
                warn!(%err, "接受连接失败");
                tokio::time::sleep(ACCEPT_BACKOFF).await;
            }
        }
    }
}

/// 第一条消息说了要进哪个网络：找到它（或者新建一个），核对能不能进
fn enter(
    shared: &Shared,
    key: &NodeKey,
    from: IpAddr,
    first: ClientMessage,
) -> Result<(Arc<Network>, std::net::Ipv4Addr, Option<ServerMessage>), String> {
    let no_hub = || "这台协调服务不托管多个网络：用不带网络 ID 的网络码".to_string();
    match first {
        ClientMessage::Hello { invite } => {
            let network = shared.legacy.clone().ok_or_else(|| {
                "这台协调服务托管多个网络：网络码里要带网络 ID（公钥@地址:端口/网络ID#邀请码）"
                    .to_string()
            })?;
            let ip = network.admit(key, invite)?;
            Ok((network, ip, None))
        }
        ClientMessage::Join { network, invite } => {
            let found = shared
                .hub()
                .ok_or_else(no_hub)?
                .get(&network)
                .ok_or("找不到这个网络：可能已经被网主解散了")?;
            let ip = found.admit(key, invite)?;
            Ok((found, ip, None))
        }
        ClientMessage::Create { name } => {
            let (network, invite) = shared.hub().ok_or_else(no_hub)?.create(*key, &name, from)?;
            let ip = network.admit(key, None)?;
            let id = network.id.expect("hub 里的网络都有 ID");
            Ok((
                network,
                ip,
                Some(ServerMessage::Created {
                    network: id,
                    invite,
                }),
            ))
        }
        _ => Err("握手后的第一条消息应该是 Hello、Join 或 Create".into()),
    }
}

async fn handle(shared: Arc<Shared>, tcp: TcpStream, from: SocketAddr) {
    // 名额一直占到确认对方能进、打过招呼为止
    let Some(pending) = shared.admission.admit(from.ip()) else {
        debug!(%from, "这个来源同时在握手的连接太多，断开");
        return;
    };
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
    let first =
        match tokio::time::timeout(HANDSHAKE_TIMEOUT, reader.recv_at_most(FIRST_MESSAGE_MAX)).await
        {
            Ok(Ok(bytes)) => ClientMessage::decode(&bytes),
            _ => {
                debug!(%from, node = %key, "握手后没等到第一条消息");
                return;
            }
        };
    let Ok(first) = first else {
        debug!(%from, node = %key, "握手后的第一条消息解不开");
        return;
    };
    let (network, overlay_ip, created) = match enter(&shared, &key, from.ip(), first) {
        Ok(entered) => entered,
        Err(reason) => {
            info!(%from, node = %key, %reason, "拒绝节点");
            let _ = writer
                .send(&ServerMessage::Rejected { reason }.encode())
                .await;
            return;
        }
    };

    drop(pending);

    let (tx, mut rx) = mpsc::channel(QUEUE);
    let net_map = Arc::new(Notify::new());
    let roster = Arc::new(Notify::new());
    let kick = Arc::new(Kick::new());
    let id = shared.next_conn.fetch_add(1, Ordering::Relaxed);
    let deleted = network.state().deleted;
    if deleted {
        let reason = "这个网络已经被网主解散了".to_string();
        let _ = writer
            .send(&ServerMessage::Rejected { reason }.encode())
            .await;
        return;
    }
    {
        let mut state = network.state();
        if let Some(created) = created {
            let _ = tx.try_send(created);
        }
        let welcome = ServerMessage::Welcome {
            overlay_ip,
            prefix_len: network.overlay.prefix_len(),
            probe: shared.probe,
            relays: shared.relays.clone(),
        };
        let _ = tx.try_send(welcome);
        // 第一份 NetMap 紧跟在 Welcome 后面：写任务优先发队列里的
        net_map.notify_one();
        if state.owner == Some(key) {
            roster.notify_one();
        }
        state.online.insert(
            key,
            Conn {
                id,
                tx: tx.clone(),
                net_map: Arc::clone(&net_map),
                roster: Arc::clone(&roster),
                kick: Arc::clone(&kick),
            },
        );
        state.seen = SystemTime::now();
        network.roster_changed(&state);
    }
    info!(%from, node = %key, network = ?network.id, ip = %overlay_ip, "节点上线");

    let writer_network = Arc::clone(&network);
    let writer_task = tokio::spawn(async move {
        let (mut map_pending, mut roster_pending) = (false, false);
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
                () = net_map.notified(), if !map_pending => map_pending = true,
                () = roster.notified(), if !roster_pending => roster_pending = true,
                () = tokio::time::sleep_until(earliest), if map_pending || roster_pending => {
                    let messages: Vec<ServerMessage> = {
                        let state = writer_network.state();
                        let mut messages = Vec::new();
                        if map_pending {
                            messages.push(writer_network.net_map_for(&state, &key));
                        }
                        if roster_pending && state.owner == Some(key) {
                            messages.push(writer_network.roster(&state));
                        }
                        messages
                    };
                    for message in messages {
                        if writer.send(&message.encode()).await.is_err() {
                            return;
                        }
                    }
                    (map_pending, roster_pending) = (false, false);
                    earliest = tokio::time::Instant::now() + NET_MAP_INTERVAL;
                }
            }
        }
    });

    loop {
        let received = tokio::select! {
            received = tokio::time::timeout(IDLE_TIMEOUT, reader.recv()) => received,
            reason = kick.kicked() => {
                // 被移出了、网络解散了：告诉它一声再断开。写任务发完队列里的就会自己结束
                let _ = tx.try_send(ServerMessage::Rejected { reason });
                drop(tx);
                if tokio::time::timeout(KICK_FLUSH, writer_task).await.is_err() {
                    debug!(node = %key, "被移出的节点迟迟不收，直接断开");
                }
                info!(node = %key, "节点被移出，已断开");
                return;
            }
        };
        let bytes = match received {
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
                let state = &mut *network.state();
                if state.endpoints.get(&key) != Some(&endpoints) {
                    state.endpoints.insert(key, endpoints);
                    network.changed(state, Some(&key));
                }
            }
            Ok(ClientMessage::CallMeMaybe { peer }) => {
                let state = network.state();
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
            Ok(ClientMessage::SetName(name)) => {
                let name = clean_name(&name);
                let state = &mut *network.state();
                let current = state.names.get(&key).map_or("", String::as_str);
                if current != name {
                    if name.is_empty() {
                        state.names.remove(&key);
                    } else {
                        state.names.insert(key, name);
                    }
                    network.changed(state, Some(&key));
                }
            }
            Ok(ClientMessage::Admin(request)) => {
                let delete = matches!(request, AdminRequest::Delete);
                let result = network.admin(&key, request);
                if delete
                    && result.is_ok()
                    && let (Some(id), Some(mut hub)) = (network.id, shared.hub())
                {
                    hub.remove(&id);
                }
                let _ = tx.try_send(ServerMessage::AdminReply(result));
            }
            Ok(
                ClientMessage::Hello { .. }
                | ClientMessage::Join { .. }
                | ClientMessage::Create { .. },
            ) => {}
            Err(err) => {
                debug!(node = %key, %err, "消息解不开，断开");
                break;
            }
        }
    }

    {
        let mut state = network.state();
        // 同一个节点可能已经重连上来了：只移除属于这条连接的登记
        if state.online.get(&key).is_some_and(|conn| conn.id == id) {
            state.online.remove(&key);
            network.touch(&mut state);
            network.roster_changed(&state);
        }
    }
    writer_task.abort();
    info!(node = %key, "节点下线");
}

/// 能不能当端点。别的节点会往端点发探测报文，一眼就知道不对的不收：
/// 未指定地址（Linux 上发往 0.0.0.0 就是发给本机）、组播、广播、端口 0
fn usable_endpoint(addr: &SocketAddr) -> bool {
    let broadcast = matches!(addr.ip(), IpAddr::V4(ip) if ip.is_broadcast());
    addr.port() != 0 && !addr.ip().is_unspecified() && !addr.ip().is_multicast() && !broadcast
}

/// 盯着单网络模式的状态文件：和上次读写之后的样子不一样了，就重新读
async fn watch_state(network: Arc<Network>) {
    loop {
        tokio::time::sleep(STATE_POLL).await;
        network.reload_if_changed();
    }
}

/// hub 模式：太久没人上线的网络删掉
async fn cleanup_loop(shared: Arc<Shared>) {
    loop {
        tokio::time::sleep(CLEANUP_INTERVAL).await;
        let idle = match shared.hub() {
            Some(hub) => hub.idle(SystemTime::now()),
            None => return,
        };
        for network in idle {
            network.delete();
            if let (Some(id), Some(mut hub)) = (network.id, shared.hub()) {
                hub.remove(&id);
                info!(network = %id, "网络太久没人上线，删掉了");
            }
        }
    }
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
