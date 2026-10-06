//! 端到端：直连模式，零服务器。一个本机的假 STUN 服务器、真的数据面、真的控制面，
//! 三个节点（房主和两位朋友）照客户端的流程交换房主码和回执码，然后互相发报文。
//! 虚拟网卡用 channel 代替，不需要 root。

use std::net::{Ipv4Addr, SocketAddr};
use std::num::NonZeroU16;
use std::sync::Arc;
use std::time::Duration;

use meshora_control::Status;
use meshora_control::direct::{
    self, DirectConfig, DirectHandle, DirectPeer, DirectSession, HOST_IP, Offer, Reply, Role,
};
use meshora_types::NodeSecret;
use meshora_wg::{TunChannels, UserspaceDataPlane};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, watch};

const CONVERGE: Duration = Duration::from_secs(20);

/// 假的 STUN 服务器：对每个 Binding 请求回一个 XOR-MAPPED-ADDRESS，内容是请求的来源地址
async fn fake_stun() -> SocketAddr {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 512];
        loop {
            let Ok((len, from)) = socket.recv_from(&mut buf).await else {
                continue;
            };
            if len < 20 {
                continue;
            }
            let SocketAddr::V4(from4) = from else {
                continue;
            };
            let cookie = [0x21, 0x12, 0xA4, 0x42];
            let mut out = vec![0x01, 0x01, 0x00, 0x0C];
            out.extend_from_slice(&cookie);
            out.extend_from_slice(&buf[8..20]);
            out.extend_from_slice(&[0x00, 0x20, 0x00, 0x08, 0x00, 0x01]);
            let port = from4.port() ^ 0x2112;
            out.extend_from_slice(&port.to_be_bytes());
            let ip = from4.ip().octets();
            out.extend((0..4).map(|i| ip[i] ^ cookie[i]));
            let _ = socket.send_to(&out, from).await;
        }
    });
    addr
}

struct Node {
    secret: NodeSecret,
    ip: Ipv4Addr,
    handle: DirectHandle,
    status: watch::Receiver<Status>,
    tun_in: mpsc::Sender<Vec<u8>>,
    tun_out: mpsc::Receiver<Vec<u8>>,
    _dataplane: Arc<UserspaceDataPlane>,
}

async fn start(
    secret: NodeSecret,
    ip: Ipv4Addr,
    role: Role,
    peers: Vec<DirectPeer>,
    stun: SocketAddr,
) -> Node {
    start_with(secret, ip, role, peers, stun, None).await
}

/// `v6` 是只收发 IPv6 的第二个 socket（数据面按地址类型选用）
async fn start_with(
    secret: NodeSecret,
    ip: Ipv4Addr,
    role: Role,
    peers: Vec<DirectPeer>,
    stun: SocketAddr,
    v6: Option<std::net::UdpSocket>,
) -> Node {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let local_port = socket.local_addr().unwrap().port();
    let session = DirectSession::new(DirectConfig {
        secret: secret.clone(),
        local_port,
        keepalive: NonZeroU16::new(25).unwrap(),
        ip,
        role,
        stun: vec![stun],
        peers,
    });
    let handle = session.handle();
    let status = session.status();
    let (tun_in, from_tun) = mpsc::channel(64);
    let (to_tun, tun_out) = mpsc::channel(64);
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let sink = move |event| {
        let _ = events_tx.send(event);
    };
    let dataplane = Arc::new(
        UserspaceDataPlane::start_dual(
            &secret,
            socket,
            v6,
            TunChannels { from_tun, to_tun },
            Arc::new(sink),
        )
        .unwrap(),
    );
    dataplane.set_lan(ip, direct::NETWORK.prefix_len());
    dataplane.set_forwarding(role == Role::Host);
    tokio::spawn(session.run(dataplane.clone(), events_rx));
    Node {
        secret,
        ip,
        handle,
        status,
        tun_in,
        tun_out,
        _dataplane: dataplane,
    }
}

fn ipv4(src: Ipv4Addr, dst: Ipv4Addr, payload: &[u8]) -> Vec<u8> {
    let total = 20 + payload.len();
    let mut packet = vec![0u8; total];
    packet[0] = 0x45;
    packet[2..4].copy_from_slice(&(total as u16).to_be_bytes());
    packet[8] = 64;
    packet[9] = 1;
    packet[12..16].copy_from_slice(&src.octets());
    packet[16..20].copy_from_slice(&dst.octets());
    packet[20..].copy_from_slice(payload);
    packet
}

