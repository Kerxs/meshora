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

pub mod direct;
pub mod paths;
pub mod stun;

use std::collections::{HashMap, VecDeque};
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
use meshora_types::{Invite, NetworkId, NodeKey, NodeSecret, Path};
use rand_core::{OsRng, RngCore};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::paths::PeerPaths;

pub use meshora_proto::control::{AdminRequest, Roster, RosterInvite, RosterMember};

/// 控制面的节拍。
const TICK: Duration = Duration::from_secs(1);
/// 连接和握手的时限。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 多久向协调服务发一次保活。协调服务 90 秒收不到任何消息就断开。
const CONTROL_PING_INTERVAL: Duration = Duration::from_secs(10);
/// 多久听不到协调服务的任何消息，就认为连接已经死了，重连。协调服务对每个保活都回 Pong，
/// 连接正常时每隔 [`CONTROL_PING_INTERVAL`] 至少听到一次。
///
/// 不能指望 TCP 自己发现：比如本机换了网络，连接绑着的旧地址已经不在了，发出去的东西
/// 哪儿也到不了，却不报错 —— TCP 要十几分钟才放弃
const COORD_SILENCE: Duration = Duration::from_secs(25);
/// 多久重新探测一次本机的公网地址。
const PROBE_INTERVAL: Duration = Duration::from_secs(20);
/// 没有直连时，多久请对方打一次洞。
const CALL_ME_MAYBE_INTERVAL: Duration = Duration::from_secs(10);
/// 发出去的 Ping 多久没回应就不再等。
const PING_TIMEOUT: Duration = Duration::from_secs(10);
/// 重连的等待时间上限。
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// 每秒最多处理多少条控制报文。每条要做两次 DH，被人灌满时必须能丢（R3）
const DISCO_RATE: u32 = 200;
/// 网主的管理请求最多等多久回音
const ADMIN_TIMEOUT: Duration = Duration::from_secs(10);
/// 一轮 STUN 最多等多久：有的服务器问不通（被墙、被封 UDP），不能一直等它
const STUN_WAIT: Duration = Duration::from_secs(3);
/// 直连模式：一个 peer 加进来后，加密探测（见 [`paths::PeerPaths::punch_eagerly`]）持续多久
const EAGER_PUNCH: Duration = Duration::from_secs(30);

/// 连协调服务时，第一条消息怎么说。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// 单网络的协调服务（1.0.0 起就有的说法）。
    Hello {
        /// 邀请码：本机还不在网里时，凭它加入。已经在网里的话带不带都一样。
        invite: Option<Invite>,
    },
    /// 进 hub 里的某个网络。
    Join {
        /// 哪个网络。
        network: NetworkId,
        /// 邀请码，同上。
        invite: Option<Invite>,
    },
    /// 在 hub 里新建一个网络，自己当网主。建好之后重连时自动改成 [`Join`](Self::Join)。
    Create {
        /// 网络的名字。
        name: String,
    },
}

impl Entry {
    fn message(&self) -> ClientMessage {
        match self {
            Self::Hello { invite } => ClientMessage::Hello { invite: *invite },
            Self::Join { network, invite } => ClientMessage::Join {
                network: *network,
                invite: *invite,
            },
            Self::Create { name } => ClientMessage::Create { name: name.clone() },
        }
    }
}

/// 管理请求的结果：成功时可能带一个新邀请码，失败时是给人看的原因。
pub type AdminResult = Result<Option<Invite>, String>;

/// 网主管理网络用的把手。见 [`Session::admin`]。不是网主也能拿到，只是请求会被拒。
#[derive(Clone)]
pub struct AdminHandle {
    requests: mpsc::UnboundedSender<(AdminRequest, oneshot::Sender<AdminResult>)>,
    roster: watch::Receiver<Option<Roster>>,
}

impl AdminHandle {
    /// 发一个管理请求，等协调服务的回音。
    pub async fn request(&self, request: AdminRequest) -> AdminResult {
        let (reply, answer) = oneshot::channel();
        self.requests
            .send((request, reply))
            .map_err(|_| "节点已经停了".to_string())?;
        match tokio::time::timeout(ADMIN_TIMEOUT, answer).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("和协调服务的连接断了，稍后再试".into()),
            Err(_) => Err("协调服务太久没回音，稍后再试".into()),
        }
    }

    /// 协调服务最近一次发来的成员清单。只有网主才有，别人一直是 `None`。
    pub fn roster(&self) -> watch::Receiver<Option<Roster>> {
        self.roster.clone()
    }

    /// 没有协调服务（直连模式）时的把手：清单一直是 `None`，请求一律办不了。
    pub fn detached() -> Self {
        let (requests, _) = mpsc::unbounded_channel();
        Self {
            requests,
            roster: watch::Sender::new(None).subscribe(),
        }
    }
}

/// 控制面的配置。
pub struct Config {
    /// 本机私钥。
    pub secret: NodeSecret,
    /// 协调服务的地址。
    pub coord: SocketAddr,
    /// 协调服务的公钥。事先知道它，才能认证协调服务（IK 的 K）。
    pub coord_key: NodeKey,
    /// 第一条消息怎么说：进哪个网络、凭什么邀请码，或者新建一个。
    pub entry: Entry,
    /// 数据面 socket 的本地端口，用来拼出本机的局域网端点。
    pub local_port: u16,
    /// 给每个 peer 的 persistent keepalive，NAT 后面的节点靠它维持映射。
    pub keepalive: Option<NonZeroU16>,
    /// 只走中继：不探测、不上报直连端点，也不选直连。
    ///
    /// UDP 整个被封的网络里用得上（反正打不通，省得白探测），排查问题时也用得上。
    /// 别人的 Ping 照样回应。
    pub relay_only: bool,
    /// 给网里别人看的名字，空串是不起。连上后告诉协调服务；连着的时候用 [`NameSetter`] 改。
    pub name: String,
    /// 本机自己当主机（协调服务、中继跑在本机）时要的额外设置；平时是空的。
    pub hosting: Hosting,
}

