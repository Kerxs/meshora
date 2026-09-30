//! 协调服务：节点注册、密钥分发、端点探测、打洞对时。
//!
//! 它是整套设计里信任最集中的地方 —— 节点靠它知道"某某的公钥是什么"。
//! 被攻陷的后果见[威胁模型](https://kerxs.github.io/meshora/guide/threat-model#恶意或被攻陷的控制面)。
//! 它可以自建。
//!
//! 取舍：
//!
//! - **成员有两种来源**。一是配置里的名单（`--node`），overlay 地址按名单顺序分配：第 n 个节点拿网段里
//!   第 n 个地址，名单顺序不变地址就不变。二是**凭邀请码加入**的：节点在 Hello 里带上邀请码，
//!   核对无误就分给它最小的空闲地址，写进状态文件（[`Config::state`]），重启后地址不变。
//!   邀请码也存在状态文件里，第一次启动时生成。见[状态文件](#状态文件)
//! - **只能加人，不能在运行时踢人**：要移出一个成员，从状态文件里删掉它那一行、换掉邀请码、重启
//! - **NetMap 是整个名单**（声明式），带上每个节点最近上报的端点。节点的控制连接断了，
//!   它仍然在网里 —— 控制面抖一下，不该把已经通了的数据面也拆掉
//! - **收到第一条加密消息（Hello）之前不做任何有副作用的事**：IK 的首个握手包可以被重放（R1）
//!
//! # 状态文件
//!
//! 纯文本，一行一项，`#` 开头的是注释。人可以直接改（改完重启）：
//!
//! ```text
//! invite 3q2-7wEYkQ6n0Cf8Hs5VYA
//! member mTe0q8vN3kRZp1u5yXcW7bLdF2gH9jK4sA6eQoIiUtY= 100.64.0.3
//! ```
//!
//! - `invite`：邀请码。删掉这一行再启动，会生成一个新的 —— 旧的网络码随之失效，
//!   已经加入的成员不受影响
//! - `member`：凭邀请码加入的成员和它的地址。删掉一行，这个成员就不在网里了

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use ipnet::Ipv4Net;
use meshora_proto::admission::Admission;
use meshora_proto::control::{ClientMessage, PeerInfo, RelayInfo, ServerMessage};
use meshora_proto::disco::{self, DiscoMessage};
use meshora_proto::noise::{Channel, NoiseStream};
use meshora_types::{Invite, NodeKey, NodeSecret};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{Notify, mpsc};
use tracing::{debug, info, warn};

/// 握手和第一条消息的时限：慢吞吞的连接不许一直占着。
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// 第一条消息（Hello，一个字节）的长度上限。这时还不知道对方在不在名单里 ——
/// 谁都能现生成一把密钥完成握手 —— 不能按它自称的长度分配内存
const FIRST_MESSAGE_MAX: usize = 64;
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
/// NetMap 不走这个队列：它是全量的，只需要把最新的一份送到，见 [`Conn::net_map`]。
const QUEUE: usize = 64;
/// 给同一个节点发 NetMap 的最小间隔，期间的变化合并成一份。有人频繁上报端点，
/// 别的节点也只是每秒多收一份 NetMap，协调服务不必为每次上报给每个节点都生成一份。
const NET_MAP_INTERVAL: Duration = Duration::from_secs(1);
/// 探测端点每秒最多回应多少条。
const PROBE_RATE: u32 = 200;
/// 网里最多多少个成员（名单里的加上凭邀请码加入的）。NetMap 要发给每个人，
/// 成员数没有上限的话，一份邀请码泄漏出去就能把协调服务拖垮
pub const MAX_MEMBERS: usize = 1024;

/// 协调服务的配置。
pub struct Config {
    /// 协调服务自己的身份。节点事先知道它的公钥，靠它认证协调服务。
    pub secret: NodeSecret,
    /// 名单里的节点。overlay 地址按这个顺序分配。
    pub nodes: Vec<NodeKey>,
    /// 状态文件：邀请码和凭它加入的成员存在这里。`None` 表示不接受邀请加入，只认名单。
    pub state: Option<PathBuf>,
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

/// 一个成员
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Member {
    key: NodeKey,
    ip: Ipv4Addr,
    /// 凭邀请码加入的（存在状态文件里），不是名单里的
    joined: bool,
}

struct State {
    /// 所有成员，同时是地址表
    members: Vec<Member>,
    online: HashMap<NodeKey, Conn>,
    endpoints: HashMap<NodeKey, Vec<SocketAddr>>,
}

impl State {
    fn address_of(&self, key: &NodeKey) -> Option<Ipv4Addr> {
        self.members
            .iter()
            .find(|member| member.key == *key)
            .map(|member| member.ip)
    }
}

struct Shared {
    secret: NodeSecret,
    /// 邀请码。`None` 表示不接受邀请加入
    invite: Option<Invite>,
    /// 状态文件，有邀请码时才有
    state_file: Option<PathBuf>,
    overlay: Ipv4Net,
    probe: Option<SocketAddr>,
    relays: Vec<RelayInfo>,
    state: Mutex<State>,
    next_conn: AtomicU64,
    admission: Arc<Admission>,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("协调服务的状态在持锁时 panic 过")
    }

