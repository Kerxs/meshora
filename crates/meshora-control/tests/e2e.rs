//! 端到端：真的协调服务、真的数据面（boringtun）、真的控制面，都在本机的 TCP/UDP 上跑。
//! 虚拟网卡用 channel 代替，不需要 root。

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroU16;
use std::sync::Arc;
use std::time::Duration;

use meshora_control::{
    AdminHandle, AdminRequest, Config, ControlError, Entry, NameSetter, Session, Status, Welcome,
};
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
        hub: None,
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
        links: None,
        rate: None,
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
    name: NameSetter,
    admin: AdminHandle,
    welcome: Welcome,
    running: tokio::task::JoinHandle<Result<(), ControlError>>,
}

fn config(secret: &NodeSecret, coord: &Coord, local_port: u16) -> Config {
    Config {
        secret: secret.clone(),
        coord: coord.addr,
        coord_key: coord.key,
        entry: Entry::Hello { invite: None },
        local_port,
        keepalive: NonZeroU16::new(25),
        relay_only: false,
        name: String::new(),
        hosting: Default::default(),
    }
}

/// 按守护进程的顺序启动一个节点：先注册拿到地址，再起数据面，最后跑控制面
async fn start_node(secret: &NodeSecret, coord: &Coord, relay_only: bool) -> Node {
    start_named(secret, coord, relay_only, "").await
}

async fn start_named(secret: &NodeSecret, coord: &Coord, relay_only: bool, name: &str) -> Node {
    start_with(
        secret,
        coord,
        relay_only,
        name,
        Entry::Hello { invite: None },
    )
    .await
}

async fn start_with(
    secret: &NodeSecret,
    coord: &Coord,
    relay_only: bool,
    name: &str,
    entry: Entry,
) -> Node {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let local_port = socket.local_addr().unwrap().port();
    let mut config = config(secret, coord, local_port);
    config.relay_only = relay_only;
    config.name = name.into();
    config.entry = entry;
    let session = Session::connect(config).await.unwrap();
    let ip = session.welcome().overlay_ip;
    let welcome = session.welcome().clone();
    let status = session.status();
    let name = session.name_setter();
    let admin = session.admin();

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
    let running = tokio::spawn(session.run(dataplane.clone(), events_rx));
    Node {
        ip,
        tun_in,
        tun_out,
        dataplane,
        status,
        name,
        admin,
        welcome,
        running,
    }
}

/// hub 模式的协调服务（同进程带一个中继，只在同一个网络里转发），数据放在临时目录
async fn start_hub(dir: &std::path::Path) -> Coord {
    let secret = NodeSecret::generate();
    let key = secret.public_key();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let probe = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let relay_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let coordinator = meshora_coord::Coordinator::new(meshora_coord::Config {
        secret: secret.clone(),
        nodes: vec![],
        state: None,
        overlay: "100.64.0.0/10".parse().unwrap(),
        probe: Some(probe.local_addr().unwrap()),
        relays: vec![RelayInfo {
            key,
            addr: relay_listener.local_addr().unwrap(),
        }],
        hub: Some(meshora_coord::HubConfig {
            dir: dir.to_path_buf(),
            limits: meshora_coord::HubLimits::default(),
            creators: None,
        }),
    })
    .unwrap();
    tokio::spawn(meshora_relay::serve(
        meshora_relay::Config {
            secret,
            allow: coordinator.members(),
            links: Some(coordinator.links()),
            rate: None,
        },
        relay_listener,
    ));
    tokio::spawn(coordinator.serve(listener, Some(probe)));
    Coord { addr, key }
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

    // 经中继的探测也通：走中继时状态里有往返时间（控制报文经中继来回）
    for node in [&mut node_a, &mut node_b] {
        let status = tokio::time::timeout(
            CONVERGE,
            node.status
                .wait_for(|s| s.peers.first().is_some_and(|p| p.rtt.is_some())),
        )
        .await
        .expect("走中继时一直没有往返时间")
        .unwrap()
        .clone();
        assert!(matches!(status.peers[0].path, Some(Path::Relay { .. })));
        assert!(status.peers[0].rtt.unwrap() < Duration::from_millis(500));
    }
}

/// 一个可以随时拔掉的 TCP 转发：模拟中继挂了（连着的连接断开，新的也连不上）
struct Cable {
    addr: SocketAddr,
    tasks: Arc<std::sync::Mutex<Vec<tokio::task::AbortHandle>>>,
}

