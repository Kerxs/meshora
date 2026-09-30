//! 端到端：真的协调服务、真的数据面（boringtun）、真的控制面，都在本机的 TCP/UDP 上跑。
//! 虚拟网卡用 channel 代替，不需要 root。

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroU16;
use std::sync::Arc;
use std::time::Duration;

use meshora_control::{Config, ControlError, Session, Status};
use meshora_dataplane::DataPlane;
use meshora_proto::control::RelayInfo;
use meshora_types::{NodeKey, NodeSecret, Path};
use meshora_wg::{TunChannels, UserspaceDataPlane};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::{mpsc, watch};

/// 路径收敛要经过上报端点、NetMap、几轮探测、WireGuard 握手 —— 给足时间
const CONVERGE: Duration = Duration::from_secs(20);

struct Coord {
    addr: SocketAddr,
    key: NodeKey,
}

async fn start_coord(nodes: &[&NodeSecret], relays: Vec<RelayInfo>) -> Coord {
    let secret = NodeSecret::generate();
    let key = secret.public_key();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let probe = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let config = meshora_coord::Config {
        secret,
        nodes: nodes.iter().map(|n| n.public_key()).collect(),
        state: None,
        overlay: "100.64.0.0/10".parse().unwrap(),
        probe: Some(probe.local_addr().unwrap()),
        relays,
    };
    tokio::spawn(meshora_coord::serve(config, listener, Some(probe)));
    Coord { addr, key }
}

async fn start_relay(nodes: &[&NodeSecret]) -> RelayInfo {
    let secret = NodeSecret::generate();
    let key = secret.public_key();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let config = meshora_relay::Config {
        secret,
        allow: meshora_relay::allow_list(nodes.iter().map(|n| n.public_key()).collect()),
    };
    tokio::spawn(meshora_relay::serve(config, listener));
    RelayInfo { key, addr }
}

struct Node {
    ip: Ipv4Addr,
    tun_in: mpsc::Sender<Vec<u8>>,
    tun_out: mpsc::Receiver<Vec<u8>>,
    dataplane: Arc<UserspaceDataPlane>,
    status: watch::Receiver<Status>,
}

fn config(secret: &NodeSecret, coord: &Coord, local_port: u16) -> Config {
    Config {
        secret: secret.clone(),
        coord: coord.addr,
        coord_key: coord.key,
        invite: None,
        local_port,
        keepalive: NonZeroU16::new(25),
        relay_only: false,
    }
}

