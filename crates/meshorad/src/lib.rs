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

use meshora_control::{Config, ControlError, NameSetter, Session, Status, Welcome};
use meshora_dataplane::{DataPlane, PeerStatus};
use meshora_tun::{Pipes, Tun, TunConfig};
use meshora_types::{NodeKey, NodeSecret};
use meshora_wg::{TunChannels, UserspaceDataPlane};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tracing::info;

pub use meshora_control::{AdminHandle, AdminResult, Entry, Hosting};
pub use meshora_control::{AdminRequest, Roster, RosterInvite, RosterMember};
pub use network::{NetworkCode, ParseNetworkCodeError};

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
    control: JoinHandle<Result<(), ControlError>>,
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
    let tun = Tun::open(&TunConfig {
        name: options.tun.clone(),
        address: welcome.overlay_ip,
        prefix_len: welcome.prefix_len,
        mtu: options.mtu,
        metric: options.metric,
    })
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
        UserspaceDataPlane::start(
            &options.secret,
            socket,
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
