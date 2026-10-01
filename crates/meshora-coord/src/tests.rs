use std::time::Duration;

use std::net::Ipv4Addr;
use std::time::UNIX_EPOCH;

use meshora_proto::control::PeerInfo;
use meshora_proto::noise::{NoiseReader, NoiseWriter};
use meshora_types::NetworkId;

use super::*;

const WAIT: Duration = Duration::from_secs(5);

struct Coord {
    addr: SocketAddr,
    probe: SocketAddr,
    key: NodeKey,
}

async fn start(nodes: &[&NodeSecret]) -> Coord {
    let secret = NodeSecret::generate();
    let key = secret.public_key();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let probe_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let probe = probe_socket.local_addr().unwrap();
    let config = Config {
        secret,
        nodes: nodes.iter().map(|n| n.public_key()).collect(),
        state: None,
        overlay: "100.64.0.0/10".parse().unwrap(),
        probe: Some(probe),
        relays: vec![],
        hub: None,
    };
    tokio::spawn(serve(config, listener, Some(probe_socket)));
    Coord { addr, probe, key }
}

type Client = (NoiseReader<TcpStream>, NoiseWriter<TcpStream>);

async fn connect(coord: &Coord, secret: &NodeSecret) -> Client {
    let tcp = TcpStream::connect(coord.addr).await.unwrap();
    NoiseStream::connect(tcp, Channel::Control, secret, &coord.key)
        .await
        .unwrap()
        .into_split()
}

async fn send(client: &mut Client, message: ClientMessage) {
    client.1.send(&message.encode()).await.unwrap();
}

async fn recv(client: &mut Client) -> ServerMessage {
    let bytes = tokio::time::timeout(WAIT, client.0.recv())
        .await
        .expect("等消息超时")
        .expect("连接断了");
    ServerMessage::decode(&bytes).unwrap()
}

/// 连上、打招呼、收下 Welcome 和第一份 NetMap
async fn join(coord: &Coord, secret: &NodeSecret) -> (Client, ServerMessage, ServerMessage) {
    let mut client = connect(coord, secret).await;
    send(&mut client, ClientMessage::Hello { invite: None }).await;
    let welcome = recv(&mut client).await;
    let net_map = recv(&mut client).await;
    (client, welcome, net_map)
}

#[tokio::test]
async fn member_gets_welcome_and_the_rest_of_the_network() {
    let a = NodeSecret::generate();
    let b = NodeSecret::generate();
    let coord = start(&[&a, &b]).await;

    let (_client, welcome, net_map) = join(&coord, &b).await;
    // 地址按名单顺序：a 是 .1，b 是 .2
    assert_eq!(
        welcome,
        ServerMessage::Welcome {
            overlay_ip: Ipv4Addr::new(100, 64, 0, 2),
            prefix_len: 10,
            probe: Some(coord.probe),
            relays: vec![],
        }
    );
    // a 还没上过线，但它是网里的成员
    assert_eq!(
        net_map,
        ServerMessage::NetMap {
            peers: vec![PeerInfo {
                key: a.public_key(),
                overlay_ip: Ipv4Addr::new(100, 64, 0, 1),
                endpoints: vec![],
                name: String::new(),
            }],
        }
    );
}

#[tokio::test]
async fn stranger_is_rejected() {
    let a = NodeSecret::generate();
    let stranger = NodeSecret::generate();
    let coord = start(&[&a]).await;

    let mut client = connect(&coord, &stranger).await;
    send(&mut client, ClientMessage::Hello { invite: None }).await;
    assert!(matches!(
        recv(&mut client).await,
        ServerMessage::Rejected { .. }
    ));
    // 随后连接被关掉
    let closed = tokio::time::timeout(WAIT, client.0.recv()).await.unwrap();
    assert!(closed.is_err());
}

#[tokio::test]
async fn endpoint_changes_reach_the_other_nodes() {
    let a = NodeSecret::generate();
    let b = NodeSecret::generate();
    let coord = start(&[&a, &b]).await;
    let (mut client_a, ..) = join(&coord, &a).await;
    let (mut client_b, ..) = join(&coord, &b).await;

    let endpoint: SocketAddr = "198.51.100.4:41641".parse().unwrap();
    send(&mut client_a, ClientMessage::Endpoints(vec![endpoint])).await;
    assert_eq!(
        recv(&mut client_b).await,
        ServerMessage::NetMap {
            peers: vec![PeerInfo {
                key: a.public_key(),
                overlay_ip: Ipv4Addr::new(100, 64, 0, 1),
                endpoints: vec![endpoint],
                name: String::new(),
            }],
        }
    );
}

#[tokio::test]
async fn names_reach_the_other_nodes_cleaned_up() {
    let a = NodeSecret::generate();
    let b = NodeSecret::generate();
    let coord = start(&[&a, &b]).await;
    let (mut client_a, ..) = join(&coord, &a).await;
    let (mut client_b, ..) = join(&coord, &b).await;

    let name_in = |message: ServerMessage| match message {
        ServerMessage::NetMap { peers } => peers[0].name.clone(),
        other => panic!("应该是 NetMap，收到 {other:?}"),
    };
    // 换行、方向控制字符去掉，首尾空白去掉
    send(
        &mut client_a,
        ClientMessage::SetName("  小明\n\u{202E}的电脑 ".into()),
    )
    .await;
    assert_eq!(name_in(recv(&mut client_b).await), "小明的电脑");
    // 太长的截断
    send(&mut client_a, ClientMessage::SetName("长".repeat(100))).await;
    assert_eq!(
        name_in(recv(&mut client_b).await).chars().count(),
        meshora_proto::control::MAX_NAME_CHARS
    );
    // 清空
    send(&mut client_a, ClientMessage::SetName(String::new())).await;
    assert_eq!(name_in(recv(&mut client_b).await), "");
}