/// 反复从 `from` 发往 `dst`，直到 `to` 收到为止
async fn deliver(from: &Node, dst: Ipv4Addr, to: &mut Node, payload: &[u8]) -> Vec<u8> {
    let packet = ipv4(from.ip, dst, payload);
    tokio::time::timeout(CONVERGE, async {
        loop {
            from.tun_in.send(packet.clone()).await.unwrap();
            if let Ok(Some(received)) =
                tokio::time::timeout(Duration::from_millis(300), to.tun_out.recv()).await
            {
                return received;
            }
        }
    })
    .await
    .expect("没能打通")
}

/// 房主给一位朋友出房主码，朋友读码、起节点、回回执码，房主读回执、加上这位朋友。
/// 返回朋友的节点和房主现在认识的全部朋友
async fn invite(
    host: &Node,
    guest_ip: Ipv4Addr,
    stun: SocketAddr,
    guests: &mut Vec<DirectPeer>,
) -> Node {
    let nat = host.handle.wait_checked(Duration::from_secs(5)).await;
    assert!(
        nat.checked && nat.public.is_some(),
        "房主问到了自己的公网端点"
    );
    let offer_text = Offer {
        host: host.secret.public_key(),
        host_ip: host.ip,
        guest_ip,
        prefix_len: direct::NETWORK.prefix_len(),
        name: "房主".into(),
        endpoints: nat.endpoints,
        hint: nat.hint,
        expires: direct::expires_from_now(),
    }
    .encode();

    // 朋友这边：只拿到一段文字
    let offer = Offer::decode(&offer_text).unwrap();
    let guest_secret = NodeSecret::generate();
    let host_peer = DirectPeer {
        key: offer.host,
        ip: offer.host_ip,
        name: offer.name.clone(),
        endpoints: offer.endpoints.clone(),
        hint: offer.hint,
    };
    let guest = start(
        guest_secret.clone(),
        offer.guest_ip,
        Role::Guest,
        vec![host_peer],
        stun,
    )
    .await;
    let guest_nat = guest.handle.wait_checked(Duration::from_secs(5)).await;
    let reply_text = Reply {
        guest: guest_secret.public_key(),
        host: offer.host,
        guest_ip: offer.guest_ip,
        name: "朋友".into(),
        endpoints: guest_nat.endpoints,
        hint: guest_nat.hint,
        expires: direct::expires_from_now(),
    }
    .encode();

    // 房主这边：读回执，加进名单
    let reply = Reply::decode(&reply_text).unwrap();
    assert_eq!(reply.host, host.secret.public_key());
    guests.push(DirectPeer {
        key: reply.guest,
        ip: reply.guest_ip,
        name: reply.name,
        endpoints: reply.endpoints,
        hint: None,
    });
    host.handle.set_peers(guests.clone());
    guest
}

#[tokio::test]
async fn two_friends_connect_through_codes_alone() {
    let stun = fake_stun().await;
    let mut host = start(NodeSecret::generate(), HOST_IP, Role::Host, vec![], stun).await;
    let mut guests = Vec::new();
    let mut b = invite(&host, Ipv4Addr::new(100, 96, 0, 2), stun, &mut guests).await;

    assert_eq!(
        deliver(&b, host.ip, &mut host, b"hello host").await[20..],
        *b"hello host"
    );
    assert_eq!(
        deliver(&host, b.ip, &mut b, b"hello guest").await[20..],
        *b"hello guest"
    );
}

#[tokio::test]
async fn friends_reach_each_other_through_the_host() {
    let stun = fake_stun().await;
    let host = start(NodeSecret::generate(), HOST_IP, Role::Host, vec![], stun).await;
    let mut guests = Vec::new();
    let b = invite(&host, Ipv4Addr::new(100, 96, 0, 2), stun, &mut guests).await;
    let mut c = invite(&host, Ipv4Addr::new(100, 96, 0, 3), stun, &mut guests).await;

    // B 和 C 没交换过码，报文经房主转发
    let got = deliver(&b, c.ip, &mut c, b"via host").await;
    assert_eq!(&got[12..16], &b.ip.octets(), "源地址还是 B");
    assert_eq!(got[20..], *b"via host");
}

/// 码里的端点全是错的：一个谁也没在听的本机端口
fn nowhere() -> SocketAddr {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.local_addr().unwrap()
}

