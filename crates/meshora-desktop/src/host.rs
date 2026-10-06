//! 本机当主机：在客户端进程里跑一个协调服务和一个中继，朋友直接连到这台电脑上。
//!
//! 省掉了一台服务器，代价是这台电脑得让外面连得进来：
//!
//! - 用 UPnP 在路由器上开端口（TCP 7443 协调服务、UDP 7443 端点探测、TCP 7444 中继、UDP 41641 本机的节点），
//!   顺便问到公网地址
//! - 问到的"公网地址"是私网地址或者 `100.64.0.0/10`：这台电脑在运营商级 NAT 后面，外面根本连不进来，
//!   开了端口也没用 —— 界面上明说，推荐改用服务器
//! - 路由器不支持 UPnP：不知道公网地址，网络码里只能写局域网地址，只有同一个局域网的人进得来
//!
//! 只有本机能在这个协调服务上建网络（[`HubConfig::creators`]），别人知道了地址也只能凭网络码加入。
//! 客户端关掉，网络就停了。

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket as StdUdpSocket};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use meshora_coord::{Config, Coordinator, HubConfig, HubLimits};
use meshora_proto::control::RelayInfo;
use meshora_types::NodeKey;
use meshorad::{Hosting, NetworkCode};
use serde::Serialize;
use tokio::net::{TcpListener, UdpSocket};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::natpmp;
use crate::store::Store;

/// 协调服务（TCP）和端点探测（UDP）的端口
pub const COORD_PORT: u16 = 7443;
/// 中继的端口
pub const RELAY_PORT: u16 = 7444;
/// 找路由器、开端口最多等多久
const UPNP_TIMEOUT: Duration = Duration::from_secs(3);
/// 端口映射的租期（秒）。客户端开着就一直在；关掉之后路由器自己会收回
const LEASE: u32 = 24 * 3600;

/// 朋友连不连得进来。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Reach {
    /// 路由器上开好了端口，公网地址也是真的公网地址。
    Open,
    /// 公网地址其实是运营商的内网（运营商级 NAT）：外面连不进来。
    Cgnat,
    /// 路由器不支持 UPnP（或者没开）：不知道公网地址，只有同一个局域网的人进得来。
    Unknown,
}

/// 本机当主机的情况，给界面看。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInfo {
    /// 朋友连不连得进来。
    pub reach: Reach,
    /// 路由器说的公网地址（问不到是 `None`）。
    pub public_ip: Option<String>,
    /// 本机的局域网地址。
    pub lan_ip: String,
}

/// 跑着的主机。丢掉它就停：协调服务、中继停下，路由器上的端口映射删掉。
pub struct Host {
    /// 写进网络码、给朋友用的服务器地址（有公网地址时是公网的）。
    pub share: NetworkCode,
    /// 本机自己连的服务器地址（局域网地址）。
    pub local: NetworkCode,
    /// 本机节点要的额外设置：公网端点、地址别名。
    pub hosting: Hosting,
    /// 给界面看的情况。
    pub info: HostInfo,
    tasks: Vec<JoinHandle<()>>,
    mapped: Option<Arc<igd_next::Gateway>>,
}

impl Drop for Host {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        // 删端口映射是阻塞的网络请求，放到单独的线程里，别卡住调用方
        if let Some(gateway) = self.mapped.take() {
            std::thread::spawn(move || {
                for (protocol, port) in MAPPINGS {
                    let _ = gateway.remove_port(protocol, port);
                }
            });
        }
    }
}

const MAPPINGS: [(igd_next::PortMappingProtocol, u16); 4] = [
    (igd_next::PortMappingProtocol::TCP, COORD_PORT),
    (igd_next::PortMappingProtocol::UDP, COORD_PORT),
    (igd_next::PortMappingProtocol::TCP, RELAY_PORT),
    (igd_next::PortMappingProtocol::UDP, 41641),
];