    fn is_member(&self, key: &NodeKey) -> bool {
        self.state().address_of(key).is_some()
    }

    /// 发给 `recipient` 的 NetMap：除它自己以外的所有成员
    fn net_map_for(&self, state: &State, recipient: &NodeKey) -> ServerMessage {
        let peers = state
            .members
            .iter()
            .filter(|member| member.key != *recipient)
            .map(|member| PeerInfo {
                key: member.key,
                overlay_ip: member.ip,
                endpoints: state
                    .endpoints
                    .get(&member.key)
                    .cloned()
                    .unwrap_or_default(),
            })
            .collect();
        ServerMessage::NetMap { peers }
    }

    /// 刚发来 Hello 的节点能不能进：在网里的直接进；不在的核对邀请码，对了就加进来。
    /// 不能进时返回给它看的原因
    fn admit(&self, key: &NodeKey, invite: Option<Invite>) -> Result<Ipv4Addr, String> {
        let mut state = self.state();
        if let Some(ip) = state.address_of(key) {
            return Ok(ip);
        }
        let (ours, theirs) = match (&self.invite, invite) {
            (Some(ours), Some(theirs)) => (ours, theirs),
            (None, Some(_)) => return Err("这个网络不接受凭邀请码加入".into()),
            (_, None) => return Err("这把公钥不在协调服务的节点名单里".into()),
        };
        if !ours.matches(&theirs) {
            return Err("邀请码不对，可能已经换过了：向建网络的人要一个新的网络码".into());
        }
        if state.members.len() >= MAX_MEMBERS {
            return Err(format!("网络已满（最多 {MAX_MEMBERS} 个成员）"));
        }
        let ip = free_address(&state.members, self.overlay)
            .ok_or_else(|| format!("overlay 网段 {} 没有空闲地址了", self.overlay))?;
        state.members.push(Member {
            key: *key,
            ip,
            joined: true,
        });
        // 先写进文件再算加入：否则重启之后这个地址可能分给别人
        if let Some(path) = &self.state_file
            && let Err(err) = save_state(path, ours, &state.members)
        {
            state.members.pop();
            warn!(%err, path = %path.display(), "写状态文件失败，拒绝新成员");
            return Err("协调服务存不下新成员，请联系建网络的人".into());
        }
        info!(node = %key, %ip, "新成员凭邀请码加入");
        // 其他人的 NetMap 里要多出这个新成员
        self.broadcast_net_maps(&state, Some(key));
        Ok(ip)
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

/// 网段里最小的、还没分出去的地址
fn free_address(members: &[Member], overlay: Ipv4Net) -> Option<Ipv4Addr> {
    overlay
        .hosts()
        .find(|ip| !members.iter().any(|member| member.ip == *ip))
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

fn invalid_data(path: &Path, line: usize, what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("状态文件 {} 第 {line} 行：{what}", path.display()),
    )
}

/// 状态文件里的东西
#[derive(Default)]
struct Stored {
    invite: Option<Invite>,
    joined: Vec<(NodeKey, Ipv4Addr)>,
}

/// 读状态文件：邀请码和凭它加入的成员。文件不在就是还没有
fn load_state(path: &Path) -> io::Result<Stored> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Stored::default()),
        Err(err) => return Err(err),
    };
    let mut invite = None;
    let mut joined = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line_no = index + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        match fields.as_slice() {
            ["invite", code] => {
                invite = Some(
                    code.parse()
                        .map_err(|_| invalid_data(path, line_no, "邀请码格式不对"))?,
                );
            }
            ["member", key, ip] => joined.push((
                key.parse()
                    .map_err(|_| invalid_data(path, line_no, "公钥格式不对"))?,
                ip.parse()
                    .map_err(|_| invalid_data(path, line_no, "地址格式不对"))?,
            )),
            _ => return Err(invalid_data(path, line_no, "认不出这一行")),
        }
    }
    Ok(Stored { invite, joined })
}

/// 整个重写状态文件：先写临时文件再改名，写到一半断电也不会留下半个文件
fn save_state(path: &Path, invite: &Invite, members: &[Member]) -> io::Result<()> {
    let mut text = String::from("# Meshora 协调服务的状态文件。可以手改，改完重启协调服务。\n");
    text.push_str(
        "# invite：邀请码，拿到它的人都能加入。删掉这一行再启动会换一个新的，旧的网络码随之失效\n",
    );
    let _ = writeln!(text, "invite {invite}");
    text.push_str("# member：凭邀请码加入的成员（公钥 地址）。删掉一行，这个成员就不在网里了\n");
    for member in members.iter().filter(|member| member.joined) {
        let _ = writeln!(text, "member {} {}", member.key, member.ip);
    }
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, path)
}