#[tokio::test]
async fn a_wrong_address_in_one_code_is_learned_from_the_other_side() {
    // 端口受限型 NAT 上常见：码里的端口不对，但对方的 Ping 从真实的端口来，学到它就通了
    let stun = fake_stun().await;
    let host_secret = NodeSecret::generate();
    let guest_secret = NodeSecret::generate();
    let mut host = start(host_secret.clone(), HOST_IP, Role::Host, vec![], stun).await;
    let host_nat = host.handle.wait_checked(Duration::from_secs(5)).await;
    let guest_ip = Ipv4Addr::new(100, 96, 0, 2);
    // 朋友拿到的房主码里，房主的端点是错的
    let guest = start(
        guest_secret.clone(),
        guest_ip,
        Role::Guest,
        vec![DirectPeer {
            key: host_secret.public_key(),
            ip: HOST_IP,
            name: String::new(),
            endpoints: vec![nowhere()],
            hint: None,
        }],
        stun,
    )
    .await;
    assert!(!host_nat.endpoints.is_empty());
    let guest_nat = guest.handle.wait_checked(Duration::from_secs(5)).await;
    host.handle.set_peers(vec![DirectPeer {
        key: guest_secret.public_key(),
        ip: guest_ip,
        name: String::new(),
        endpoints: guest_nat.endpoints,
        hint: None,
    }]);
    assert_eq!(
        deliver(&guest, HOST_IP, &mut host, b"learned").await[20..],
        *b"learned"
    );
}

#[tokio::test]
async fn diagnostics_say_nothing_arrived_when_both_codes_are_wrong() {
    let stun = fake_stun().await;
    let host_secret = NodeSecret::generate();
    let guest_secret = NodeSecret::generate();
    let mut host = start(host_secret.clone(), HOST_IP, Role::Host, vec![], stun).await;
    let guest_ip = Ipv4Addr::new(100, 96, 0, 2);
    let _guest = start(
        guest_secret.clone(),
        guest_ip,
        Role::Guest,
        vec![DirectPeer {
            key: host_secret.public_key(),
            ip: HOST_IP,
            name: String::new(),
            endpoints: vec![nowhere()],
            hint: None,
        }],
        stun,
    )
    .await;
    let wrong = nowhere();
    host.handle.set_peers(vec![DirectPeer {
        key: guest_secret.public_key(),
        ip: guest_ip,
        name: String::new(),
        endpoints: vec![wrong],
        hint: None,
    }]);
    // 等几轮探测
    tokio::time::sleep(Duration::from_secs(3)).await;
    let status = host.status.borrow_and_update().clone();
    let peer = &status.peers[0];
    assert!(!peer.heard, "对方的报文一次都没到");
    assert_eq!(peer.path, None, "没有直连可走");
    assert!(
        peer.candidates.contains(&wrong),
        "诊断里列出试过的地址：{:?}",
        peer.candidates
    );
}

#[tokio::test]
async fn friends_connect_over_ipv6_alone() {
    // 码里只有 IPv6 地址：数据面的 IPv6 socket 收发，探测、握手、数据都走它
    let Ok(host6) = std::net::UdpSocket::bind("[::1]:0") else {
        eprintln!("本机没有 IPv6 回环，跳过");
        return;
    };
    let guest6 = std::net::UdpSocket::bind("[::1]:0").unwrap();
    let (host_at, guest_at) = (host6.local_addr().unwrap(), guest6.local_addr().unwrap());
    let stun = fake_stun().await;
    let host_secret = NodeSecret::generate();
    let guest_secret = NodeSecret::generate();
    let guest_ip = Ipv4Addr::new(100, 96, 0, 2);
    let mut host = start_with(
        host_secret.clone(),
        HOST_IP,
        Role::Host,
        vec![],
        stun,
        Some(host6),
    )
    .await;
    let guest = start_with(
        guest_secret.clone(),
        guest_ip,
        Role::Guest,
        vec![DirectPeer {
            key: host_secret.public_key(),
            ip: HOST_IP,
            name: String::new(),
            endpoints: vec![host_at],
            hint: None,
        }],
        stun,
        Some(guest6),
    )
    .await;
    host.handle.set_peers(vec![DirectPeer {
        key: guest_secret.public_key(),
        ip: guest_ip,
        name: String::new(),
        endpoints: vec![guest_at],
        hint: None,
    }]);
    assert_eq!(
        deliver(&guest, HOST_IP, &mut host, b"over v6").await[20..],
        *b"over v6"
    );
    let status = host.status.borrow().clone();
    assert!(
        matches!(status.peers[0].path, Some(meshora_types::Path::Direct(addr)) if addr.is_ipv6()),
        "走的是 IPv6 直连：{:?}",
        status.peers[0].path
    );
}
