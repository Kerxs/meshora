//! 端到端：直连模式，零服务器。一个本机的假 STUN 服务器、真的数据面、真的控制面，
//! 三个节点（房主和两位朋友）照客户端的流程交换房主码和回执码，然后互相发报文。
//! 虚拟网卡用 channel 代替，不需要 root。

use std::net::{Ipv4Addr, SocketAddr};
use std::num::NonZeroU16;
use std::sync::Arc;
use std::time::Duration;

use meshora_control::direct::{
    self, DirectConfig, DirectHandle, DirectPeer, DirectSession, HOST_IP, Offer, Reply, Role,
};
use meshora_types::NodeSecret;
use meshora_wg::{TunChannels, UserspaceDataPlane};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

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
    let (tun_in, from_tun) = mpsc::channel(64);
    let (to_tun, tun_out) = mpsc::channel(64);
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let sink = move |event| {
        let _ = events_tx.send(event);
    };
    let dataplane = Arc::new(
        UserspaceDataPlane::start(
            &secret,
            socket,
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