/// 本机当主机时的额外设置。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hosting {
    /// 额外上报的端点：路由器上用 UPnP 映射出来的公网端点。本机探测公网地址时
    /// 绕回自己的协调服务，看到的只是局域网地址，别人得靠它才找得到直连的路。
    pub extra_endpoints: Vec<SocketAddr>,
    /// 地址别名（公网地址，本机实际用的地址）：协调服务告诉大家的中继、探测地址是公网的，
    /// 本机自己去连它们时换成局域网地址 —— 不少路由器不支持从里面绕回自己的公网地址。
    pub aliases: Vec<(SocketAddr, SocketAddr)>,
}

impl Hosting {
    fn resolve(&self, addr: SocketAddr) -> SocketAddr {
        self.aliases
            .iter()
            .find(|(public, _)| *public == addr)
            .map_or(addr, |(_, local)| *local)
    }
}

/// 连着的时候改名字。见 [`Session::name_setter`]。
#[derive(Clone)]
pub struct NameSetter(Arc<watch::Sender<String>>);

impl NameSetter {
    /// 没有协调服务（直连模式）时的把手：改了也没人收。名字写在连接码里。
    pub fn detached() -> Self {
        Self(Arc::new(watch::Sender::new(String::new())))
    }

    /// 换一个名字。连着协调服务就马上告诉它；断着的话重连时带上。
    pub fn set(&self, name: impl Into<String>) {
        let name = name.into();
        self.0.send_if_modified(|current| {
            let changed = *current != name;
            if changed {
                *current = name;
            }
            changed
        });
    }
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
    /// 在 hub 里的哪个网络（单网络的协调服务没有）。
    pub network: Option<NetworkId>,
    /// 这次是新建的网络：它长期有效的邀请码。
    pub created: Option<Invite>,
}

/// 控制面此刻的样子，给界面看。见 [`Session::status`]。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// 和协调服务的连接此刻是否连着。断了的时候已经通了的 peer 照常通，只是看不到新变化。
    pub coord_connected: bool,
    /// 协调服务最近一次告诉本机的 peer，按 overlay 地址排好。
    pub peers: Vec<PeerView>,
}

/// 一个 peer 在控制面眼里的样子。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerView {
    /// peer 的身份。
    pub key: NodeKey,
    /// peer 的 overlay 地址。
    pub overlay_ip: Ipv4Addr,
    /// peer 给自己起的名字，空串是没起。谁都能随便填：认人以地址和公钥为准。
    pub name: String,
    /// 已经交给数据面的发送路径。还没选出来为 `None`。
    pub path: Option<Path>,
    /// 走直连时，这条直连平滑后的往返时间。走中继时没测，为 `None`。
    pub rtt: Option<Duration>,
    /// 走直连时，这条直连往返时间的抖动（平均偏差）。走中继时为 `None`。
    pub jitter: Option<Duration>,
    /// 走直连时，这条直连探测的丢包率（百分比，平滑过）。走中继时为 `None`。
    pub loss_percent: Option<u8>,
    /// 直接（不经中继）收到过它的报文：Ping、Pong 或者 WireGuard 握手。打洞没通时用来判断卡在哪一边：
    /// 收到过，说明对方的报文进得来、是我们的回不去；没收到过，是对方的报文到不了这里
    pub heard: bool,
    /// 正在探测的候选地址（诊断用）。
    pub candidates: Vec<SocketAddr>,
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
    status: watch::Sender<Status>,
    name: Arc<watch::Sender<String>>,
    admin: AdminHandle,
    admin_requests: mpsc::UnboundedReceiver<(AdminRequest, oneshot::Sender<AdminResult>)>,
    roster: watch::Sender<Option<Roster>>,
}

impl Session {
    /// 连上协调服务并注册。本机不在协调服务的名单里时返回 [`ControlError::Rejected`]。
    pub async fn connect(mut config: Config) -> Result<Self, ControlError> {
        let (welcome, connection) = connect_once(&config, &config.entry, &config.name).await?;
        info!(ip = %welcome.overlay_ip, network = ?welcome.network, "已注册到协调服务");
        // 新建的网络：以后重连就是"进这个网络"，不能再建一个
        if let (Entry::Create { .. }, Some(network)) = (&config.entry, welcome.network) {
            config.entry = Entry::Join {
                network,
                invite: None,
            };
        }
        let name = Arc::new(watch::Sender::new(config.name.clone()));
        let (requests, admin_requests) = mpsc::unbounded_channel();
        let roster = watch::Sender::new(None);
        let admin = AdminHandle {
            requests,
            roster: roster.subscribe(),
        };
        Ok(Self {
            config,
            welcome,
            connection,
            status: watch::Sender::new(Status {
                coord_connected: true,
                peers: Vec::new(),
            }),
            name,
            admin,
            admin_requests,
            roster,
        })
    }