/// 一个准备好的协调服务：成员表、邀请码都已经就位，还没开始接受连接。
pub struct Coordinator {
    shared: Arc<Shared>,
}

impl Coordinator {
    /// 按配置建好成员表。有状态文件时读它；里面还没有邀请码就生成一个写回去。
    pub fn new(config: Config) -> io::Result<Self> {
        let invalid = |message: String| io::Error::new(io::ErrorKind::InvalidInput, message);
        let mut members: Vec<Member> = assign(&config.nodes, config.overlay)?
            .into_iter()
            .map(|(key, ip)| Member {
                key,
                ip,
                joined: false,
            })
            .collect();

        let mut invite = None;
        if let Some(path) = &config.state {
            let stored = load_state(path)?;
            for (key, ip) in stored.joined {
                if members.iter().any(|member| member.key == key) {
                    // 后来又写进了名单：以名单为准，这一条下次存的时候自然消失
                    continue;
                }
                if !config.overlay.contains(&ip) {
                    return Err(invalid(format!(
                        "状态文件里 {key} 的地址 {ip} 不在 overlay 网段 {} 里",
                        config.overlay
                    )));
                }
                if let Some(other) = members.iter().find(|member| member.ip == ip) {
                    return Err(invalid(format!(
                        "状态文件里 {key} 的地址 {ip} 已经分给了 {}（名单加长之后撞上了）。\
                         把名单里的这个节点挪到最后，或者从状态文件里删掉那一行",
                        other.key
                    )));
                }
                members.push(Member {
                    key,
                    ip,
                    joined: true,
                });
            }
            let invite = *invite.insert(stored.invite.unwrap_or_else(Invite::generate));
            // 写回去：新生成的邀请码要存下来；已经写进名单的成员也顺带从文件里去掉
            save_state(path, &invite, &members)?;
        }
        if members.is_empty() && invite.is_none() {
            return Err(invalid(
                "既没有名单也没有状态文件：谁都加入不了".to_string(),
            ));
        }
        if members.len() > MAX_MEMBERS {
            return Err(invalid(format!("成员超过了上限 {MAX_MEMBERS}")));
        }

        Ok(Self {
            shared: Arc::new(Shared {
                secret: config.secret,
                invite,
                state_file: config.state,
                overlay: config.overlay,
                probe: config.probe,
                relays: config.relays,
                state: Mutex::new(State {
                    members,
                    online: HashMap::new(),
                    endpoints: HashMap::new(),
                }),
                next_conn: AtomicU64::new(0),
                admission: Admission::new(PENDING_PER_SOURCE),
            }),
        })
    }

    /// 邀请码。没有状态文件时是 `None`：不接受邀请加入。
    pub fn invite(&self) -> Option<Invite> {
        self.shared.invite
    }

    /// 判断一把公钥是不是成员。同一个进程里的中继用它：凭邀请码新加入的成员也马上能用中继。
    pub fn members(&self) -> Arc<dyn Fn(&NodeKey) -> bool + Send + Sync> {
        let shared = Arc::clone(&self.shared);
        Arc::new(move |key| shared.is_member(key))
    }

    /// 运行，直到监听的 socket 出错。
    ///
    /// `probe` 是端点探测用的 UDP socket；[`Config::probe`] 是它在公网上的地址（告诉节点往哪发）。
    pub async fn serve(self, listener: TcpListener, probe: Option<UdpSocket>) -> io::Result<()> {
        run(self.shared, listener, probe).await
    }
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
    info!(
        key = %shared.secret.public_key(),
        nodes = shared.state().members.len(),
        invite = shared.invite.is_some(),
        "协调服务启动"
    );

    if let Some(socket) = probe {
        tokio::spawn(probe_loop(Arc::clone(&shared), socket));
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

async fn handle(shared: Arc<Shared>, tcp: TcpStream, from: SocketAddr) {
    // 名额一直占到确认对方在名单里、打过招呼为止
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
    let hello =
        match tokio::time::timeout(HANDSHAKE_TIMEOUT, reader.recv_at_most(FIRST_MESSAGE_MAX)).await
        {
            Ok(Ok(bytes)) => ClientMessage::decode(&bytes),
            _ => {
                debug!(%from, node = %key, "握手后没等到第一条消息");
                return;
            }
        };
    let Ok(ClientMessage::Hello { invite }) = hello else {
        debug!(%from, node = %key, "握手后第一条不是 Hello");
        return;
    };
    let overlay_ip = match shared.admit(&key, invite) {
        Ok(ip) => ip,
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
            Ok(ClientMessage::Hello { .. }) => {}
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
