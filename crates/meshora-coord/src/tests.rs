use std::time::Duration;

use meshora_proto::noise::{NoiseReader, NoiseWriter};

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
        overlay: "100.64.0.0/10".parse().unwrap(),
        probe: Some(probe),
        relays: vec![],
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
    send(&mut client, ClientMessage::Hello).await;
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
    send(&mut client, ClientMessage::Hello).await;
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
            }],
        }
    );
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
    let ips: Vec<Ipv4Addr> = members.iter().map(|(_, ip)| *ip).collect();
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