    /// 网主管理网络用的把手，[`run`](Self::run) 跑起来之后照样能用。
    pub fn admin(&self) -> AdminHandle {
        self.admin.clone()
    }

    /// 改名字用的把手，[`run`](Self::run) 跑起来之后照样能用。
    pub fn name_setter(&self) -> NameSetter {
        NameSetter(Arc::clone(&self.name))
    }

    /// 订阅控制面的状态。[`run`](Self::run) 跑起来之后，每处理完一件事就更新一次（内容没变不通知）。
    pub fn status(&self) -> watch::Receiver<Status> {
        self.status.subscribe()
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
            status,
            name,
            admin: _,
            mut admin_requests,
            roster,
        } = self;
        let mut names = name.subscribe();
        // 发出去、还在等回音的管理请求，按发出的顺序
        let mut waiting: VecDeque<oneshot::Sender<AdminResult>> = VecDeque::new();
        let mut node = Node::new(&config, &welcome, dataplane);
        let mut tick = tokio::time::interval(TICK);

        loop {
            // 连着的时候。这条连接是在当前的网络上建的：此前记下的"换了网络"不再作数
            let mut last_heard = Instant::now();
            node.take_network_changed();
            loop {
                // 时间在事件到了之后再取：select 可能等了将近一个节拍，拿等待之前的时刻去记
                // Ping 的发出时间，量出来的往返时间会凭空多出这段等待
                let outgoing = tokio::select! {
                    message = connection.inbox.recv() => match message {
                        // 连着连着被拒了：网络主人把本机移出了网络。不再重连 ——
                        // 手里的邀请码要是还有效，一重连就又加回去了
                        Some(ServerMessage::Rejected { reason }) => {
                            warn!(%reason, "协调服务把本机移出了网络");
                            return Err(ControlError::Rejected(reason));
                        }
                        Some(ServerMessage::Roster(list)) => {
                            last_heard = Instant::now();
                            roster.send_replace(Some(list));
                            vec![]
                        }
                        Some(ServerMessage::AdminReply(result)) => {
                            last_heard = Instant::now();
                            if let Some(reply) = waiting.pop_front() {
                                let _ = reply.send(result);
                            }
                            vec![]
                        }
                        Some(message) => {
                            let now = Instant::now();
                            last_heard = now;
                            node.on_server_message(message, now)
                        }
                        None => break,
                    },
                    event = events.recv() => match event {
                        Some(event) => node.on_event(event, Instant::now()),
                        None => return Ok(()),
                    },
                    _ = tick.tick() => node.on_tick(Instant::now()),
                    Ok(()) = names.changed() => {
                        vec![ClientMessage::SetName(names.borrow_and_update().clone())]
                    }
                    Some((request, reply)) = admin_requests.recv() => {
                        waiting.push_back(reply);
                        vec![ClientMessage::Admin(request)]
                    }
                };
                node.publish(&status, true);
                for message in outgoing {
                    if let Err(err) = connection.writer.send(&message.encode()).await {
                        debug!(%err, "发往协调服务失败");
                    }
                }
                if last_heard.elapsed() > COORD_SILENCE {
                    warn!(silence = ?COORD_SILENCE, "协调服务太久没有回音");
                    break;
                }
                // 这条连接多半绑着已经不在的旧地址，别等它自己超时
                if node.take_network_changed() {
                    break;
                }
            }

            // 断开了：边重连边照常处理事件和节拍，只是发不了消息给协调服务。
            // 还在等回音的管理请求等不到了（丢掉回音的发送端，请求方会收到"连接断了"）
            warn!("和协调服务的连接断了，重连中");
            waiting.clear();
            node.publish(&status, false);
            let mut backoff = Duration::from_secs(1);
            connection = loop {
                let (delay, config) = (backoff, &config);
                // 断着的时候改过名字：重连时带上最新的
                let current = names.borrow_and_update().clone();
                let reconnect = async move {
                    tokio::time::sleep(delay).await;
                    connect_once(config, &config.entry, &current).await
                };
                tokio::pin!(reconnect);
                let result = loop {
                    tokio::select! {
                        result = &mut reconnect => break result,
                        event = events.recv() => match event {
                            Some(event) => drop(node.on_event(event, Instant::now())),
                            None => return Ok(()),
                        },
                        _ = tick.tick() => drop(node.on_tick(Instant::now())),
                        // 断着的时候来的管理请求：直接告诉它现在办不了
                        Some((_, reply)) = admin_requests.recv() => {
                            let _ = reply.send(Err("和协调服务的连接断了，稍后再试".into()));
                        }
                    }
                    node.publish(&status, false);
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

/// 只建网络、不留下来：连上 hub 模式的协调服务，新建一个网络（自己当网主），拿到网络 ID 和
/// 长期有效的邀请码就断开。之后照常用 [`Entry::Join`] 连进去。
///
/// 和"连上之后再建好网卡"分开做：网卡建不起来（没有管理员权限之类）的时候，
/// 建好的网络也已经拿在手里了，不会成为一个谁都进不去、还占着名额的网络。
pub async fn create_network(
    config: &Config,
    name: &str,
) -> Result<(NetworkId, Invite), ControlError> {
    let entry = Entry::Create {
        name: name.to_owned(),
    };
    let (welcome, _connection) = connect_once(config, &entry, &config.name).await?;
    match (welcome.network, welcome.created) {
        (Some(network), Some(invite)) => Ok((network, invite)),
        _ => Err(ControlError::Protocol("协调服务没说新网络的 ID 和邀请码")),
    }
}

async fn connect_once(
    config: &Config,
    entry: &Entry,
    name: &str,
) -> Result<(Welcome, Connection), ControlError> {
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
    writer.send(&entry.message().encode()).await?;

    let mut network = match entry {
        Entry::Join { network, .. } => Some(*network),
        _ => None,
    };
    let mut created = None;
    let welcome = loop {
        let bytes = tokio::time::timeout(CONNECT_TIMEOUT, reader.recv())
            .await
            .map_err(timeout("等协调服务的 Welcome"))??;
        match ServerMessage::decode(&bytes)? {
            // 新建的网络：先告诉 ID 和邀请码，接着才是 Welcome
            ServerMessage::Created {
                network: id,
                invite,
            } if matches!(entry, Entry::Create { .. }) && created.is_none() => {
                network = Some(id);
                created = Some(invite);
            }
            ServerMessage::Welcome {
                overlay_ip,
                prefix_len,
                probe,
                relays,
            } => {
                break Welcome {
                    overlay_ip,
                    prefix_len,
                    probe,
                    relays,
                    network,
                    created,
                };
            }
            ServerMessage::Rejected { reason } => return Err(ControlError::Rejected(reason)),
            _ => return Err(ControlError::Protocol("协调服务的第一条消息不是 Welcome")),
        }
    };
    // 协调服务只在内存里记名字：每次连上都告诉它一遍
    writer
        .send(&ClientMessage::SetName(name.to_owned()).encode())
        .await?;

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
    overlay_ip: Ipv4Addr,
    name: String,
    paths: PeerPaths,
    /// 已经告诉数据面的路径
    current: Option<Path>,
    last_call_me_maybe: Option<Instant>,
    /// 直接收到过它的报文（见 [`PeerView::heard`]）
    heard: bool,
}

/// 令牌桶：每秒补满
struct TokenBucket {
    capacity: u32,
    tokens: u32,
    refilled: Instant,
}

impl TokenBucket {
    fn new(capacity: u32, now: Instant) -> Self {
        Self {
            capacity,
            tokens: capacity,
            refilled: now,
        }
    }

    fn take(&mut self, now: Instant) -> bool {
        if now.duration_since(self.refilled) >= Duration::from_secs(1) {
            self.tokens = self.capacity;
            self.refilled = now;
        }
        if self.tokens == 0 {
            return false;
        }
        self.tokens -= 1;
        true
    }
}

/// 发出去、还在等回应的 Ping
struct Pending {
    to: NodeKey,
    /// 走的哪条路：直连地址，或者经哪个中继
    via: Path,
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
    relay_only: bool,
    hosting: Hosting,
    dataplane: Arc<dyn DataPlane>,
    /// 协调服务给的中继，按它排的顺序
    relays: Vec<Path>,
    probe: Option<SocketAddr>,
    peers: HashMap<NodeKey, PeerState>,
    pending: HashMap<TxId, Pending>,
    disco_budget: TokenBucket,
    reflexive: Option<SocketAddr>,
    /// 本机的局域网端点，每个节拍重新看一次。没有网络时是 None
    local: Option<SocketAddr>,
    /// 本机所有能用的局域网地址（见 [`lan_endpoints`]），每个节拍重新看一次
    lan: Vec<SocketAddr>,
    /// 本机的公网 IPv6 地址（见 [`ipv6_endpoints`]），每个节拍重新看一次；数据面没有 IPv6 时一直是空的
    ipv6: Vec<SocketAddr>,
    /// 数据面 IPv6 socket 的端口
    ipv6_port: Option<u16>,
    /// 最近一次有网络时的本机地址。换网络往往中间断一下（None），所以跟它比，不跟上一拍比
    last_local_ip: Option<IpAddr>,
    network_changed: bool,
    reported: Option<Vec<SocketAddr>>,
    next_probe: Instant,
    next_control_ping: Instant,
    /// 直连模式：问哪些公共 STUN 服务器（hub 模式是空的，用协调服务的探测）
    stun_servers: Vec<SocketAddr>,
    /// 发出去还没回的 STUN 请求：事务 ID → (服务器, 发出的时刻)
    stun_pending: HashMap<stun::TxId, (SocketAddr, Instant)>,
    /// 这一轮每个服务器看到的本机端点
    stun_seen: HashMap<SocketAddr, SocketAddr>,
    /// 这一轮 STUN 是什么时候发的
    stun_round: Option<Instant>,
    /// 直连模式的朋友：房主这个 peer 的网段是整个 overlay（经它到别的朋友）
    gateway: Option<IpNet>,
    /// 直连模式：新加的 peer 打洞期间加密探测
    eager: bool,
}

impl Node {
    fn new(config: &Config, welcome: &Welcome, dataplane: Arc<dyn DataPlane>) -> Self {
        let now = Instant::now();
        let ipv6_port = dataplane.ipv6_port();
        let mut node = Self {
            secret: config.secret.clone(),
            coord: config.coord,
            coord_key: config.coord_key,
            local_port: config.local_port,
            keepalive: config.keepalive,
            relay_only: config.relay_only,
            hosting: config.hosting.clone(),
            dataplane,
            relays: Vec::new(),
            probe: None,
            peers: HashMap::new(),
            pending: HashMap::new(),
            disco_budget: TokenBucket::new(DISCO_RATE, now),
            reflexive: None,
            local: None,
            lan: Vec::new(),
            ipv6: Vec::new(),
            ipv6_port,
            last_local_ip: None,
            network_changed: false,
            reported: None,
            next_probe: now,
            next_control_ping: now + CONTROL_PING_INTERVAL,
            stun_servers: Vec::new(),
            stun_pending: HashMap::new(),
            stun_seen: HashMap::new(),
            stun_round: None,
            gateway: None,
            eager: false,
        };
        node.on_reconnected(welcome);
        node
    }

    fn on_reconnected(&mut self, welcome: &Welcome) {
        // 每个中继都探测、都可以走：哪个挂了，探测不通，选路自然换到别的
        self.relays = welcome
            .relays
            .iter()
            .map(|relay| Path::Relay {
                relay: relay.key,
                addr: self.hosting.resolve(relay.addr),
            })
            .collect();
        for state in self.peers.values_mut() {
            state.paths.retain_relays(&self.relays);
        }
        self.probe = welcome.probe.map(|probe| self.hosting.resolve(probe));
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
            // 成员清单、管理结果在 Session::run 里就处理了；Created 只在注册时出现
            ServerMessage::Created { .. }
            | ServerMessage::Roster(_)
            | ServerMessage::AdminReply(_) => vec![],
            ServerMessage::Welcome { .. } | ServerMessage::Rejected { .. } => {
                // Rejected 在 Session::run 里就处理了，走不到这里
                debug!("注册之后又收到了 Welcome，忽略");
                vec![]
            }
        }
    }

    fn on_net_map(&mut self, peers: Vec<PeerInfo>, now: Instant) -> Vec<ClientMessage> {
        let gateway = self.gateway;
        let configs = peers.iter().map(|peer| PeerConfig {
            key: peer.key,
            allowed_ips: vec![gateway.unwrap_or(IpNet::from(IpAddr::V4(peer.overlay_ip)))],
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
            let eager = self.eager;
            let state = self.peers.entry(peer.key).or_insert_with(|| {
                info!(peer = %peer.key, ip = %peer.overlay_ip, endpoints = ?peer.endpoints, "新的 peer，开始探测");
                let mut paths = PeerPaths::default();
                if eager {
                    paths.punch_eagerly(now + EAGER_PUNCH);
                }
                PeerState {
                    overlay_ip: peer.overlay_ip,
                    name: String::new(),
                    paths,
                    current: None,
                    last_call_me_maybe: None,
                    heard: false,
                }
            });
            state.overlay_ip = peer.overlay_ip;
            state.name.clone_from(&peer.name);
            state.paths.set_advertised(&peer.endpoints);
        }
        self.update_paths(now);
        self.ping_due(now);
        vec![]
    }

    fn on_event(&mut self, event: Event, now: Instant) -> Vec<ClientMessage> {
        match event {
            Event::ControlDatagram { from, datagram } => self.on_disco(from, &datagram, now),
            Event::RelayedControl {
                peer,
                via,
                datagram,
            } => self.on_relayed_disco(peer, via, &datagram, now),
            Event::HandshakeCompleted { peer, via } => {
                debug!(%peer, ?via, "WireGuard 握手完成");
                if let Path::Direct(from) = via {
                    self.heard_from(peer, from);
                }
            }
            Event::StunDatagram { from, datagram } => self.on_stun(from, &datagram, now),
        }
        vec![]
    }

    /// STUN 回应：只认自己发过的事务、只认发往的那个服务器
    fn on_stun(&mut self, from: SocketAddr, datagram: &[u8], now: Instant) {
        if !self.disco_budget.take(now) {
            return;
        }
        let Some((tx, mapped)) = stun::parse_response(datagram) else {
            return;
        };
        let Some(&(server, _)) = self.stun_pending.get(&tx) else {
            return;
        };
        if server != from {
            return;
        }
        self.stun_pending.remove(&tx);
        self.stun_seen.insert(server, mapped);
        // 公网端点取第一个服务器（按配置的顺序）问到的
        let first = self
            .stun_servers
            .iter()
            .find_map(|server| self.stun_seen.get(server).copied());
        if first != self.reflexive {
            info!(observed = ?first, "STUN 问到本机的公网端点");
            self.reflexive = first;
        }
    }

    /// 发一轮 STUN 请求
    fn stun_round(&mut self, now: Instant) {
        self.stun_seen.clear();
        self.stun_round = Some(now);
        for server in self.stun_servers.clone() {
            let mut tx = stun::TxId::default();
            OsRng.fill_bytes(&mut tx);
            if let Err(err) = self.dataplane.send_stun(server, &stun::request(&tx)) {
                debug!(%server, %err, "发 STUN 请求失败");
                continue;
            }
            self.stun_pending.insert(tx, (server, now));
        }
    }

    /// 直连模式给界面、连接码用的 NAT 情况
    fn nat_info(&self, now: Instant) -> direct::NatInfo {
        let mut seen: Vec<SocketAddr> = self.stun_seen.values().copied().collect();
        seen.sort();
        seen.dedup();
        let answered_all = !self.stun_servers.is_empty()
            && self
                .stun_servers
                .iter()
                .all(|server| self.stun_seen.contains_key(server));
        let waited = self
            .stun_round
            .is_some_and(|round| now.duration_since(round) >= STUN_WAIT);
        direct::NatInfo {
            endpoints: self.endpoints(),
            public: self.reflexive,
            symmetric: seen.len() > 1,
            checked: answered_all || waited || self.stun_servers.is_empty(),
            mapped: self.hosting.extra_endpoints.first().copied(),
            hint: {
                // 按配置的顺序（也是发请求的顺序）取前两个回了的服务器
                let mut answered = self
                    .stun_servers
                    .iter()
                    .filter_map(|server| self.stun_seen.get(server).copied());
                match (answered.next(), answered.next()) {
                    (Some(first), Some(second)) => direct::PortHint::from_observed(first, second),
                    _ => None,
                }
            },
        }
    }

    fn on_disco(&mut self, from: SocketAddr, datagram: &[u8], now: Instant) {
        // 限速放在任何密码学运算之前
        if !self.disco_budget.take(now) {
            return;
        }
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
        self.heard_from(sender, from);
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
                if !self.relay_only && state.paths.learn(from, now) {
                    self.ping_due(now);
                }
            }
            DiscoMessage::Pong { tx, observed } => self.on_pong(sender, tx, observed, now),
        }
    }

    /// 直接收到了 `peer` 的一个（认证过的）报文。第一次收到时记一笔日志：打洞卡住时看它
    fn heard_from(&mut self, peer: NodeKey, from: SocketAddr) {
        if let Some(state) = self.peers.get_mut(&peer)
            && !state.heard
        {
            state.heard = true;
            info!(%peer, %from, "第一次直接收到对方的报文");
        }
    }

    /// 经中继来的控制报文。中继告诉了我们是谁发的，报文本身照样要验证
    fn on_relayed_disco(&mut self, peer: NodeKey, via: Path, datagram: &[u8], now: Instant) {
        if !self.disco_budget.take(now) {
            return;
        }
        let peers = &self.peers;
        let opened = disco::open(&self.secret, datagram, |key| peers.contains_key(key));
        let (sender, message) = match opened {
            Ok(opened) => opened,
            Err(err) => {
                debug!(%peer, %err, "丢弃一条经中继来的控制报文");
                return;
            }
        };
        // 中继说是 peer 发的，报文里的签名却是别人：不认
        if sender != peer {
            debug!(%peer, %sender, "经中继来的控制报文发送方对不上，丢弃");
            return;
        }
        match message {
            DiscoMessage::Ping { tx } => {
                // 原路（经同一个中继）回过去：对方一定连着这个中继。
                // observed 对经中继的探测没有意义，填中继的地址
                let Path::Relay { addr, .. } = via else {
                    return;
                };
                let pong = disco::seal(
                    &self.secret,
                    &sender,
                    &DiscoMessage::Pong { tx, observed: addr },
                );
                if let Err(err) = self.dataplane.send_control_via(via, &sender, &pong) {
                    debug!(%sender, %err, "经中继发 Pong 失败");
                }
            }
            DiscoMessage::Pong { tx, observed } => self.on_pong(sender, tx, observed, now),
        }
    }

    /// 收到一个 Pong：对上是哪次 Ping，记下往返时间
    fn on_pong(&mut self, sender: NodeKey, tx: TxId, observed: SocketAddr, now: Instant) {
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
            match pending.via {
                Path::Direct(addr) => state.paths.on_pong(addr, rtt, now),
                relay @ Path::Relay { .. } => state.paths.on_relay_pong(relay, rtt, now),
            }
            self.update_paths(now);
        }
    }

    fn on_tick(&mut self, now: Instant) -> Vec<ClientMessage> {
        let mut outgoing = Vec::new();
        self.pending
            .retain(|_, pending| now.duration_since(pending.sent) < PING_TIMEOUT);
        self.stun_pending
            .retain(|_, (_, sent)| now.duration_since(*sent) < PING_TIMEOUT);

        if !self.stun_servers.is_empty() && now >= self.next_probe {
            self.stun_round(now);
            self.next_probe = now + PROBE_INTERVAL;
        }

        if let Some(probe) = self.probe
            && now >= self.next_probe
        {
            self.ping(self.coord_key, probe, now);
            self.next_probe = now + PROBE_INTERVAL;
        }

        self.check_network(now);
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
            if !self.relay_only && !state.paths.has_fresh(now) && waited {
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

    /// 看看本机换没换网络：通往协调服务的那个本地地址变了，就是换了（Wi-Fi 换成蜂窝网、
    /// 换了一个路由器……）。换了的话，探测到的公网地址也作废，马上重新探测；
    /// 到协调服务的连接由 [`Session::run`] 重连
    fn check_network(&mut self, now: Instant) {
        self.local = local_endpoint(self.coord, self.local_port);
        self.lan = lan_endpoints(self.local_port);
        self.ipv6 = self.ipv6_port.map(ipv6_endpoints).unwrap_or_default();
        let Some(new) = self.local.map(|local| local.ip()) else {
            return;
        };
        if let Some(old) = self.last_local_ip
            && old != new
        {
            info!(%old, %new, "本机换了网络，重连协调服务、重新探测");
            self.network_changed = true;
            self.reflexive = None;
            self.stun_seen.clear();
            self.next_probe = now;
            // 直连是在旧网络上打通的：一律当作不通，先走中继，重新探测。
            // 不这样的话要等它们一个个超时，这几秒里游戏的报文都发进了黑洞
            for state in self.peers.values_mut() {
                state.paths.network_changed();
            }
        }
        self.last_local_ip = Some(new);
    }

    /// 把此刻的样子交给订阅者，没变就不通知
    fn publish(&self, status: &watch::Sender<Status>, coord_connected: bool) {
        let mut peers: Vec<PeerView> = self
            .peers
            .iter()
            .map(|(key, state)| PeerView {
                key: *key,
                overlay_ip: state.overlay_ip,
                name: state.name.clone(),
                path: state.current,
                rtt: match state.current {
                    Some(Path::Direct(addr)) => state.paths.rtt(addr),
                    Some(relay @ Path::Relay { .. }) => {
                        state.paths.relay_quality(relay).map(|q| q.0)
                    }
                    None => None,
                },
                jitter: match state.current {
                    Some(Path::Direct(addr)) => state.paths.jitter(addr),
                    Some(relay @ Path::Relay { .. }) => {
                        state.paths.relay_quality(relay).map(|q| q.1)
                    }
                    None => None,
                },
                loss_percent: match state.current {
                    Some(Path::Direct(addr)) => state.paths.loss(addr),
                    Some(relay @ Path::Relay { .. }) => {
                        state.paths.relay_quality(relay).map(|q| q.2)
                    }
                    None => None,
                }
                .map(|loss| (loss * 100.0).round().clamp(0.0, 100.0) as u8),
                heard: state.heard,
                candidates: state.paths.candidate_addrs(),
            })
            .collect();
        peers.sort_by_key(|peer| peer.overlay_ip);
        let next = Status {
            coord_connected,
            peers,
        };
        status.send_if_modified(|current| {
            let changed = *current != next;
            if changed {
                *current = next;
            }
            changed
        });
    }

    fn take_network_changed(&mut self) -> bool {
        std::mem::take(&mut self.network_changed)
    }

    /// 本机的候选端点：局域网地址，加上探测到的公网地址
    fn endpoints(&self) -> Vec<SocketAddr> {
        let mut endpoints = Vec::new();
        if self.relay_only {
            return endpoints;
        }
        // 每块真实网卡的局域网地址都带上：在同一个局域网里的人靠它直接连，不用绕路由器
        // （很多路由器不支持从里面绕回自己的公网地址）。通往外面那条路由用的网卡排第一
        if let Some(local) = self.local
            && usable_lan(local.ip(), None)
        {
            endpoints.push(local);
        }
        for lan in &self.lan {
            if !endpoints.contains(lan) {
                endpoints.push(*lan);
            }
        }
        // 公网 IPv6：不用问 STUN，地址本身就是公网的
        for v6 in &self.ipv6 {
            if !endpoints.contains(v6) {
                endpoints.push(*v6);
            }
        }
        if let Some(reflexive) = self.reflexive
            && !endpoints.contains(&reflexive)
        {
            endpoints.push(reflexive);
        }
        for extra in &self.hosting.extra_endpoints {
            if !endpoints.contains(extra) {
                endpoints.push(*extra);
            }
        }
        endpoints
    }

    /// 探测所有到时候的候选
    fn ping_due(&mut self, now: Instant) {
        // 中继那条路也探测：知道它多快、丢不丢包，才能和直连比；走中继时界面上也有延迟可看。
        // 只走中继的节点也探测它。每个中继都探测：顺带让数据面连着每个中继，
        // 对方经哪个中继发来都收得到
        let mut due = Vec::new();
        for &relay in &self.relays {
            for (key, state) in &mut self.peers {
                let active = state.current == Some(relay);
                if state.paths.relay_due(relay, now, active) {
                    due.push((*key, relay));
                }
            }
        }
        for (key, relay) in due {
            self.ping_via_relay(key, relay, now);
        }
        if self.relay_only {
            return;
        }
        let due: Vec<(NodeKey, SocketAddr)> = self
            .peers
            .iter_mut()
            .flat_map(|(key, state)| {
                // 正在用的直连探测得更勤：它断了要尽快发现。正在走中继时，最好的那条通着的直连
                // 也这样探测：要回到直连，得先把它测准
                let active = match state.current {
                    Some(Path::Direct(addr)) => Some(addr),
                    _ => state.paths.best_direct(now),
                };
                state
                    .paths
                    .due_pings(now, active)
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
                via: Path::Direct(addr),
                sent: now,
            },
        );
    }

    fn ping_via_relay(&mut self, to: NodeKey, relay: Path, now: Instant) {
        let mut tx = TxId::default();
        OsRng.fill_bytes(&mut tx);
        let datagram = disco::seal(&self.secret, &to, &DiscoMessage::Ping { tx });
        if let Err(err) = self.dataplane.send_control_via(relay, &to, &datagram) {
            debug!(peer = %to, %err, "经中继发 Ping 失败");
            return;
        }
        self.pending.insert(
            tx,
            Pending {
                to,
                via: relay,
                sent: now,
            },
        );
    }

    /// 按选路规则算出每个 peer 该走的路，变了就告诉数据面
    fn update_paths(&mut self, now: Instant) {
        for (key, state) in &mut self.peers {
            // 只走中继的节点从不探测直连，直连永远不"通"，选出来的只会是中继
            let Some(desired) = state.paths.choose(state.current, &self.relays, now) else {
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

/// 最多带几个局域网地址：虚拟机、容器的网卡也是私网地址，全带上会挤掉公网端点
const MAX_LAN: usize = 4;

/// 本机每块网卡上能用的局域网地址（见 [`usable_lan`]），配上数据面 socket 的端口
fn lan_endpoints(port: u16) -> Vec<SocketAddr> {
    let Ok(interfaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for interface in interfaces {
        let if_addrs::IfAddr::V4(v4) = interface.addr else {
            continue;
        };
        let prefix = u32::from(v4.netmask).leading_ones() as u8;
        let ip = IpAddr::V4(v4.ip);
        let addr = SocketAddr::new(ip, port);
        if usable_lan(ip, Some(prefix)) && !out.contains(&addr) {
            out.push(addr);
        }
        if out.len() == MAX_LAN {
            break;
        }
    }
    out
}

/// 最多带几个 IPv6 地址：系统常常同时有一个固定的和几个临时的，带两个够了
const MAX_IPV6: usize = 2;

/// 本机每块网卡上的公网 IPv6 地址（全球单播 2000::/3），配上数据面 IPv6 socket 的端口
fn ipv6_endpoints(port: u16) -> Vec<SocketAddr> {
    let Ok(interfaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for interface in interfaces {
        let if_addrs::IfAddr::V6(v6) = interface.addr else {
            continue;
        };
        let addr = SocketAddr::new(IpAddr::V6(v6.ip), port);
        if global_ipv6(v6.ip) && !out.contains(&addr) {
            out.push(addr);
        }
        if out.len() == MAX_IPV6 {
            break;
        }
    }
    out
}

/// 公网 IPv6：全球单播（2000::/3），去掉文档用的 2001:db8::/32 和 Teredo（2001::/32，经 IPv4 隧道，打洞不靠谱）
fn global_ipv6(ip: std::net::Ipv6Addr) -> bool {
    let s = ip.segments();
    let global = (s[0] & 0xe000) == 0x2000;
    let documentation = s[0] == 0x2001 && s[1] == 0x0db8;
    let teredo = s[0] == 0x2001 && s[1] == 0;
    global && !documentation && !teredo
}

/// 一个本机地址能不能当局域网候选：
///
/// - 只要私网地址（10/8、172.16/12、192.168/16）。公网地址由 STUN 问
/// - 不要 198.18.0.0/15：代理软件（Clash、mihomo 的 TUN、fake-ip）用的虚拟网卡，别人往那发收不到
/// - 不要 100.64.0.0/10：Meshora 自己的网卡（还有运营商级 NAT 的地址）
/// - 不要掩码在 /30 以上的：点对点的虚拟网卡（sing-box 默认 172.19.0.1/30 之类），不是真的局域网
fn usable_lan(ip: IpAddr, prefix_len: Option<u8>) -> bool {
    let IpAddr::V4(v4) = ip else {
        return false;
    };
    let [a, b, ..] = v4.octets();
    let benchmark = a == 198 && (b & 0xfe) == 18;
    let shared = a == 100 && (b & 0xc0) == 64;
    let point_to_point = prefix_len.is_some_and(|len| len >= 30);
    v4.is_private() && !benchmark && !shared && !point_to_point
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

#[cfg(test)]
mod lan_tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn real_lan_addresses_are_candidates() {
        assert!(usable_lan(ip("192.168.1.20"), Some(24)));
        assert!(usable_lan(ip("10.0.0.5"), Some(8)));
        assert!(usable_lan(ip("172.20.3.4"), Some(16)));
        assert!(usable_lan(ip("192.168.1.20"), None));
    }

    #[test]
    fn proxy_and_overlay_adapters_are_not() {
        // Clash / mihomo 的 TUN（fake-ip）
        assert!(!usable_lan(ip("198.18.0.1"), Some(16)));
        assert!(!usable_lan(ip("198.19.255.1"), None));
        // sing-box 的点对点 TUN
        assert!(!usable_lan(ip("172.19.0.1"), Some(30)));
        // Meshora 自己的网卡、运营商级 NAT
        assert!(!usable_lan(ip("100.96.0.1"), Some(24)));
        assert!(!usable_lan(ip("100.64.0.3"), Some(10)));
        // 公网、回环、链路本地、IPv6
        assert!(!usable_lan(ip("119.123.186.54"), None));
        assert!(!usable_lan(ip("127.0.0.1"), Some(8)));
        assert!(!usable_lan(ip("169.254.3.4"), Some(16)));
        assert!(!usable_lan(ip("fd00::1"), None));
    }

    #[test]
    fn only_global_ipv6_counts() {
        let v6 = |s: &str| s.parse::<std::net::Ipv6Addr>().unwrap();
        assert!(global_ipv6(v6("240e:3b7:1234::5")));
        assert!(global_ipv6(v6("2408:8207:1::1")));
        assert!(!global_ipv6(v6("fe80::1")), "链路本地");
        assert!(!global_ipv6(v6("fd12:3456::1")), "唯一本地");
        assert!(!global_ipv6(v6("::1")));
        assert!(!global_ipv6(v6("2001:db8::1")), "文档用");
        assert!(!global_ipv6(v6("2001:0:4136:e378::1")), "Teredo");
    }

    #[test]
    fn lan_endpoints_carry_the_port_and_are_capped() {
        let found = lan_endpoints(41641);
        assert!(found.len() <= MAX_LAN);
        assert!(
            found
                .iter()
                .all(|addr| addr.port() == 41641 && usable_lan(addr.ip(), None))
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_bucket_refills_every_second() {
        let now = Instant::now();
        let mut bucket = TokenBucket::new(2, now);
        assert!(bucket.take(now));
        assert!(bucket.take(now));
        assert!(!bucket.take(now), "用完了");
        assert!(!bucket.take(now + Duration::from_millis(999)));
        assert!(bucket.take(now + Duration::from_secs(1)), "一秒后补满");
    }
}