impl Cable {
    async fn to(target: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let tasks = Arc::new(std::sync::Mutex::new(Vec::new()));
        let spawned = Arc::clone(&tasks);
        let accept = tokio::spawn(async move {
            while let Ok((mut inbound, _)) = listener.accept().await {
                let task = tokio::spawn(async move {
                    if let Ok(mut outbound) = tokio::net::TcpStream::connect(target).await {
                        let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                    }
                });
                spawned.lock().unwrap().push(task.abort_handle());
            }
        });
        tasks.lock().unwrap().push(accept.abort_handle());
        Self { addr, tasks }
    }

    fn cut(&self) {
        for task in self.tasks.lock().unwrap().drain(..) {
            task.abort();
        }
    }
}

#[tokio::test]
async fn a_dead_relay_is_replaced_by_the_next_one() {
    let a = NodeSecret::generate();
    let b = NodeSecret::generate();
    // 两个中继都接在可以拔掉的线上，拔掉正在用的那个
    let mut relays = Vec::new();
    for _ in 0..2 {
        let relay = start_relay(&[&a, &b]).await;
        let cable = Cable::to(relay.addr).await;
        let info = RelayInfo {
            key: relay.key,
            addr: cable.addr,
        };
        relays.push((info, cable));
    }
    let infos = relays.iter().map(|(info, _)| info.clone()).collect();
    let coord = start_coord(&[&a, &b], infos).await;
    let mut node_a = start_node(&a, &coord, true).await;
    let mut node_b = start_node(&b, &coord, true).await;

    let via = |relay: &RelayInfo| Path::Relay {
        relay: relay.key,
        addr: relay.addr,
    };
    let received = deliver(&node_a, &mut node_b, b"first relay").await;
    assert_eq!(&received[20..], b"first relay");
    // 两个中继差不多快，走哪个看谁先测出来；两边各自选路，也不一定是同一个。拔掉 A 在用的那个
    let used = node_a.dataplane.status()[0].path;
    let (dead, alive) = if used == Some(via(&relays[0].0)) {
        (&relays[0], &relays[1])
    } else {
        (&relays[1], &relays[0])
    };
    let second = alive.0.clone();

    dead.1.cut();
    let cut = std::time::Instant::now();
    for node in [&mut node_a, &mut node_b] {
        tokio::time::timeout(
            Duration::from_secs(10),
            node.status.wait_for(|s| {
                s.peers
                    .first()
                    .is_some_and(|p| p.path == Some(via(&second)))
            }),
        )
        .await
        .expect("中继挂了，一直没换到另一个")
        .unwrap();
    }
    // 正在用的中继每秒探测，连丢两次就换：几秒之内，不是等中继连接自己超时（25 秒）
    assert!(
        cut.elapsed() < Duration::from_secs(8),
        "{:?}",
        cut.elapsed()
    );
    let received = deliver(&node_a, &mut node_b, b"second relay").await;
    assert_eq!(&received[20..], b"second relay");
    let received = deliver(&node_b, &mut node_a, b"and back").await;
    assert_eq!(&received[20..], b"and back");
}

/// 等到 `node` 看到的第一个 peer 叫 `expected`
async fn wait_name(node: &mut Node, expected: &str) {
    tokio::time::timeout(
        CONVERGE,
        node.status
            .wait_for(|s| s.peers.first().is_some_and(|p| p.name == expected)),
    )
    .await
    .unwrap_or_else(|_| panic!("一直没看到名字 {expected:?}"))
    .unwrap();
}

#[tokio::test]
async fn names_reach_the_other_side_and_can_change_while_connected() {
    let a = NodeSecret::generate();
    let b = NodeSecret::generate();
    let coord = start_coord(&[&a, &b], vec![]).await;
    let node_a = start_named(&a, &coord, false, "阿杰的台式机").await;
    let mut node_b = start_named(&b, &coord, false, "小明").await;

    wait_name(&mut node_b, "阿杰的台式机").await;
    // 连着改名，不用重连
    node_a.name.set("阿杰");
    wait_name(&mut node_b, "阿杰").await;
}