#[tokio::test]
async fn endpoint_list_is_capped() {
    let a = NodeSecret::generate();
    let b = NodeSecret::generate();
    let coord = start(&[&a, &b]).await;
    let (mut client_a, ..) = join(&coord, &a).await;
    let (mut client_b, ..) = join(&coord, &b).await;

    let many: Vec<SocketAddr> = (0..100u16)
        .map(|port| SocketAddr::from(([198, 51, 100, 4], 40000 + port)))
        .collect();
    send(&mut client_a, ClientMessage::Endpoints(many)).await;
    let ServerMessage::NetMap { peers } = recv(&mut client_b).await else {
        panic!("应该收到 NetMap");
    };
    assert_eq!(peers[0].endpoints.len(), MAX_ENDPOINTS);
}

#[tokio::test]
async fn endpoints_nobody_should_probe_are_dropped() {
    let a = NodeSecret::generate();
    let b = NodeSecret::generate();
    let coord = start(&[&a, &b]).await;
    let (mut client_a, ..) = join(&coord, &a).await;
    let (mut client_b, ..) = join(&coord, &b).await;

    let good: SocketAddr = "198.51.100.4:41641".parse().unwrap();
    let reported: Vec<SocketAddr> = [
        "0.0.0.0:41641",
        "[::]:41641",
        "224.0.0.251:5353",
        "[ff02::1]:41641",
        "255.255.255.255:41641",
        "198.51.100.5:0",
    ]
    .iter()
    .map(|addr| addr.parse().unwrap())
    .chain([good])
    .collect();
    send(&mut client_a, ClientMessage::Endpoints(reported)).await;
    let ServerMessage::NetMap { peers } = recv(&mut client_b).await else {
        panic!("应该收到 NetMap");
    };
    assert_eq!(peers[0].endpoints, [good]);
}

/// 端点变化得很快时，NetMap 合并着发，但最后一份一定是最新的
#[tokio::test]
async fn a_burst_of_updates_ends_with_the_latest_net_map() {
    let a = NodeSecret::generate();
    let b = NodeSecret::generate();
    let coord = start(&[&a, &b]).await;
    let (mut client_a, ..) = join(&coord, &a).await;
    let (mut client_b, ..) = join(&coord, &b).await;

    let endpoint = |port: u16| SocketAddr::from(([198, 51, 100, 4], port));
    for port in 40000..40200 {
        send(
            &mut client_a,
            ClientMessage::Endpoints(vec![endpoint(port)]),
        )
        .await;
    }
    let mut received = 0;
    loop {
        let ServerMessage::NetMap { peers } = recv(&mut client_b).await else {
            panic!("应该收到 NetMap");
        };
        received += 1;
        if peers[0].endpoints == [endpoint(40199)] {
            break;
        }
    }
    // 200 次变化，每秒最多一份：这里只可能收到寥寥几份
    assert!(received <= 3, "收到了 {received} 份 NetMap");
}

#[tokio::test]
async fn call_me_maybe_is_forwarded_with_the_callers_endpoints() {
    let a = NodeSecret::generate();
    let b = NodeSecret::generate();
    let coord = start(&[&a, &b]).await;
    let (mut client_a, ..) = join(&coord, &a).await;
    let (mut client_b, ..) = join(&coord, &b).await;

    let endpoint: SocketAddr = "198.51.100.4:41641".parse().unwrap();
    send(&mut client_a, ClientMessage::Endpoints(vec![endpoint])).await;
    recv(&mut client_b).await; // 更新后的 NetMap

    send(
        &mut client_a,
        ClientMessage::CallMeMaybe {
            peer: b.public_key(),
        },
    )
    .await;
    assert_eq!(
        recv(&mut client_b).await,
        ServerMessage::CallMeMaybe {
            peer: a.public_key(),
            endpoints: vec![endpoint],
        }
    );
}

/// 同一个来源开一堆连接、光连不握手，占不走别人的名额，只占它自己的
#[tokio::test]
async fn one_source_can_only_hold_a_few_handshakes() {
    let a = NodeSecret::generate();
    let coord = start(&[&a]).await;

    let mut idle = Vec::new();
    for _ in 0..PENDING_PER_SOURCE {
        idle.push(TcpStream::connect(coord.addr).await.unwrap());
    }
    // 等服务端把它们都接下来、占上名额
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 同一个来源再来一条：连上就被断开，握手失败
    let tcp = TcpStream::connect(coord.addr).await.unwrap();
    let refused = tokio::time::timeout(
        WAIT,
        NoiseStream::connect(tcp, Channel::Control, &a, &coord.key),
    )
    .await
    .expect("应该马上被断开");
    assert!(refused.is_err());

    // 占着的连接一放，名额就回来了
    drop(idle);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (_client, welcome, _) = join(&coord, &a).await;
    assert!(matches!(welcome, ServerMessage::Welcome { .. }));
}

