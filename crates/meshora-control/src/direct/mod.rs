//! 直连模式：不用任何服务器，靠手动交换连接码和 NAT 打洞把几台设备连起来。
//!
//! - 房主给每位朋友发一个房主码（[`Offer`]），朋友贴进去，回一个回执码（[`Reply`]），房主再贴进去
//! - 码里的公网端点是各自问公共 STUN 服务器得来的（[`crate::stun`]）
//! - 两边都知道对方的端点之后，同时向对方发探测（和 hub 模式的打洞是同一套逻辑、同一个节奏）：
//!   两边的 NAT 都先为"往对方去"开了映射，对方的报文就进得来
//! - **没有中继兜底。** 一边是对称型 NAT（每个目的地换一个端口）时多半打不通，只能换用服务器
//! - 房主是中心：朋友之间不交换码，彼此的报文经房主转发（数据面的转发开关）
//!
//! 这里只是控制面：拿 peer 列表喂给和 hub 模式同一个 `Node`（探测、选路），没有协调服务。
//! 谁分哪个地址、码怎么传，由调用方（客户端）管。

mod code;

pub(crate) use code::probeable;

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::num::NonZeroU16;
use std::sync::Arc;
use std::time::Instant;

use ipnet::{IpNet, Ipv4Net};
use meshora_dataplane::{DataPlane, Event};
use meshora_proto::control::PeerInfo;
use meshora_types::{NodeKey, NodeSecret};
use tokio::sync::{mpsc, watch};

use crate::{Config, ControlError, Entry, Hosting, Node, Status, TICK, Welcome};

pub use code::{
    CODE_TTL, CodeError, OFFER_PREFIX, Offer, PortHint, REPLY_PREFIX, Reply, expires_from_now,
};

/// 直连模式的 overlay 网段：房主是 `.1`，朋友从 `.2` 起。
pub const NETWORK: Ipv4Net = match Ipv4Net::new(Ipv4Addr::new(100, 96, 0, 0), 24) {
    Ok(net) => net,
    Err(_) => panic!("网段写错了"),
};
/// 房主的 overlay 地址。
pub const HOST_IP: Ipv4Addr = Ipv4Addr::new(100, 96, 0, 1);

/// 默认问的公共 STUN 服务器。国内的排前面（国外的在国内常常不通），问不通的跳过。
/// 至少要两个回了，才判断得出 NAT 的类型
pub const DEFAULT_STUN: &[&str] = &[
    "stun.miwifi.com:3478",
    "stun.chat.bilibili.com:3478",
    "stun.douyucdn.cn:18000",
    "stun.cloudflare.com:3478",
    "stun.l.google.com:19302",
];

/// 一个直连的 peer：房主眼里的一位朋友，或者朋友眼里的房主。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectPeer {
    /// 身份。
    pub key: NodeKey,
    /// overlay 地址。
    pub ip: Ipv4Addr,
    /// 名字（码里带的，对方自己起的）。
    pub name: String,
    /// 候选端点。
    pub endpoints: Vec<SocketAddr>,
    /// 对方是对称型 NAT 时的端口提示：刚加进来的头 30 秒往预测的那一段端口也探测（见 [`PortHint::predict`]）。
    pub hint: Option<PortHint>,
}

/// 预测的端点只在刚加进来的这段时间里探测：一直扫一大段端口，打不通也是白发
const PREDICT_FOR: std::time::Duration = std::time::Duration::from_secs(30);

/// 本机在直连网络里的角色。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// 房主：认识每位朋友，替他们之间转发。
    Host,
    /// 朋友：只认识房主，把房主当整个网段的出口。
    Guest,
}

/// 直连模式的配置。
pub struct DirectConfig {
    /// 本机私钥。
    pub secret: NodeSecret,
    /// 数据面 socket 的本地端口。
    pub local_port: u16,
    /// persistent keepalive：打通之后靠它维持 NAT 映射。直连模式一定要有。
    pub keepalive: NonZeroU16,
    /// 本机的 overlay 地址。
    pub ip: Ipv4Addr,
    /// 角色。
    pub role: Role,
    /// 问哪些 STUN 服务器（已经解析好的地址）。
    pub stun: Vec<SocketAddr>,
    /// 一开始就认识的 peer（重启后接着连上次的）。
    pub peers: Vec<DirectPeer>,
}

