//! 把控制面、数据面、虚拟网卡接起来的那一段。命令行的 meshorad 和桌面端共用。
//!
//! 顺序：向协调服务注册拿到 overlay 地址 → 按这个地址建虚拟网卡 → 起数据面 → 跑控制面。
//! 需要管理员权限（建虚拟网卡）。必须在 tokio 的多线程运行时里调用。

mod network;

use std::fmt;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::num::NonZeroU16;
use std::sync::Arc;

use meshora_control::direct::{DirectConfig, DirectHandle, DirectPeer, DirectSession, Role};
use meshora_control::{Config, ControlError, NameSetter, Session, Status, Welcome};
use meshora_dataplane::{DataPlane, PeerStatus};
use meshora_tun::Pipes;
use meshora_types::{NodeKey, NodeSecret};
use meshora_wg::{TunChannels, UserspaceDataPlane};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tracing::info;

pub use meshora_control::direct;
pub use meshora_control::{AdminHandle, AdminResult, Entry, Hosting};
pub use meshora_control::{AdminRequest, Roster, RosterInvite, RosterMember};
pub use meshora_tun::{Tun, TunConfig};
pub use network::{NetworkCode, ParseNetworkCodeError};

/// 自己建虚拟网卡的办法。不给就用 [`Tun::open`]（Windows、Linux）；安卓上网卡要请系统的 VpnService 建，
/// 客户端在这里接上它。拿到的配置里是协调服务刚分的地址。会在异步任务里同步地调用，可以阻塞一会儿
pub type TunOpener = Arc<dyn Fn(&TunConfig) -> io::Result<Tun> + Send + Sync>;

/// 启动一个节点要的东西。
pub struct Options {
    /// 本机私钥。
    pub secret: NodeSecret,
    /// 协调服务的地址。
    pub coord: SocketAddr,
    /// 协调服务的公钥。
    pub coord_key: NodeKey,
    /// 第一条消息怎么说：进哪个网络（凭什么邀请码），或者新建一个。见 [`NetworkCode::entry`]。
    pub entry: Entry,
    /// WireGuard 和控制报文共用的 UDP 端口，0 表示让系统挑。
    pub port: u16,
    /// 虚拟网卡的名字。
    pub tun: String,
    /// 虚拟网卡的 MTU。
    pub mtu: u16,
    /// 虚拟网卡的接口跃点数，见 [`TunConfig::metric`]。
    pub metric: Option<u32>,
    /// persistent keepalive。
    pub keepalive: Option<NonZeroU16>,
    /// 只走中继，不尝试直连。
    pub relay_only: bool,
    /// 给网里别人看的名字，空串是不起。
    pub name: String,
    /// 本机当主机时的额外设置，见 [`Hosting`]。
    pub hosting: Hosting,
    /// 自己建虚拟网卡的办法，见 [`TunOpener`]。`None` 是 [`Tun::open`]。
    pub open_tun: Option<TunOpener>,
}

/// 启动失败的原因。
#[derive(Debug)]
pub enum StartError {
    /// 绑定 UDP 端口失败。
    Bind(u16, io::Error),
    /// 注册失败。协调服务不认本机时是 [`ControlError::Rejected`]。
    Register(ControlError),
    /// 创建或启动虚拟网卡失败。多半是没有管理员权限，或者 wintun.dll 不在程序旁边。
    Tun(String, io::Error),
    /// 启动数据面失败。
    DataPlane(io::Error),
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bind(port, err) => write!(f, "绑定 UDP 端口 {port} 失败：{err}"),
            Self::Register(err) => write!(f, "注册失败：{err}"),
            Self::Tun(name, err) => write!(f, "创建虚拟网卡 {name} 失败：{err}"),
            Self::DataPlane(err) => write!(f, "启动数据面失败：{err}"),
        }
    }
}

impl std::error::Error for StartError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Bind(_, err) | Self::Tun(_, err) | Self::DataPlane(err) => Some(err),
            Self::Register(err) => Some(err),
        }
    }
}

/// 一个跑起来的节点。丢掉它就停下：控制面停止，数据面的任务结束，虚拟网卡被删掉。
pub struct Node {
    welcome: Welcome,
    tun_name: String,
    dataplane: Arc<UserspaceDataPlane>,
    status: watch::Receiver<Status>,
    name: NameSetter,
    admin: AdminHandle,
    direct: Option<DirectHandle>,
    control: JoinHandle<Result<(), ControlError>>,
}