#[tokio::test]
async fn nothing_happens_before_the_first_encrypted_message() {
    // A 完成了握手但不发 Hello（就像一个被重放的握手包）：它不算上线，
    // 发给它的打洞请求不会被转过去
    let a = NodeSecret::generate();
    let b = NodeSecret::generate();
    let coord = start(&[&a, &b]).await;
    let mut silent_a = connect(&coord, &a).await;
    let (mut client_b, ..) = join(&coord, &b).await;

    send(
        &mut client_b,
        ClientMessage::CallMeMaybe {
            peer: a.public_key(),
        },
    )
    .await;
    send(&mut client_b, ClientMessage::Ping).await;
    assert_eq!(recv(&mut client_b).await, ServerMessage::Pong);
    let nothing = tokio::time::timeout(Duration::from_millis(200), silent_a.0.recv()).await;
    assert!(nothing.is_err(), "没打招呼的连接不该收到任何东西");
}

#[tokio::test]
async fn probe_answers_members_with_their_observed_address() {
    let a = NodeSecret::generate();
    let stranger = NodeSecret::generate();
    let coord = start(&[&a]).await;
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut buf = vec![0u8; 2048];

    // 陌生人：不回应
    let ping = disco::seal(&stranger, &coord.key, &DiscoMessage::Ping { tx: [1; 12] });
    socket.send_to(&ping, coord.probe).await.unwrap();
    // 成员：回 Pong，带着"看到你从哪来"
    let ping = disco::seal(&a, &coord.key, &DiscoMessage::Ping { tx: [2; 12] });
    socket.send_to(&ping, coord.probe).await.unwrap();

    let (len, from) = tokio::time::timeout(WAIT, socket.recv_from(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(from, coord.probe);
    let (sender, message) = disco::open(&a, &buf[..len], |k| *k == coord.key).unwrap();
    assert_eq!(sender, coord.key);
    assert_eq!(
        message,
        DiscoMessage::Pong {
            tx: [2; 12],
            observed: socket.local_addr().unwrap(),
        }
    );
}

#[test]
fn addresses_follow_the_member_list() {
    let keys: Vec<NodeKey> = (1..=3).map(|n| NodeKey::from_bytes([n; 32])).collect();
    let members = assign(&keys, "100.64.0.0/10".parse().unwrap()).unwrap();
    let ips: Vec<Ipv4Addr> = members.iter().map(|member| member.ip).collect();
    assert_eq!(
        ips,
        [
            Ipv4Addr::new(100, 64, 0, 1),
            Ipv4Addr::new(100, 64, 0, 2),
            Ipv4Addr::new(100, 64, 0, 3)
        ]
    );
}

#[test]
fn bad_member_lists_are_refused() {
    let k = NodeKey::from_bytes([1; 32]);
    assert!(assign(&[k, k], "100.64.0.0/10".parse().unwrap()).is_err());
    // /30 只有两个可用地址
    let keys: Vec<NodeKey> = (1..=3).map(|n| NodeKey::from_bytes([n; 32])).collect();
    assert!(assign(&keys, "10.0.0.0/30".parse().unwrap()).is_err());
}

/// 测试用的状态文件，放在单独的临时目录里，用完删掉
struct TempState(PathBuf);

impl TempState {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("meshora-coord-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Self(dir.join("coord.state"))
    }

    fn text(&self) -> String {
        std::fs::read_to_string(&self.0).unwrap()
    }
}

impl Drop for TempState {
    fn drop(&mut self) {
        if let Some(dir) = self.0.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

fn state_config(secret: NodeSecret, nodes: Vec<NodeKey>, state: &TempState) -> Config {
    Config {
        secret,
        nodes,
        state: Some(state.0.clone()),
        overlay: "100.64.0.0/10".parse().unwrap(),
        probe: None,
        relays: vec![],
        hub: None,
    }
}

/// 带状态文件起一个协调服务，交出它的邀请码
async fn start_with(
    secret: NodeSecret,
    nodes: &[&NodeSecret],
    state: &TempState,
) -> (Coord, Invite) {
    let key = secret.public_key();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let coordinator = Coordinator::new(state_config(
        secret,
        nodes.iter().map(|n| n.public_key()).collect(),
        state,
    ))
    .unwrap();
    let invite = coordinator.invite().expect("有状态文件就有邀请码");
    tokio::spawn(coordinator.serve(listener, None));
    let coord = Coord {
        addr,
        probe: addr,
        key,
    };
    (coord, invite)
}

async fn hello(
    coord: &Coord,
    secret: &NodeSecret,
    invite: Option<Invite>,
) -> (Client, ServerMessage) {
    let mut client = connect(coord, secret).await;
    send(&mut client, ClientMessage::Hello { invite }).await;
    let first = recv(&mut client).await;
    (client, first)
}

fn welcome_ip(message: &ServerMessage) -> Ipv4Addr {
    match message {
        ServerMessage::Welcome { overlay_ip, .. } => *overlay_ip,
        other => panic!("应该是 Welcome，收到 {other:?}"),
    }
}

#[tokio::test]
async fn an_invite_lets_a_stranger_join_with_the_next_free_address() {
    let state = TempState::new("join");
    let a = NodeSecret::generate();
    let friend = NodeSecret::generate();
    let (coord, invite) = start_with(NodeSecret::generate(), &[&a], &state).await;
    let (mut client_a, _, _) = join(&coord, &a).await;

    let (_friend, welcome) = hello(&coord, &friend, Some(invite)).await;
    assert_eq!(welcome_ip(&welcome), Ipv4Addr::new(100, 64, 0, 2));

    // 已经在线的人收到新的 NetMap，里面有了新成员
    let net_map = recv(&mut client_a).await;
    assert_eq!(
        net_map,
        ServerMessage::NetMap {
            peers: vec![PeerInfo {
                key: friend.public_key(),
                overlay_ip: Ipv4Addr::new(100, 64, 0, 2),
                endpoints: vec![],
                name: String::new(),
            }],
        }
    );
    // 写进了状态文件
    assert!(
        state
            .text()
            .contains(&format!("member {} 100.64.0.2", friend.public_key()))
    );
}

#[tokio::test]
async fn a_wrong_invite_is_rejected_and_changes_nothing() {
    let state = TempState::new("wrong");
    let a = NodeSecret::generate();
    let stranger = NodeSecret::generate();
    let (coord, _) = start_with(NodeSecret::generate(), &[&a], &state).await;

    let (_client, reply) = hello(&coord, &stranger, Some(Invite::generate())).await;
    let ServerMessage::Rejected { reason } = reply else {
        panic!("应该被拒绝");
    };
    assert!(reason.contains("邀请码不对"), "{reason}");
    assert!(!state.text().contains("member "));

    // 不带邀请码的陌生人也进不来
    let (_client, reply) = hello(&coord, &stranger, None).await;
    assert!(matches!(reply, ServerMessage::Rejected { .. }));
}

#[tokio::test]
async fn without_a_state_file_invites_are_refused() {
    let a = NodeSecret::generate();
    let stranger = NodeSecret::generate();
    let coord = start(&[&a]).await;
    let (_client, reply) = hello(&coord, &stranger, Some(Invite::generate())).await;
    let ServerMessage::Rejected { reason } = reply else {
        panic!("应该被拒绝");
    };
    assert!(reason.contains("不接受"), "{reason}");
}

#[tokio::test]
async fn joined_members_keep_their_address_across_restarts() {
    let state = TempState::new("restart");
    let a = NodeSecret::generate();
    let friend = NodeSecret::generate();
    let other = NodeSecret::generate();
    let secret = NodeSecret::generate();

    let (coord, invite) = start_with(secret.clone(), &[&a], &state).await;
    let (_c1, welcome) = hello(&coord, &friend, Some(invite)).await;
    assert_eq!(welcome_ip(&welcome), Ipv4Addr::new(100, 64, 0, 2));
    let (_c2, welcome) = hello(&coord, &other, Some(invite)).await;
    assert_eq!(welcome_ip(&welcome), Ipv4Addr::new(100, 64, 0, 3));

    // 重启：同一个状态文件。邀请码不变，成员不用邀请码也认得，地址不变
    let (coord, again) = start_with(secret, &[&a], &state).await;
    assert_eq!(again, invite);
    let (_c3, welcome) = hello(&coord, &other, None).await;
    assert_eq!(welcome_ip(&welcome), Ipv4Addr::new(100, 64, 0, 3));
}

#[tokio::test]
async fn deleting_the_invite_line_rotates_it_without_touching_members() {
    let state = TempState::new("rotate");
    let a = NodeSecret::generate();
    let friend = NodeSecret::generate();
    let late = NodeSecret::generate();
    let secret = NodeSecret::generate();

    let (coord, old) = start_with(secret.clone(), &[&a], &state).await;
    let (_c, _) = hello(&coord, &friend, Some(old)).await;

    let text: String = state
        .text()
        .lines()
        .filter(|line| !line.starts_with("invite"))
        .map(|line| format!("{line}\n"))
        .collect();
    std::fs::write(&state.0, text).unwrap();

    let (coord, new) = start_with(secret, &[&a], &state).await;
    assert_ne!(new, old);
    // 旧的邀请码进不来了；已经加入的照常
    let (_c, reply) = hello(&coord, &late, Some(old)).await;
    assert!(matches!(reply, ServerMessage::Rejected { .. }));
    let (_c, reply) = hello(&coord, &friend, None).await;
    assert_eq!(welcome_ip(&reply), Ipv4Addr::new(100, 64, 0, 2));
}

#[test]
fn a_state_file_that_collides_with_the_node_list_is_an_error() {
    let state = TempState::new("collide");
    let a = NodeSecret::generate();
    let b = NodeSecret::generate();
    let joined = NodeSecret::generate();
    std::fs::create_dir_all(state.0.parent().unwrap()).unwrap();
    // joined 占着 .2，名单后来又加了一个 b，也要 .2
    std::fs::write(
        &state.0,
        format!("member {} 100.64.0.2\n", joined.public_key()),
    )
    .unwrap();
    let ok = Coordinator::new(state_config(
        NodeSecret::generate(),
        vec![a.public_key()],
        &state,
    ));
    assert!(ok.is_ok());
    let err = Coordinator::new(state_config(
        NodeSecret::generate(),
        vec![a.public_key(), b.public_key()],
        &state,
    ))
    .err()
    .unwrap();
    assert!(err.to_string().contains("已经分给了"), "{err}");

    std::fs::write(&state.0, "member not-a-key 100.64.0.2\n").unwrap();
    assert!(Coordinator::new(state_config(NodeSecret::generate(), vec![], &state)).is_err());
}

#[test]
fn the_member_check_for_the_relay_sees_new_members() {
    let state = TempState::new("relay");
    let a = NodeSecret::generate();
    let friend = NodeSecret::generate();
    let coordinator = Coordinator::new(state_config(
        NodeSecret::generate(),
        vec![a.public_key()],
        &state,
    ))
    .unwrap();
    let members = coordinator.members();
    assert!(members(&a.public_key()));
    assert!(!members(&friend.public_key()));

    let invite = coordinator.invite().unwrap();
    coordinator
        .shared
        .legacy
        .as_ref()
        .unwrap()
        .admit(&friend.public_key(), Some(invite))
        .unwrap();
    assert!(members(&friend.public_key()));
}

/// 改状态文件：`edit` 拿到现在的内容，返回新的
fn rewrite(state: &TempState, edit: impl FnOnce(&str) -> String) {
    let text = edit(&state.text());
    std::fs::write(&state.0, text).unwrap();
}

/// 反复试，直到 `check` 成立；最多等过两轮状态文件检查
async fn eventually<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + STATE_POLL * 3;
    while tokio::time::Instant::now() < deadline {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("{what}：等了 {:?} 还没生效", STATE_POLL * 3);
}

#[tokio::test]
async fn removing_a_member_line_kicks_it_at_once() {
    let state = TempState::new("kick");
    let a = NodeSecret::generate();
    let friend = NodeSecret::generate();
    let (coord, invite) = start_with(NodeSecret::generate(), &[&a], &state).await;
    let (mut friend_client, welcome) = hello(&coord, &friend, Some(invite)).await;
    welcome_ip(&welcome);
    let _ = recv(&mut friend_client).await; // 第一份 NetMap
    let (mut client_a, _, _) = join(&coord, &a).await;

    let friend_key = friend.public_key().to_string();
    rewrite(&state, |text| {
        text.lines()
            .filter(|line| !line.contains(&friend_key))
            .map(|line| format!("{line}\n"))
            .collect()
    });

    // 被移出的一方先收到原因，然后连接被关掉
    let ServerMessage::Rejected { reason } = recv(&mut friend_client).await else {
        panic!("应该收到 Rejected");
    };
    assert!(reason.contains("移出"), "{reason}");
    let closed = tokio::time::timeout(WAIT, friend_client.0.recv())
        .await
        .unwrap();
    assert!(closed.is_err());

    // 其他人的 NetMap 里没有它了
    loop {
        match recv(&mut client_a).await {
            ServerMessage::NetMap { peers } if peers.is_empty() => break,
            ServerMessage::NetMap { .. } => continue,
            other => panic!("没想到收到 {other:?}"),
        }
    }
    // 不带邀请码也回不来
    let (_c, reply) = hello(&coord, &friend, None).await;
    assert!(matches!(reply, ServerMessage::Rejected { .. }));
}

#[tokio::test]
async fn a_new_invite_in_the_file_takes_effect_live() {
    let state = TempState::new("live-invite");
    let a = NodeSecret::generate();
    let (coord, old) = start_with(NodeSecret::generate(), &[&a], &state).await;
    let new = Invite::generate();
    rewrite(&state, |text| {
        text.replace(&old.to_string(), &new.to_string())
    });

    eventually("旧邀请码作废", || async {
        let (_c, reply) = hello(&coord, &NodeSecret::generate(), Some(old)).await;
        matches!(reply, ServerMessage::Rejected { .. })
    })
    .await;
    let (_c, reply) = hello(&coord, &NodeSecret::generate(), Some(new)).await;
    welcome_ip(&reply);
}

#[tokio::test]
async fn deleting_the_invite_line_while_running_writes_a_new_one() {
    let state = TempState::new("live-rotate");
    let a = NodeSecret::generate();
    let (coord, old) = start_with(NodeSecret::generate(), &[&a], &state).await;
    rewrite(&state, |text| {
        text.lines()
            .filter(|line| !line.starts_with("invite"))
            .map(|line| format!("{line}\n"))
            .collect()
    });

    eventually("写回新的邀请码", || async {
        state.text().lines().any(|line| line.starts_with("invite "))
    })
    .await;
    assert!(!state.text().contains(&old.to_string()));
    let (_c, reply) = hello(&coord, &NodeSecret::generate(), Some(old)).await;
    assert!(matches!(reply, ServerMessage::Rejected { .. }));
}

#[tokio::test]
async fn a_broken_state_file_changes_nothing() {
    let state = TempState::new("live-broken");
    let a = NodeSecret::generate();
    let friend = NodeSecret::generate();
    let (coord, invite) = start_with(NodeSecret::generate(), &[&a], &state).await;
    let (_c, reply) = hello(&coord, &friend, Some(invite)).await;
    welcome_ip(&reply);

    std::fs::write(&state.0, "这不是状态文件\n").unwrap();
    // 等过一轮检查：成员和邀请码都还在
    tokio::time::sleep(STATE_POLL * 2).await;
    let (_c, reply) = hello(&coord, &friend, None).await;
    assert_eq!(welcome_ip(&reply), Ipv4Addr::new(100, 64, 0, 2));
    let (_c, reply) = hello(&coord, &NodeSecret::generate(), Some(invite)).await;
    welcome_ip(&reply);
}

#[test]
fn utc_times_round_trip() {
    assert_eq!(format_utc(UNIX_EPOCH), "1970-01-01T00:00:00Z");
    let leap = UNIX_EPOCH + Duration::from_secs(951_782_400);
    assert_eq!(format_utc(leap), "2000-02-29T00:00:00Z");
    assert_eq!(parse_utc("2000-02-29T00:00:00Z"), Some(leap));
    let at = parse_utc("2026-10-01T12:34:56Z").unwrap();
    assert_eq!(format_utc(at), "2026-10-01T12:34:56Z");
    for bad in [
        "2026-02-30T00:00:00Z",
        "2026-10-01T12:00:00",
        "2026-10-01 12:00:00Z",
        "2026-13-01T00:00:00Z",
        "2026-10-01T24:00:00Z",
        "1969-12-31T23:59:59Z",
    ] {
        assert_eq!(parse_utc(bad), None, "{bad}");
    }
}

/// 在状态文件里加一行带限制的邀请码（协调服务启动之前）
fn add_limited(state: &TempState, line: &str) {
    std::fs::create_dir_all(state.0.parent().unwrap()).unwrap();
    let mut text = std::fs::read_to_string(&state.0).unwrap_or_default();
    text.push_str(line);
    text.push('\n');
    std::fs::write(&state.0, text).unwrap();
}

#[tokio::test]
async fn a_one_time_invite_works_once() {
    let state = TempState::new("one-time");
    let (code, line) = limited_invite(Some(1), None).unwrap();
    add_limited(&state, &line);
    let a = NodeSecret::generate();
    let (coord, permanent) = start_with(NodeSecret::generate(), &[&a], &state).await;
    assert_ne!(code, permanent);

    let (_c, reply) = hello(&coord, &NodeSecret::generate(), Some(code)).await;
    assert_eq!(welcome_ip(&reply), Ipv4Addr::new(100, 64, 0, 2));
    // 用掉了：从文件里清掉，第二个人进不来
    assert!(!state.text().contains(&code.to_string()));
    let (_c, reply) = hello(&coord, &NodeSecret::generate(), Some(code)).await;
    assert!(matches!(reply, ServerMessage::Rejected { .. }));
    // 长期有效的那个不受影响
    let (_c, reply) = hello(&coord, &NodeSecret::generate(), Some(permanent)).await;
    welcome_ip(&reply);
}

#[tokio::test]
async fn uses_are_counted_down_in_the_state_file() {
    let state = TempState::new("uses");
    let (code, line) = limited_invite(Some(2), None).unwrap();
    add_limited(&state, &line);
    let (coord, _) = start_with(NodeSecret::generate(), &[&NodeSecret::generate()], &state).await;
    let (_c, reply) = hello(&coord, &NodeSecret::generate(), Some(code)).await;
    welcome_ip(&reply);
    assert!(state.text().contains(&format!("invite {code} uses=1")));
}

#[tokio::test]
async fn an_expired_invite_is_refused_and_cleaned_up() {
    let state = TempState::new("expired");
    let code = Invite::generate();
    add_limited(
        &state,
        &format!("invite {code} expires=2000-01-01T00:00:00Z"),
    );
    let (coord, _) = start_with(NodeSecret::generate(), &[&NodeSecret::generate()], &state).await;
    let (_c, reply) = hello(&coord, &NodeSecret::generate(), Some(code)).await;
    let ServerMessage::Rejected { reason } = reply else {
        panic!("应该被拒绝");
    };
    assert!(reason.contains("过期"), "{reason}");
    assert!(
        !state.text().contains(&code.to_string()),
        "过期的从文件里清掉"
    );
}

#[tokio::test]
async fn an_invite_added_while_running_takes_effect() {
    // meshora-coord invite 就是这么干的：协调服务开着，往状态文件里追加一行
    let state = TempState::new("live-limited");
    let (coord, _) = start_with(NodeSecret::generate(), &[&NodeSecret::generate()], &state).await;
    let (code, line) = limited_invite(None, Some(Duration::from_secs(3600))).unwrap();
    assert!(line.contains("expires="));
    add_limited(&state, &line);
    let (_c, reply) = hello(&coord, &NodeSecret::generate(), Some(code)).await;
    welcome_ip(&reply);
}

#[test]
fn a_limited_invite_needs_a_limit() {
    assert!(limited_invite(None, None).is_none());
}

// ---------- hub 模式 ----------

/// hub 模式的协调服务，数据放在 `state` 的目录里
async fn start_hub(state: &TempState, limits: HubLimits) -> (Coord, meshora_relay::Links) {
    let secret = NodeSecret::generate();
    let key = secret.public_key();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let probe_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let probe = probe_socket.local_addr().unwrap();
    let coordinator = Coordinator::new(Config {
        secret,
        nodes: vec![],
        state: None,
        overlay: "100.64.0.0/10".parse().unwrap(),
        probe: Some(probe),
        relays: vec![],
        hub: Some(HubConfig {
            dir: state.0.parent().unwrap().to_path_buf(),
            limits,
            creators: None,
        }),
    })
    .unwrap();
    let links = coordinator.links();
    tokio::spawn(coordinator.serve(listener, Some(probe_socket)));
    (Coord { addr, probe, key }, links)
}

/// 建一个网络：收下 Created、Welcome、第一份 NetMap 和成员清单
async fn create(coord: &Coord, owner: &NodeSecret, name: &str) -> (Client, NetworkId, Invite) {
    let mut client = connect(coord, owner).await;
    send(&mut client, ClientMessage::Create { name: name.into() }).await;
    let ServerMessage::Created { network, invite } = recv(&mut client).await else {
        panic!("应该先收到 Created");
    };
    let ServerMessage::Welcome { overlay_ip, .. } = recv(&mut client).await else {
        panic!("接着是 Welcome");
    };
    assert_eq!(overlay_ip, Ipv4Addr::new(100, 64, 0, 1), "网主拿第一个地址");
    (client, network, invite)
}

/// 凭网络 ID 和邀请码加入：返回连接和 Welcome
async fn join_hub(
    coord: &Coord,
    secret: &NodeSecret,
    network: NetworkId,
    invite: Option<Invite>,
) -> (Client, ServerMessage) {
    let mut client = connect(coord, secret).await;
    send(&mut client, ClientMessage::Join { network, invite }).await;
    let first = recv(&mut client).await;
    (client, first)
}

/// 一直收，直到收到满足条件的消息（中间夹着的 NetMap、成员清单跳过）
async fn recv_until(
    client: &mut Client,
    mut wanted: impl FnMut(&ServerMessage) -> bool,
) -> ServerMessage {
    loop {
        let message = recv(client).await;
        if wanted(&message) {
            return message;
        }
    }
}

#[tokio::test]
async fn a_hub_network_is_created_joined_and_seen_by_its_owner() {
    let state = TempState::new("hub-create");
    let (coord, _) = start_hub(&state, HubLimits::default()).await;
    let owner = NodeSecret::generate();
    let friend = NodeSecret::generate();
    let (mut owner_client, network, invite) = create(&coord, &owner, "周末开黑").await;

    let (_friend_client, welcome) = join_hub(&coord, &friend, network, Some(invite)).await;
    assert!(
        matches!(welcome, ServerMessage::Welcome { overlay_ip, .. } if overlay_ip == Ipv4Addr::new(100, 64, 0, 2)),
        "{welcome:?}"
    );

    // 网主会收到两样：有朋友的 NetMap，和两个人都在线的成员清单。它们合并着发、先后不定，一起等
    let mut net_map = None;
    let mut roster = None;
    while net_map.is_none() || roster.is_none() {
        match recv(&mut owner_client).await {
            ServerMessage::NetMap { peers } if peers.len() == 1 => net_map = Some(peers),
            ServerMessage::Roster(r)
                if r.members.len() == 2 && r.members.iter().all(|m| m.online) =>
            {
                roster = Some(r);
            }
            _ => {}
        }
    }
    // 网主的 NetMap 里有朋友
    assert_eq!(net_map.unwrap()[0].key, friend.public_key());
    // 成员清单：两个人，都在线，自己是网主
    let roster = roster.unwrap();
    assert_eq!(roster.name, "周末开黑");
    assert_eq!(roster.invite, Some(invite));
    assert!(
        roster
            .members
            .iter()
            .any(|m| m.key == owner.public_key() && m.owner)
    );
    assert!(
        roster
            .members
            .iter()
            .any(|m| m.key == friend.public_key() && !m.owner)
    );
}

#[tokio::test]
async fn hub_networks_do_not_see_each_other() {
    let state = TempState::new("hub-isolated");
    let (coord, links) = start_hub(&state, HubLimits::default()).await;
    let (a, b, c) = (
        NodeSecret::generate(),
        NodeSecret::generate(),
        NodeSecret::generate(),
    );
    let (_a_client, network_a, invite_a) = create(&coord, &a, "A").await;
    let (_c_client, network_c, _) = create(&coord, &c, "C").await;
    assert_ne!(network_a, network_c);

    let (mut b_client, _) = join_hub(&coord, &b, network_a, Some(invite_a)).await;
    let net_map = recv_until(&mut b_client, |m| matches!(m, ServerMessage::NetMap { .. })).await;
    let ServerMessage::NetMap { peers } = net_map else {
        unreachable!()
    };
    assert_eq!(peers.len(), 1, "b 只看得到同一个网络里的 a");
    assert_eq!(peers[0].key, a.public_key());

    // 中继只在同一个网络里转发
    assert!(links(&a.public_key(), &b.public_key()));
    assert!(!links(&a.public_key(), &c.public_key()));
    assert!(!links(&b.public_key(), &c.public_key()));

    // 拿着 A 的邀请码进不了 C
    let (_, rejected) = join_hub(&coord, &NodeSecret::generate(), network_c, Some(invite_a)).await;
    assert!(
        matches!(rejected, ServerMessage::Rejected { .. }),
        "{rejected:?}"
    );
}

#[tokio::test]
async fn only_the_owner_can_manage_and_the_owner_can_do_everything() {
    let state = TempState::new("hub-admin");
    let (coord, _) = start_hub(&state, HubLimits::default()).await;
    let owner = NodeSecret::generate();
    let friend = NodeSecret::generate();
    let (mut owner_client, network, invite) = create(&coord, &owner, "网").await;
    let (mut friend_client, _) = join_hub(&coord, &friend, network, Some(invite)).await;

    let is_reply = |m: &ServerMessage| matches!(m, ServerMessage::AdminReply(_));
    // 不是网主：管不了
    send(
        &mut friend_client,
        ClientMessage::Admin(AdminRequest::Delete),
    )
    .await;
    let reply = recv_until(&mut friend_client, is_reply).await;
    assert!(
        matches!(reply, ServerMessage::AdminReply(Err(_))),
        "{reply:?}"
    );

    // 新建一个一次性邀请码，陌生人用它进来
    send(
        &mut owner_client,
        ClientMessage::Admin(AdminRequest::NewInvite {
            uses: Some(1),
            hours: None,
        }),
    )
    .await;
    let ServerMessage::AdminReply(Ok(Some(once))) = recv_until(&mut owner_client, is_reply).await
    else {
        panic!("新建邀请码应该成功");
    };
    let stranger = NodeSecret::generate();
    let (_stranger_client, welcome) = join_hub(&coord, &stranger, network, Some(once)).await;
    assert!(matches!(welcome, ServerMessage::Welcome { .. }));
    // 用过了，第二个人进不来
    let (_, rejected) = join_hub(&coord, &NodeSecret::generate(), network, Some(once)).await;
    assert!(matches!(rejected, ServerMessage::Rejected { .. }));

    // 换掉长期邀请码：旧的作废
    send(
        &mut owner_client,
        ClientMessage::Admin(AdminRequest::RotateInvite),
    )
    .await;
    let ServerMessage::AdminReply(Ok(Some(rotated))) =
        recv_until(&mut owner_client, is_reply).await
    else {
        panic!("换码应该成功");
    };
    assert_ne!(rotated, invite);
    let (_, rejected) = join_hub(&coord, &NodeSecret::generate(), network, Some(invite)).await;
    assert!(matches!(rejected, ServerMessage::Rejected { .. }));

    // 网主不能把自己踢出去
    send(
        &mut owner_client,
        ClientMessage::Admin(AdminRequest::Kick {
            member: owner.public_key(),
        }),
    )
    .await;
    assert!(matches!(
        recv_until(&mut owner_client, is_reply).await,
        ServerMessage::AdminReply(Err(_))
    ));

    // 踢掉朋友：他收到"被移出"，然后连接断开
    send(
        &mut owner_client,
        ClientMessage::Admin(AdminRequest::Kick {
            member: friend.public_key(),
        }),
    )
    .await;
    assert!(matches!(
        recv_until(&mut owner_client, is_reply).await,
        ServerMessage::AdminReply(Ok(None))
    ));
    let kicked = recv_until(&mut friend_client, |m| {
        matches!(m, ServerMessage::Rejected { .. })
    })
    .await;
    assert!(matches!(kicked, ServerMessage::Rejected { reason } if reason.contains("移出")));

    // 解散：网主自己也被断开，网络找不到了
    send(
        &mut owner_client,
        ClientMessage::Admin(AdminRequest::Delete),
    )
    .await;
    let gone = recv_until(&mut owner_client, |m| {
        matches!(m, ServerMessage::Rejected { .. })
    })
    .await;
    assert!(matches!(gone, ServerMessage::Rejected { reason } if reason.contains("解散")));
    let (_, rejected) = join_hub(&coord, &NodeSecret::generate(), network, Some(rotated)).await;
    assert!(matches!(rejected, ServerMessage::Rejected { reason } if reason.contains("找不到")));
    assert!(
        !state
            .0
            .parent()
            .unwrap()
            .join(format!("{network}.state"))
            .exists(),
        "状态文件删了"
    );
}

#[tokio::test]
async fn one_owner_can_only_create_so_many_networks() {
    let state = TempState::new("hub-limit");
    let limits = HubLimits {
        networks_per_owner: 2,
        ..HubLimits::default()
    };
    let (coord, _) = start_hub(&state, limits).await;
    let owner = NodeSecret::generate();
    let _first = create(&coord, &owner, "一").await;
    let _second = create(&coord, &owner, "二").await;
    let mut client = connect(&coord, &owner).await;
    send(&mut client, ClientMessage::Create { name: "三".into() }).await;
    assert!(
        matches!(recv(&mut client).await, ServerMessage::Rejected { reason } if reason.contains("最多"))
    );
}

#[tokio::test]
async fn hub_networks_survive_a_restart() {
    let state = TempState::new("hub-restart");
    let owner = NodeSecret::generate();
    let network;
    let invite;
    {
        let (coord, _) = start_hub(&state, HubLimits::default()).await;
        (_, network, invite) = create(&coord, &owner, "留下来").await;
    }
    // 同一个目录再起一个：网络还在，网主照样能进、拿回原来的地址；朋友凭原来的邀请码能加入
    let (coord, _) = start_hub(&state, HubLimits::default()).await;
    let (_, welcome) = join_hub(&coord, &owner, network, None).await;
    assert!(
        matches!(welcome, ServerMessage::Welcome { overlay_ip, .. } if overlay_ip == Ipv4Addr::new(100, 64, 0, 1)),
        "{welcome:?}"
    );
    let (_, welcome) = join_hub(&coord, &NodeSecret::generate(), network, Some(invite)).await;
    assert!(
        matches!(welcome, ServerMessage::Welcome { .. }),
        "{welcome:?}"
    );
}

#[tokio::test]
async fn a_hub_only_server_turns_away_old_style_hello() {
    let state = TempState::new("hub-hello");
    let (coord, _) = start_hub(&state, HubLimits::default()).await;
    let mut client = connect(&coord, &NodeSecret::generate()).await;
    send(&mut client, ClientMessage::Hello { invite: None }).await;
    assert!(
        matches!(recv(&mut client).await, ServerMessage::Rejected { reason } if reason.contains("网络 ID"))
    );
}