#[tokio::test]
async fn a_network_is_created_on_a_hub_joined_managed_and_left() {
    let dir = std::env::temp_dir().join(format!("meshora-control-hub-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let coord = start_hub(&dir).await;
    let (a, b) = (NodeSecret::generate(), NodeSecret::generate());

    // A 建网：拿到网络 ID 和邀请码，自己是第一个地址
    let mut node_a = start_with(
        &a,
        &coord,
        false,
        "阿杰",
        Entry::Create {
            name: "周末开黑".into(),
        },
    )
    .await;
    let network = node_a.welcome.network.expect("建好的网络有 ID");
    let invite = node_a.welcome.created.expect("建好的网络有邀请码");
    assert_eq!(node_a.ip, Ipv4Addr::new(100, 64, 0, 1));

    // B 凭网络码加入，两边互通
    let mut node_b = start_with(
        &b,
        &coord,
        false,
        "小明",
        Entry::Join {
            network,
            invite: Some(invite),
        },
    )
    .await;
    assert_eq!(node_b.ip, Ipv4Addr::new(100, 64, 0, 2));
    let received = deliver(&node_a, &mut node_b, b"hello from the owner").await;
    assert_eq!(&received[20..], b"hello from the owner");
    let received = deliver(&node_b, &mut node_a, b"and back").await;
    assert_eq!(&received[20..], b"and back");

    // A 是网主：看得到两个人的成员清单
    let mut roster = node_a.admin.roster();
    let list = tokio::time::timeout(
        CONVERGE,
        roster.wait_for(|r| r.as_ref().is_some_and(|r| r.members.len() == 2)),
    )
    .await
    .expect("网主一直没收到两个人的成员清单")
    .unwrap()
    .clone()
    .unwrap();
    assert_eq!(list.name, "周末开黑");
    assert!(
        list.members
            .iter()
            .any(|m| m.key == b.public_key() && m.name == "小明")
    );
    // B 不是网主：没有清单，管理请求被拒
    assert!(node_b.admin.roster().borrow().is_none());
    assert!(node_b.admin.request(AdminRequest::Delete).await.is_err());

    // 网主新建一个一次性邀请码
    let once = node_a
        .admin
        .request(AdminRequest::NewInvite {
            uses: Some(1),
            hours: None,
        })
        .await
        .unwrap();
    assert!(once.is_some());

    // 网主把 B 移出：B 的控制面停下，原因是"被移出"
    node_a
        .admin
        .request(AdminRequest::Kick {
            member: b.public_key(),
        })
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(10), &mut node_b.running)
        .await
        .expect("被移出之后控制面一直没停")
        .unwrap();
    assert!(
        matches!(&result, Err(ControlError::Rejected(reason)) if reason.contains("移出")),
        "{result:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
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
        hub: None,
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
        entry: Entry::Hello { invite },
        local_port,
        keepalive: NonZeroU16::new(25),
        relay_only: false,
        name: String::new(),
        hosting: Default::default(),
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

/// 对一台真的 hub（比如刚部署好的官方服务器）走一遍：建网、凭网络码加入、经它的中继互发报文、
/// 网主看得到名单，最后解散，服务器上不留东西。两个节点都只走中继：本机的 UDP 绑在回环上，
/// 测的就是服务器的协调服务和中继。
///
/// ```text
/// MESHORA_HUB=<服务器公钥>@<地址>:7443 cargo test -p meshora-control --test e2e -- --ignored a_real_hub
/// ```
#[tokio::test]
#[ignore = "要一台真的 hub：设 MESHORA_HUB=公钥@地址:端口"]
async fn a_real_hub_creates_relays_and_deletes() {
    let hub = std::env::var("MESHORA_HUB").expect("设 MESHORA_HUB=公钥@地址:端口");
    let (key, addr) = hub
        .split_once('@')
        .expect("MESHORA_HUB 的格式是 公钥@地址:端口");
    let coord = Coord {
        addr: addr.parse().expect("地址:端口"),
        key: key.parse().expect("服务器公钥"),
    };
    let (a, b) = (NodeSecret::generate(), NodeSecret::generate());

    let mut node_a = start_with(
        &a,
        &coord,
        true,
        "冒烟测试 A",
        Entry::Create {
            name: "冒烟测试".into(),
        },
    )
    .await;
    let network = node_a.welcome.network.expect("建好的网络有 ID");
    let invite = node_a.welcome.created.expect("建好的网络有邀请码");
    let mut node_b = start_with(
        &b,
        &coord,
        true,
        "冒烟测试 B",
        Entry::Join {
            network,
            invite: Some(invite),
        },
    )
    .await;

    let received = deliver(&node_a, &mut node_b, b"over the real relay").await;
    assert_eq!(&received[20..], b"over the real relay");
    let received = deliver(&node_b, &mut node_a, b"and back").await;
    assert_eq!(&received[20..], b"and back");

    let mut roster = node_a.admin.roster();
    tokio::time::timeout(
        CONVERGE,
        roster.wait_for(|r| r.as_ref().is_some_and(|r| r.members.len() == 2)),
    )
    .await
    .expect("网主没收到两个人的名单")
    .unwrap();

    node_a
        .admin
        .request(AdminRequest::Delete)
        .await
        .expect("解散网络");
}