/// 起一个主机。`dir` 放主机自己的私钥和网络的数据；`owner` 是唯一能在上面建网络的公钥（本机）。
pub async fn start(dir: &Path, owner: NodeKey) -> Result<Host, String> {
    let secret = Store::new(dir)
        .load_or_create_key()
        .map_err(|err| format!("主机的私钥读写失败：{err}"))?;
    let key = secret.public_key();
    let lan = lan_ip().ok_or("找不到本机的局域网地址：这台电脑连着网吗？")?;

    let bind = |port| SocketAddr::from(([0, 0, 0, 0], port));
    let in_use = |what: &str, port: u16, err: io::Error| {
        format!("{what}要用的端口 {port} 打不开（{err}）：是不是还开着 meshora-coord 之类的程序？")
    };
    let listener = TcpListener::bind(bind(COORD_PORT))
        .await
        .map_err(|err| in_use("协调服务", COORD_PORT, err))?;
    let probe = UdpSocket::bind(bind(COORD_PORT))
        .await
        .map_err(|err| in_use("端点探测", COORD_PORT, err))?;
    let relay_listener = TcpListener::bind(bind(RELAY_PORT))
        .await
        .map_err(|err| in_use("中继", RELAY_PORT, err))?;

    // UPnP 是阻塞的：放到线程池里，最多等几秒
    let mapping = tokio::task::spawn_blocking(move || map_ports(lan))
        .await
        .unwrap_or(None);
    let (reach, public, gateway) = match mapping {
        Some((gateway, public)) if shared_or_private(public) => {
            (Reach::Cgnat, Some(public), Some(gateway))
        }
        Some((gateway, public)) => (Reach::Open, Some(public), Some(gateway)),
        None => (Reach::Unknown, None, None),
    };
    // 写进网络码的地址：外面真连得进来才写公网的，不然写局域网的（至少同一个局域网的人进得来）
    let advertised = match (reach, public) {
        (Reach::Open, Some(public)) => public,
        _ => lan,
    };
    info!(?reach, %lan, ?public, "本机当主机");

    let coordinator = Coordinator::new(Config {
        secret: secret.clone(),
        nodes: vec![],
        state: None,
        overlay: "100.64.0.0/10".parse().expect("常量"),
        probe: Some(SocketAddr::new(advertised.into(), COORD_PORT)),
        relays: vec![RelayInfo {
            key,
            addr: SocketAddr::new(advertised.into(), RELAY_PORT),
        }],
        hub: Some(HubConfig {
            dir: dir.join("networks"),
            limits: HubLimits::default(),
            creators: Some(vec![owner]),
        }),
    })
    .map_err(|err| format!("协调服务起不来：{err}"))?;
    let relay = meshora_relay::Config {
        secret,
        allow: coordinator.members(),
        links: Some(coordinator.links()),
        rate: None,
    };
    let tasks = vec![
        tokio::spawn(async move {
            if let Err(err) = meshora_relay::serve(relay, relay_listener).await {
                warn!(%err, "本机的中继停了");
            }
        }),
        tokio::spawn(async move {
            if let Err(err) = coordinator.serve(listener, Some(probe)).await {
                warn!(%err, "本机的协调服务停了");
            }
        }),
    ];

    let share = NetworkCode::new(key, SocketAddr::new(advertised.into(), COORD_PORT));
    let local = NetworkCode::new(key, SocketAddr::new(lan.into(), COORD_PORT));
    let mut hosting = Hosting::default();
    if advertised != lan {
        for port in [COORD_PORT, RELAY_PORT] {
            hosting.aliases.push((
                SocketAddr::new(advertised.into(), port),
                SocketAddr::new(lan.into(), port),
            ));
        }
    }
    if let (Reach::Open, Some(public)) = (reach, public) {
        hosting
            .extra_endpoints
            .push(SocketAddr::new(public.into(), 41641));
    }
    Ok(Host {
        share,
        local,
        hosting,
        info: HostInfo {
            reach,
            public_ip: public.map(|ip| ip.to_string()),
            lan_ip: lan.to_string(),
        },
        tasks,
        mapped: gateway.map(Arc::new),
    })
}

/// 本机的局域网地址：对一个公网地址做一次 UDP connect（不发包），看系统选了哪个本地地址。
/// 选出来的是代理软件的虚拟网卡（Clash、mihomo 的 TUN 是 198.18.0.1）时不能用：路由器不认它，
/// 换成网卡里第一个真实的局域网地址
fn lan_ip() -> Option<Ipv4Addr> {
    lan_interface().map(|(ip, _)| ip)
}

/// 本机真实的局域网地址和它的前缀长度（猜网关用，见 [`natpmp::guess_gateway`]）
fn lan_interface() -> Option<(Ipv4Addr, u8)> {
    let interfaces: Vec<(Ipv4Addr, u8)> = if_addrs::get_if_addrs()
        .ok()?
        .into_iter()
        .filter_map(|interface| match interface.addr {
            if_addrs::IfAddr::V4(v4) => Some((v4.ip, u32::from(v4.netmask).leading_ones() as u8)),
            _ => None,
        })
        .filter(|&(ip, prefix)| real_lan(ip, prefix))
        .collect();
    // 系统往外走的那块网卡是真实的局域网，就用它；不是（被代理接管了）就用第一块真实的
    let routed = StdUdpSocket::bind("0.0.0.0:0")
        .and_then(|socket| socket.connect("1.1.1.1:80").map(|()| socket))
        .and_then(|socket| socket.local_addr())
        .ok()
        .and_then(|addr| match addr.ip() {
            IpAddr::V4(ip) => interfaces.iter().find(|(lan, _)| *lan == ip).copied(),
            IpAddr::V6(_) => None,
        });
    routed.or_else(|| interfaces.first().copied())
}