/// 启动一个直连模式（不用服务器）的节点要的东西。
pub struct DirectOptions {
    /// 本机私钥。
    pub secret: NodeSecret,
    /// WireGuard 和控制报文共用的 UDP 端口，0 表示让系统挑。
    pub port: u16,
    /// 虚拟网卡的名字。
    pub tun: String,
    /// 虚拟网卡的 MTU。
    pub mtu: u16,
    /// 虚拟网卡的接口跃点数。
    pub metric: Option<u32>,
    /// persistent keepalive：打通的洞靠它维持。
    pub keepalive: NonZeroU16,
    /// 本机的 overlay 地址（房主是 [`direct::HOST_IP`]，朋友是房主码里分的）。
    pub ip: std::net::Ipv4Addr,
    /// 房主还是朋友。
    pub role: Role,
    /// 问哪些 STUN 服务器（`主机名:端口`），解析不了的跳过。
    pub stun: Vec<String>,
    /// 一开始就认识的 peer。
    pub peers: Vec<DirectPeer>,
    /// 自己建虚拟网卡的办法，见 [`TunOpener`]。
    pub open_tun: Option<TunOpener>,
}

/// 启动一个直连模式的节点：不连任何服务器。顺序和 [`start`] 一样：网卡 → 数据面 → 控制面，
/// 只是地址不是协调服务分的，是自己定的（房主）或者房主码里给的（朋友）。
pub async fn start_direct(options: DirectOptions) -> Result<Node, StartError> {
    info!(key = %options.secret.public_key(), "本机身份（直连模式）");
    let socket = UdpSocket::bind(("0.0.0.0", options.port))
        .map_err(|err| StartError::Bind(options.port, err))?;
    let local_port = socket
        .local_addr()
        .map_err(|err| StartError::Bind(options.port, err))?
        .port();

    // 数据面的 socket 绑的是 IPv4，只问 IPv4 的 STUN 地址
    let mut stun = Vec::new();
    for host in &options.stun {
        match tokio::net::lookup_host(host.as_str()).await {
            Ok(addrs) => stun.extend(addrs.filter(SocketAddr::is_ipv4).take(1)),
            Err(err) => info!(%host, %err, "解析不了 STUN 服务器，跳过"),
        }
    }

    let session = DirectSession::new(DirectConfig {
        secret: options.secret.clone(),
        local_port,
        keepalive: options.keepalive,
        ip: options.ip,
        role: options.role,
        stun,
        peers: options.peers,
    });
    let welcome = session.welcome().clone();

    let tun_error = |err| StartError::Tun(options.tun.clone(), err);
    let tun_config = TunConfig {
        name: options.tun.clone(),
        address: welcome.overlay_ip,
        prefix_len: welcome.prefix_len,
        mtu: options.mtu,
        metric: options.metric,
    };
    let tun = match &options.open_tun {
        Some(open) => open(&tun_config),
        None => Tun::open(&tun_config),
    }
    .map_err(tun_error)?;
    let tun_name = tun.name().to_owned();
    let Pipes { from_tun, to_tun } = tun.spawn().map_err(tun_error)?;

    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let sink = move |event| {
        let _ = events_tx.send(event);
    };
    let dataplane = Arc::new(
        UserspaceDataPlane::start_dual(
            &options.secret,
            socket,
            bind_ipv6(local_port),
            TunChannels { from_tun, to_tun },
            Arc::new(sink),
        )
        .map_err(StartError::DataPlane)?,
    );
    dataplane.set_lan(welcome.overlay_ip, welcome.prefix_len);
    // 房主替朋友们互相转发：朋友之间没交换过码
    dataplane.set_forwarding(options.role == Role::Host);

    let status = session.status();
    let direct = session.handle();
    let control =
        tokio::spawn(session.run(Arc::clone(&dataplane) as Arc<dyn DataPlane>, events_rx));
    Ok(Node {
        welcome,
        tun_name,
        dataplane,
        status,
        name: NameSetter::detached(),
        admin: AdminHandle::detached(),
        direct: Some(direct),
        control,
    })
}

/// 在同一个端口号上绑一个只收发 IPv6 的 socket（IPV6_V6ONLY）。本机没有 IPv6、端口被占了，就是 `None`：
/// 只是少了 IPv6 这条路，IPv4 照常
fn bind_ipv6(port: u16) -> Option<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP)).ok()?;
    socket.set_only_v6(true).ok()?;
    let addr = SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED, port));
    if let Err(err) = socket.bind(&addr.into()) {
        info!(port, %err, "绑不上 IPv6 的端口，只走 IPv4");
        return None;
    }
    Some(socket.into())
}