/// 本机这一侧的 NAT 情况，给界面看、写进连接码。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NatInfo {
    /// 写进连接码的候选端点：局域网地址、问到的公网地址。
    pub endpoints: Vec<SocketAddr>,
    /// STUN 问到的公网端点。
    pub public: Option<SocketAddr>,
    /// 几个 STUN 服务器看到的是同一个公网 IP、不同的端口：对称型 NAT（每个目的地换一个端口）。
    pub symmetric: bool,
    /// 像是开着代理：STUN 服务器的域名被解析成了 198.18.x.x（Clash 之类的 fake-ip），
    /// 或者几个 STUN 服务器看到的公网 IP 不一样（流量从不同的出口出去）。这时公网 IPv4 端点多半是代理的，对方连不进来
    pub proxied: bool,
    /// 这一轮 STUN 问完了（都回了，或者等够了）。生成连接码前等它。
    pub checked: bool,
    /// 路由器用 UPnP 映射出来的公网端点（见 [`DirectHandle::set_mapped`]）。
    pub mapped: Option<SocketAddr>,
    /// 本机是对称型 NAT、端口像是按顺序分配时的提示，写进连接码。
    pub hint: Option<PortHint>,
}

enum Command {
    Set(Vec<DirectPeer>),
    Mapped(Option<SocketAddr>),
}

/// 跑起来之后改 peer 列表、看 NAT 情况用的把手。
#[derive(Clone)]
pub struct DirectHandle {
    commands: mpsc::UnboundedSender<Command>,
    nat: watch::Receiver<NatInfo>,
    role: Role,
    ip: Ipv4Addr,
}

impl DirectHandle {
    /// 换一份完整的 peer 列表（声明式：没列出的就断开）。
    pub fn set_peers(&self, peers: Vec<DirectPeer>) {
        let _ = self.commands.send(Command::Set(peers));
    }

    /// 路由器用 UPnP 给本机映射出来的公网端点：作为额外的候选写进连接码。
    ///
    /// 路由器肯开的话，对方往这个端点发的报文路由器会直接转进来，不靠打洞 —— 本机这一侧是对称型 NAT 时尤其要紧。
    pub fn set_mapped(&self, mapped: Option<SocketAddr>) {
        let _ = self.commands.send(Command::Mapped(mapped));
    }

    /// 本机这一侧的 NAT 情况。
    pub fn nat(&self) -> watch::Receiver<NatInfo> {
        self.nat.clone()
    }

    /// 等这一轮 STUN 问完，最多等 `limit`。
    pub async fn wait_checked(&self, limit: std::time::Duration) -> NatInfo {
        let mut nat = self.nat.clone();
        let _ = tokio::time::timeout(limit, nat.wait_for(|info| info.checked)).await;
        nat.borrow().clone()
    }

    /// 本机的角色。
    pub fn role(&self) -> Role {
        self.role
    }

    /// 本机的 overlay 地址。
    pub fn ip(&self) -> Ipv4Addr {
        self.ip
    }
}

/// 直连模式的控制面。
pub struct DirectSession {
    node_config: Config,
    welcome: Welcome,
    role: Role,
    stun: Vec<SocketAddr>,
    peers: Vec<DirectPeer>,
    commands: mpsc::UnboundedReceiver<Command>,
    handle: DirectHandle,
    nat: watch::Sender<NatInfo>,
    status: watch::Sender<Status>,
}

impl DirectSession {
    /// 准备好，还没开始收发。
    pub fn new(config: DirectConfig) -> Self {
        // Node 是给 hub 模式写的：协调服务的地址只用来挑本机的局域网地址（往哪个方向出网），
        // 公钥填一个谁也没有私钥的，没有人能冒充它
        let coord = config
            .stun
            .first()
            .copied()
            .unwrap_or_else(|| SocketAddr::from(([1, 1, 1, 1], 53)));
        let nobody = NodeSecret::generate().public_key();
        let node_config = Config {
            secret: config.secret,
            coord,
            coord_key: nobody,
            entry: Entry::Hello { invite: None },
            local_port: config.local_port,
            keepalive: Some(config.keepalive),
            relay_only: false,
            name: String::new(),
            hosting: Hosting::default(),
        };
        let welcome = Welcome {
            overlay_ip: config.ip,
            prefix_len: NETWORK.prefix_len(),
            probe: None,
            relays: Vec::new(),
            network: None,
            created: None,
        };
        let (commands_tx, commands) = mpsc::unbounded_channel();
        let nat = watch::Sender::new(NatInfo::default());
        let handle = DirectHandle {
            commands: commands_tx,
            nat: nat.subscribe(),
            role: config.role,
            ip: config.ip,
        };
        Self {
            node_config,
            welcome,
            role: config.role,
            stun: config.stun,
            peers: config.peers,
            commands,
            handle,
            nat,
            status: watch::Sender::new(Status::default()),
        }
    }