/// 真实的局域网地址：私网、不是代理的虚拟网卡（198.18.0.0/15）、不是点对点的小网段（/30 以上）
fn real_lan(ip: Ipv4Addr, prefix_len: u8) -> bool {
    let [a, b, ..] = ip.octets();
    ip.is_private() && !(a == 198 && (b & 0xfe) == 18) && prefix_len < 30
}

/// 找路由器、开端口、问公网地址。任何一步不行就是 `None`
fn map_ports(lan: Ipv4Addr) -> Option<(igd_next::Gateway, Ipv4Addr)> {
    let gateway = igd_next::search_gateway(igd_next::SearchOptions {
        // 从真实的局域网网卡发现路由器：默认绑 0.0.0.0，开着代理（Clash 的 TUN）时组播会被发进虚拟网卡
        bind_addr: SocketAddr::V4(SocketAddrV4::new(lan, 0)),
        timeout: Some(UPNP_TIMEOUT),
        single_search_timeout: Some(UPNP_TIMEOUT),
        ..Default::default()
    })
    .map_err(|err| info!(%err, "没找到支持 UPnP 的路由器"))
    .ok()?;
    let public = match gateway.get_external_ip() {
        Ok(IpAddr::V4(ip)) => ip,
        Ok(other) => {
            info!(%other, "路由器给的公网地址不是 IPv4");
            return None;
        }
        Err(err) => {
            info!(%err, "路由器没说公网地址");
            return None;
        }
    };
    for (protocol, port) in MAPPINGS {
        let local = SocketAddr::V4(SocketAddrV4::new(lan, port));
        if let Err(err) = gateway.add_port(protocol, port, local, LEASE, "Meshora") {
            warn!(%err, ?protocol, port, "路由器不让开这个端口");
            return None;
        }
    }
    Some((gateway, public))
}

/// 直连模式在路由器上开的一个 UDP 端口（UPnP 或 NAT-PMP 开的）。丢掉它就撤掉映射。
pub enum UdpMapping {
    /// UPnP 开的
    Upnp {
        /// 路由器
        gateway: Arc<igd_next::Gateway>,
        /// 端口
        port: u16,
    },
    /// PCP 开的
    Pcp {
        /// 网关地址
        gateway: Ipv4Addr,
        /// 本机的局域网地址
        client: Ipv4Addr,
        /// 端口
        port: u16,
        /// 开映射时用的随机数，删的时候要同一个
        nonce: [u8; 12],
    },
    /// NAT-PMP 开的
    NatPmp {
        /// 网关地址
        gateway: Ipv4Addr,
        /// 端口
        port: u16,
    },
}

impl Drop for UdpMapping {
    fn drop(&mut self) {
        // 删端口映射是阻塞的网络请求，放到单独的线程里
        match self {
            Self::Upnp { gateway, port } => {
                let (gateway, port) = (Arc::clone(gateway), *port);
                std::thread::spawn(move || {
                    let _ = gateway.remove_port(igd_next::PortMappingProtocol::UDP, port);
                });
            }
            Self::Pcp {
                gateway,
                client,
                port,
                nonce,
            } => {
                let (gateway, client, port, nonce) = (*gateway, *client, *port, *nonce);
                std::thread::spawn(move || natpmp::pcp_unmap(gateway, client, port, nonce));
            }
            Self::NatPmp { gateway, port } => {
                let (gateway, port) = (*gateway, *port);
                std::thread::spawn(move || natpmp::unmap(gateway, port));
            }
        }
    }
}

/// 直连模式：请路由器把 UDP `port` 映射到这台电脑的同一个端口，交回映射和外面看到的端点。
///
/// 阻塞（找路由器最多 3 秒），在 `spawn_blocking` 里调。路由器不支持、不肯开，
/// 或者路由器自己的公网地址是私网 / 运营商级 NAT 的（开了外面也连不进来）时是 `None`
pub fn map_udp(port: u16) -> Option<(UdpMapping, SocketAddr)> {
    let (lan, prefix) = lan_interface()?;
    map_upnp(lan, port)
        .or_else(|| map_pcp(lan, prefix, port))
        .or_else(|| map_natpmp(lan, prefix, port))
}