/// 按守护进程的顺序启动一个节点：先注册拿到地址，再起数据面，最后跑控制面
async fn start_node(secret: &NodeSecret, coord: &Coord, relay_only: bool) -> Node {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let local_port = socket.local_addr().unwrap().port();
    let mut config = config(secret, coord, local_port);
    config.relay_only = relay_only;
    let session = Session::connect(config).await.unwrap();
    let ip = session.welcome().overlay_ip;
    let status = session.status();

    let (tun_in, from_tun) = mpsc::channel(64);
    let (to_tun, tun_out) = mpsc::channel(64);
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let sink = move |event| {
        let _ = events_tx.send(event);
    };
    let dataplane = Arc::new(
        UserspaceDataPlane::start(
            secret,
            socket,
            TunChannels { from_tun, to_tun },
            Arc::new(sink),
        )
        .unwrap(),
    );
    tokio::spawn(session.run(dataplane.clone(), events_rx));
    Node {
        ip,
        tun_in,
        tun_out,
        dataplane,
        status,
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

/// 反复从 `from` 发，直到 `to` 收到为止：路径没收敛之前报文会被丢掉
async fn deliver(from: &Node, to: &mut Node, payload: &[u8]) -> Vec<u8> {
    let packet = ipv4(from.ip, to.ip, payload);
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
    .expect("两个节点没能打通")
}

#[tokio::test]
async fn two_nodes_find_each_other_and_talk_directly() {
    let a = NodeSecret::generate();
    let b = NodeSecret::generate();
    let coord = start_coord(&[&a, &b], vec![]).await;
    let mut node_a = start_node(&a, &coord, false).await;
    let mut node_b = start_node(&b, &coord, false).await;
    assert_eq!(node_a.ip, Ipv4Addr::new(100, 64, 0, 1));
    assert_eq!(node_b.ip, Ipv4Addr::new(100, 64, 0, 2));

    let payload = b"hello over meshora";
    let received = deliver(&node_a, &mut node_b, payload).await;
    assert_eq!(&received[20..], payload);
    let received = deliver(&node_b, &mut node_a, b"and back").await;
    assert_eq!(&received[20..], b"and back");

    // 没有中继，打通靠的是直连
    for node in [&node_a, &node_b] {
        let status = node.dataplane.status();
        assert_eq!(status.len(), 1);
        assert!(
            matches!(status[0].path, Some(Path::Direct(addr)) if addr.ip() == IpAddr::from([127, 0, 0, 1]))
        );
        assert!(status[0].last_handshake.is_some());
    }

    // 界面看到的：对方的地址、走的直连、测到的延迟
    let (ip_a, ip_b) = (node_a.ip, node_b.ip);
    for (node, other) in [(&mut node_a, ip_b), (&mut node_b, ip_a)] {
        let status = tokio::time::timeout(
            CONVERGE,
            node.status
                .wait_for(|s| s.peers.first().is_some_and(|p| p.rtt.is_some())),
        )
        .await
        .expect("状态里一直没有延迟")
        .unwrap()
        .clone();
        assert!(status.coord_connected);
        assert_eq!(status.peers.len(), 1);
        assert_eq!(status.peers[0].overlay_ip, other);
        assert!(matches!(status.peers[0].path, Some(Path::Direct(_))));

        // 本机回环上的往返时间是微秒级。曾经把 select 空等的那段也算进了往返时间：
        // 流量停下来之后，节拍发出的下一轮探测量出来是几百毫秒。所以等到下一轮（每 3 秒一轮）再看
        let first = status.peers[0].rtt;
        let later = tokio::time::timeout(
            Duration::from_secs(10),
            node.status
                .wait_for(|s| s.peers.first().is_some_and(|p| p.rtt != first)),
        )
        .await
        .expect("一直没有第二次测量")
        .unwrap()
        .clone();
        let rtt = later.peers[0].rtt.unwrap();
        assert!(
            rtt < Duration::from_millis(100),
            "回环上的往返时间是 {rtt:?}"
        );
    }
}

#[tokio::test]
async fn relay_carries_the_traffic_when_direct_is_off() {
    let a = NodeSecret::generate();
    let b = NodeSecret::generate();
    let relay = start_relay(&[&a, &b]).await;
    let coord = start_coord(&[&a, &b], vec![relay.clone()]).await;
    let mut node_a = start_node(&a, &coord, true).await;
    let mut node_b = start_node(&b, &coord, true).await;

    let received = deliver(&node_a, &mut node_b, b"through the relay").await;
    assert_eq!(&received[20..], b"through the relay");
    let received = deliver(&node_b, &mut node_a, b"and back").await;
    assert_eq!(&received[20..], b"and back");

    for node in [&node_a, &node_b] {
        let status = node.dataplane.status();
        assert_eq!(
            status[0].path,
            Some(Path::Relay {
                relay: relay.key,
                addr: relay.addr
            })
        );
        assert!(status[0].last_handshake.is_some());
    }
}

#[tokio::test]
async fn a_node_outside_the_member_list_is_turned_away() {
    let a = NodeSecret::generate();
    let stranger = NodeSecret::generate();
    let coord = start_coord(&[&a], vec![]).await;
    let result = Session::connect(config(&stranger, &coord, 41641)).await;
    assert!(matches!(result, Err(ControlError::Rejected(_))));
}

#[tokio::test]
async fn a_wrong_coordinator_key_is_refused() {
    let a = NodeSecret::generate();
    let mut coord = start_coord(&[&a], vec![]).await;
    // 以为自己在和协调服务说话，其实公钥对不上：握手失败，不会把自己交给冒充者
    coord.key = NodeSecret::generate().public_key();
    let result = Session::connect(config(&a, &coord, 41641)).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn a_member_removed_from_the_state_file_is_told_and_stops() {
    let dir = std::env::temp_dir().join(format!("meshora-control-kick-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let state = dir.join("coord.state");

    let coord_secret = NodeSecret::generate();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let coordinator = meshora_coord::Coordinator::new(meshora_coord::Config {
        secret: coord_secret.clone(),
        nodes: vec![],
        state: Some(state.clone()),
        overlay: "100.64.0.0/10".parse().unwrap(),
        probe: None,
        relays: vec![],
    })
    .unwrap();
    let invite = coordinator.invite();
    tokio::spawn(coordinator.serve(listener, None));

    let friend = NodeSecret::generate();
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let local_port = socket.local_addr().unwrap().port();
    let session = Session::connect(Config {
        secret: friend.clone(),
        coord: addr,
        coord_key: coord_secret.public_key(),
        invite,
        local_port,
        keepalive: NonZeroU16::new(25),
        relay_only: false,
    })
    .await
    .expect("凭邀请码加入");
    let (_tun_in, from_tun) = mpsc::channel(8);
    let (to_tun, _tun_out) = mpsc::channel(8);
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let sink = move |event| {
        let _ = events_tx.send(event);
    };
    let dataplane = Arc::new(
        UserspaceDataPlane::start(
            &friend,
            socket,
            TunChannels { from_tun, to_tun },
            Arc::new(sink),
        )
        .unwrap(),
    );
    let running = tokio::spawn(session.run(dataplane, events_rx));

    // 网络主人从状态文件里删掉它（邀请码没换）
    let key = friend.public_key().to_string();
    let text: String = std::fs::read_to_string(&state)
        .unwrap()
        .lines()
        .filter(|line| !line.contains(&key))
        .map(|line| format!("{line}\n"))
        .collect();
    std::fs::write(&state, text).unwrap();

    // 控制面停下、说明原因，而不是拿着还有效的邀请码重连回去
    let result = tokio::time::timeout(Duration::from_secs(10), running)
        .await
        .expect("被移出之后控制面一直没停")
        .unwrap();
    match result {
        Err(ControlError::Rejected(reason)) => assert!(reason.contains("移出"), "{reason}"),
        other => panic!("应该是被移出，结果是 {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}