    /// 合成出来的注册信息：本机地址和网段。守护进程据此配虚拟网卡。
    pub fn welcome(&self) -> &Welcome {
        &self.welcome
    }

    /// 改 peer 列表、看 NAT 情况的把手。
    pub fn handle(&self) -> DirectHandle {
        self.handle.clone()
    }

    /// 订阅 peer 的状态（路径、延迟），和 hub 模式的同一个结构。
    pub fn status(&self) -> watch::Receiver<Status> {
        self.status.subscribe()
    }

    /// 运行，直到事件 channel 关闭（数据面停了）。
    pub async fn run(
        mut self,
        dataplane: Arc<dyn DataPlane>,
        mut events: mpsc::UnboundedReceiver<Event>,
    ) -> Result<(), ControlError> {
        let mut node = Node::new(&self.node_config, &self.welcome, dataplane);
        node.stun_servers = self.stun.clone();
        node.eager = true;
        if self.role == Role::Guest {
            node.gateway = Some(IpNet::V4(NETWORK));
        }
        let mut tick = tokio::time::interval(TICK);
        let started = Instant::now();
        // 端口预测的那批地址只在这之前带上
        let mut predict_until = Some(started + PREDICT_FOR);
        node.on_net_map(peer_infos(&self.peers, true), started);

        loop {
            tokio::select! {
                event = events.recv() => match event {
                    Some(event) => drop(node.on_event(event, Instant::now())),
                    None => return Ok(()),
                },
                // 对协调服务要说的话（上报端点、请对方打洞、保活）在这里没有听的人：丢掉。
                // 打洞靠两边各自按码里的端点一直探测
                _ = tick.tick() => {
                    let now = Instant::now();
                    drop(node.on_tick(now));
                    // 预测的端口扫够了：收回，只留码里的端点和学到的
                    if predict_until.is_some_and(|until| now >= until) {
                        predict_until = None;
                        node.on_net_map(peer_infos(&self.peers, false), now);
                    }
                }
                Some(command) = self.commands.recv() => match command {
                    Command::Set(peers) => {
                        self.peers = peers;
                        let now = Instant::now();
                        predict_until = Some(now + PREDICT_FOR);
                        node.on_net_map(peer_infos(&self.peers, true), now);
                    }
                    Command::Mapped(mapped) => {
                        node.hosting.extra_endpoints = mapped.into_iter().collect();
                    }
                },
            }
            node.take_network_changed();
            node.publish(&self.status, true);
            let info = node.nat_info(Instant::now());
            self.nat.send_if_modified(|current| {
                let changed = *current != info;
                if changed {
                    *current = info;
                }
                changed
            });
        }
    }
}

/// `predict`：对称型 NAT 的 peer 把预测的端点也带上
fn peer_infos(peers: &[DirectPeer], predict: bool) -> Vec<PeerInfo> {
    // 同一把钥匙出现两次（同一个人贴了两次回执）只留最后一个
    let mut by_key: HashMap<NodeKey, &DirectPeer> = HashMap::new();
    for peer in peers {
        by_key.insert(peer.key, peer);
    }
    by_key
        .into_values()
        .map(|peer| PeerInfo {
            key: peer.key,
            overlay_ip: peer.ip,
            endpoints: {
                let mut endpoints = peer.endpoints.clone();
                if predict && let Some(hint) = peer.hint {
                    for addr in hint.predict() {
                        if !endpoints.contains(&addr) {
                            endpoints.push(addr);
                        }
                    }
                }
                endpoints
            },
            name: peer.name.clone(),
        })
        .collect()
}