/// PCP：NAT-PMP 的后继，新一些的路由器只开了它。同样是往（猜出来的）网关单播
fn map_pcp(lan: Ipv4Addr, prefix: u8, port: u16) -> Option<(UdpMapping, SocketAddr)> {
    let gateway = natpmp::guess_gateway(lan, prefix)?;
    match natpmp::pcp_map(gateway, lan, port, LEASE) {
        Ok((outside, nonce)) => {
            let SocketAddr::V4(v4) = outside else {
                return None;
            };
            if shared_or_private(*v4.ip()) {
                info!(%outside, "直连：路由器自己在别的 NAT 后面（运营商级 NAT），PCP 开了也连不进来");
                natpmp::pcp_unmap(gateway, lan, port, nonce);
                return None;
            }
            info!(%outside, %gateway, "直连：路由器用 PCP 开了端口");
            Some((
                UdpMapping::Pcp {
                    gateway,
                    client: lan,
                    port,
                    nonce,
                },
                outside,
            ))
        }
        Err(err) => {
            info!(%gateway, %err, "直连：PCP 开不了端口");
            None
        }
    }
}

/// NAT-PMP：UPnP 不行时的后备。往（猜出来的）网关单播一个请求，安卓上也能发
fn map_natpmp(lan: Ipv4Addr, prefix: u8, port: u16) -> Option<(UdpMapping, SocketAddr)> {
    let gateway = natpmp::guess_gateway(lan, prefix)?;
    match natpmp::map(gateway, port, LEASE) {
        Ok(outside) => {
            let SocketAddr::V4(v4) = outside else {
                return None;
            };
            if shared_or_private(*v4.ip()) {
                info!(%outside, "直连：路由器自己在别的 NAT 后面（运营商级 NAT），NAT-PMP 开了也连不进来");
                natpmp::unmap(gateway, port);
                return None;
            }
            info!(%outside, %gateway, "直连：路由器用 NAT-PMP 开了端口");
            Some((UdpMapping::NatPmp { gateway, port }, outside))
        }
        Err(err) => {
            info!(%gateway, %err, "直连：NAT-PMP 也开不了端口");
            None
        }
    }
}

/// UPnP：找路由器、问公网地址、开端口
fn map_upnp(lan: Ipv4Addr, port: u16) -> Option<(UdpMapping, SocketAddr)> {
    let gateway = igd_next::search_gateway(igd_next::SearchOptions {
        // 从真实的局域网网卡发现路由器：默认绑 0.0.0.0，开着代理（Clash 的 TUN）时组播会被发进虚拟网卡
        bind_addr: SocketAddr::V4(SocketAddrV4::new(lan, 0)),
        timeout: Some(UPNP_TIMEOUT),
        single_search_timeout: Some(UPNP_TIMEOUT),
        ..Default::default()
    })
    .map_err(|err| info!(%err, "直连：没找到支持 UPnP 的路由器"))
    .ok()?;
    let public = match gateway.get_external_ip() {
        Ok(IpAddr::V4(ip)) if !shared_or_private(ip) => ip,
        Ok(ip) => {
            info!(%ip, "直连：路由器自己在别的 NAT 后面（运营商级 NAT），映射了也连不进来");
            return None;
        }
        Err(err) => {
            info!(%err, "直连：路由器没说公网地址");
            return None;
        }
    };
    let local = SocketAddr::V4(SocketAddrV4::new(lan, port));
    if let Err(err) = gateway.add_port(
        igd_next::PortMappingProtocol::UDP,
        port,
        local,
        LEASE,
        "Meshora",
    ) {
        info!(%err, port, "直连：路由器不让开这个端口");
        return None;
    }
    let outside = SocketAddr::V4(SocketAddrV4::new(public, port));
    info!(%outside, "直连：路由器用 UPnP 开了端口");
    Some((
        UdpMapping::Upnp {
            gateway: Arc::new(gateway),
            port,
        },
        outside,
    ))
}

/// 是不是外面连不进来的地址：私网、运营商级 NAT（100.64.0.0/10）、链路本地
fn shared_or_private(ip: Ipv4Addr) -> bool {
    let cgnat = ip.octets()[0] == 100 && (ip.octets()[1] & 0xc0) == 64;
    ip.is_private() || cgnat || ip.is_link_local() || ip.is_loopback() || ip.is_unspecified()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_adapters_are_not_the_lan() {
        assert!(real_lan(Ipv4Addr::new(192, 168, 1, 127), 24));
        assert!(!real_lan(Ipv4Addr::new(198, 18, 0, 1), 30), "Clash 的 TUN");
        assert!(
            !real_lan(Ipv4Addr::new(172, 30, 221, 100), 32),
            "点对点虚拟网卡"
        );
        assert!(!real_lan(Ipv4Addr::new(119, 123, 186, 54), 24), "公网");
    }

    #[test]
    fn carrier_grade_nat_and_private_addresses_are_not_reachable() {
        for ip in [
            "10.1.2.3",
            "192.168.1.1",
            "172.20.0.1",
            "100.64.0.1",
            "100.127.255.254",
        ] {
            assert!(shared_or_private(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["203.0.113.5", "100.128.0.1", "100.63.255.255", "8.8.8.8"] {
            assert!(!shared_or_private(ip.parse().unwrap()), "{ip}");
        }
    }
}