/// 启动一个节点。
pub async fn start(options: Options) -> Result<Node, StartError> {
    info!(key = %options.secret.public_key(), "本机身份");

    let socket = UdpSocket::bind(("0.0.0.0", options.port))
        .map_err(|err| StartError::Bind(options.port, err))?;
    // 端口为 0 时由系统挑，上报给协调服务的得是实际绑上的那个
    let local_port = socket
        .local_addr()
        .map_err(|err| StartError::Bind(options.port, err))?
        .port();

    let session = Session::connect(Config {
        secret: options.secret.clone(),
        coord: options.coord,
        coord_key: options.coord_key,
        entry: options.entry,
        local_port,
        keepalive: options.keepalive,
        relay_only: options.relay_only,
        name: options.name,
        hosting: options.hosting,
    })
    .await
    .map_err(StartError::Register)?;
    let welcome = session.welcome().clone();

    let tun_error = |err| StartError::Tun(options.tun.clone(), err);
    let tun_config = TunConfig {
        name: options.tun.clone(),
        address: welcome.overlay_ip,
        prefix_len: welcome.prefix_len,
        mtu: options.mtu,
        metric: options.metric,
    };
    let tun = match &options.open_tun {
        Some(open) => open(&tun_config),
        None => Tun::open(&tun_config),
    }
    .map_err(tun_error)?;
    let tun_name = tun.name().to_owned();
    info!(
        tun = %tun_name,
        ip = %welcome.overlay_ip,
        prefix = welcome.prefix_len,
        "虚拟网卡已就绪"
    );
    let Pipes { from_tun, to_tun } = tun.spawn().map_err(tun_error)?;

    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let sink = move |event| {
        let _ = events_tx.send(event);
    };
    let dataplane = Arc::new(
        UserspaceDataPlane::start_dual(
            &options.secret,
            socket,
            bind_ipv6(local_port),
            TunChannels { from_tun, to_tun },
            Arc::new(sink),
        )
        .map_err(StartError::DataPlane)?,
    );
    // 让数据面认得出发往本网段广播地址的报文 —— 局域网游戏找房间会用到
    dataplane.set_lan(welcome.overlay_ip, welcome.prefix_len);

    let status = session.status();
    let name = session.name_setter();
    let admin = session.admin();
    let control =
        tokio::spawn(session.run(Arc::clone(&dataplane) as Arc<dyn DataPlane>, events_rx));
    Ok(Node {
        welcome,
        tun_name,
        dataplane,
        status,
        name,
        admin,
        direct: None,
        control,
    })
}

impl Node {
    /// 协调服务分配的地址等信息。
    pub fn welcome(&self) -> &Welcome {
        &self.welcome
    }

    /// 系统里虚拟网卡实际的名字。
    pub fn tun_name(&self) -> &str {
        &self.tun_name
    }

    /// 订阅控制面的状态。
    pub fn status(&self) -> watch::Receiver<Status> {
        self.status.clone()
    }

    /// 网主管理网络用的把手（踢人、邀请码、解散……），以及网主看到的成员清单。
    pub fn admin(&self) -> AdminHandle {
        self.admin.clone()
    }

    /// 改名字用的把手：换一个给网里别人看的名字，不用重连。节点停了再用也无妨，只是没人收。
    pub fn name_setter(&self) -> NameSetter {
        self.name.clone()
    }

    /// WireGuard 和控制报文共用的那个 UDP 端口（绑定时让系统挑的话，是它挑的那个）。
    pub fn local_port(&self) -> Option<u16> {
        self.dataplane.local_addr().ok().map(|addr| addr.port())
    }

    /// 直连模式的把手（改 peer、看 NAT 情况）。连服务器的节点没有。
    pub fn direct(&self) -> Option<&DirectHandle> {
        self.direct.as_ref()
    }

    /// 数据面眼里每个 peer 的状态（握手、流量）。
    pub fn peers(&self) -> Vec<PeerStatus> {
        self.dataplane.status()
    }

    /// 控制面还在不在跑。不在了，[`wait`](Self::wait) 立刻返回它退出的原因。
    pub fn is_running(&self) -> bool {
        !self.control.is_finished()
    }

    /// 等控制面退出。正常情况下它一直跑，退出就是出了无法恢复的错
    /// （比如协调服务把本机从名单里删了）。
    pub async fn wait(&mut self) -> Result<(), ControlError> {
        match (&mut self.control).await {
            Ok(result) => result,
            Err(err) => Err(ControlError::Io(io::Error::other(err))),
        }
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        self.control.abort();
    }
}
