//! 守护进程的真机测试：一边是真的 meshorad（真的虚拟网卡），另一边是跑在测试进程里的节点
//! （虚拟网卡用 channel 代替，收到 UDP 报文就原样弹回去）。系统往对端的 overlay 地址发 UDP，
//! 要能经加密隧道收到回应。直连和经中继各测一遍。
//!
//! 不用网络命名空间，所以 Linux 和 Windows 都能跑。需要管理员权限：Linux 上是 root；
//! Windows 上是管理员，并且 wintun.dll 放在 meshorad.exe 旁边（`target/debug/`）。

use std::fs::File;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use meshora_control::{Config, Session};
use meshora_proto::control::RelayInfo;
use meshora_types::{NodeKey, NodeSecret};
use meshora_wg::{TunChannels, UserspaceDataPlane};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::mpsc;

/// 注册、建网卡、探测、握手，一路下来要的时间
const WAIT: Duration = Duration::from_secs(30);
const ECHO_PORT: u16 = 7777;

/// 一次测试的参数。两个测试并行跑，网段和网卡名都得错开
struct Case {
    /// 只走中继：两个节点都带 relay_only，协调服务告诉它们一个中继
    relay_only: bool,
    /// overlay 网段的第三个字节。网段取自 RFC 2544 保留给网络测试的 198.18.0.0/15
    subnet: u8,
    /// 网卡名的后缀
    suffix: char,
}

impl Case {
    /// 名单里 meshorad 排第一，拿 .1；测试进程里的节点拿 .2
    fn daemon_ip(&self) -> Ipv4Addr {
        Ipv4Addr::new(198, 18, self.subnet, 1)
    }

    fn echo(&self) -> SocketAddrV4 {
        SocketAddrV4::new(Ipv4Addr::new(198, 18, self.subnet, 2), ECHO_PORT)
    }
}

struct Coord {
    addr: SocketAddr,
    key: NodeKey,
}

async fn start_relay(nodes: Vec<NodeKey>) -> RelayInfo {
    let secret = NodeSecret::generate();
    let key = secret.public_key();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(meshora_relay::serve(
        meshora_relay::Config { secret, nodes },
        listener,
    ));
    RelayInfo { key, addr }
}

async fn start_coord(case: &Case, nodes: Vec<NodeKey>, relays: Vec<RelayInfo>) -> Coord {
    let secret = NodeSecret::generate();
    let key = secret.public_key();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let probe = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let config = meshora_coord::Config {
        secret,
        nodes,
        overlay: format!("198.18.{}.0/24", case.subnet).parse().unwrap(),
        probe: Some(probe.local_addr().unwrap()),
        relays,
    };
    tokio::spawn(meshora_coord::serve(config, listener, Some(probe)));
    Coord { addr, key }
}

/// 把 UDP 报文原样弹回去：两端的地址和端口对调。
///
/// 校验和不用重算：IP 首部和 UDP 的校验和都是按 16 位求和，对调两个加数不改变和
fn bounce(packet: &[u8]) -> Option<Vec<u8>> {
    if packet.len() < 28 || packet[0] != 0x45 || packet[9] != 17 {
        return None;
    }
    let mut reply = packet.to_vec();
    reply[12..16].copy_from_slice(&packet[16..20]);
    reply[16..20].copy_from_slice(&packet[12..16]);
    reply[20..22].copy_from_slice(&packet[22..24]);
    reply[22..24].copy_from_slice(&packet[20..22]);
    Some(reply)
}

/// 测试进程里的节点：数据面是真的，虚拟网卡换成 channel，收到的 UDP 报文都弹回去
async fn start_echo_node(case: &Case, secret: &NodeSecret, coord: &Coord) {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let local_port = socket.local_addr().unwrap().port();
    let session = Session::connect(Config {
        secret: secret.clone(),
        coord: coord.addr,
        coord_key: coord.key,
        local_port,
        keepalive: NonZeroU16::new(25),
        relay_only: case.relay_only,
    })
    .await
    .unwrap();
    assert_eq!(session.welcome().overlay_ip, *case.echo().ip());

    let (tun_in, from_tun) = mpsc::channel(64);
    let (to_tun, mut tun_out) = mpsc::channel::<Vec<u8>>(64);
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let sink = move |event| {
        let _ = events_tx.send(event);
    };
    let dataplane = UserspaceDataPlane::start(
        secret,
        socket,
        TunChannels { from_tun, to_tun },
        Arc::new(sink),
    )
    .unwrap();
    tokio::spawn(session.run(Arc::new(dataplane), events_rx));
    tokio::spawn(async move {
        while let Some(packet) = tun_out.recv().await {
            if let Some(reply) = bounce(&packet) {
                let _ = tun_in.send(reply).await;
            }
        }
    });
}

/// 跑着的 meshorad。测试结束时杀掉；测试失败的话，顺带把它的日志打出来
struct Daemon {
    child: Child,
    dir: PathBuf,
}

impl Daemon {
    fn start(case: &Case, secret: &NodeSecret, coord: &Coord) -> Self {
        let id = std::process::id();
        let dir = std::env::temp_dir().join(format!("meshorad-test-{id}-{}", case.suffix));
        std::fs::create_dir_all(&dir).unwrap();
        let key = dir.join("node.key");
        std::fs::write(&key, secret.to_base64().as_bytes()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let log = File::create(dir.join("meshorad.log")).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_meshorad"));
        command
            .arg("up")
            .arg("--key")
            .arg(&key)
            .args(["--coord", &coord.addr.to_string()])
            .args(["--coord-key", &coord.key.to_string()])
            .args(["--port", "0"])
            .args(["--tun", &format!("mshd{}{}", id % 100_000, case.suffix)])
            .arg("--verbose");
        if case.relay_only {
            command.arg("--relay-only");
        }
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .unwrap();
        Daemon { child, dir }
    }

    /// 日志里最后一次"切换路径"选的是哪种：Direct 或 Relay
    fn last_path(&self) -> Option<String> {
        let log = std::fs::read_to_string(self.dir.join("meshorad.log")).ok()?;
        let line = log.lines().rfind(|line| line.contains("切换路径"))?;
        let path = line.rsplit("path=").next()?;
        Some(path.chars().take_while(char::is_ascii_alphabetic).collect())
    }

    /// 进程要是已经退出了，就直接判失败 —— 不用干等到超时
    fn assert_running(&mut self) {
        if let Some(status) = self.child.try_wait().unwrap() {
            panic!("meshorad 提前退出了：{status}");
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if std::thread::panicking() {
            let log = std::fs::read_to_string(self.dir.join("meshorad.log")).unwrap_or_default();
            eprintln!("---- meshorad 的日志\n{log}");
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 绑定到 meshorad 的虚拟网卡的地址上：网卡建好、地址设好之前绑不上
async fn bind_overlay(ip: Ipv4Addr, daemon: &mut Daemon) -> UdpSocket {
    tokio::time::timeout(WAIT, async {
        loop {
            match UdpSocket::bind(SocketAddrV4::new(ip, 0)).await {
                Ok(socket) => return socket,
                Err(err) if err.kind() == std::io::ErrorKind::AddrNotAvailable => {
                    daemon.assert_running();
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                Err(err) => panic!("绑定 {ip} 失败：{err}"),
            }
        }
    })
    .await
    .expect("meshorad 的虚拟网卡一直没就绪")
}

async fn round_trip(case: Case) {
    let daemon_secret = NodeSecret::generate();
    let echo_secret = NodeSecret::generate();
    let nodes = vec![daemon_secret.public_key(), echo_secret.public_key()];
    let relays = if case.relay_only {
        vec![start_relay(nodes.clone()).await]
    } else {
        Vec::new()
    };
    let coord = start_coord(&case, nodes, relays).await;
    start_echo_node(&case, &echo_secret, &coord).await;
    let mut daemon = Daemon::start(&case, &daemon_secret, &coord);

    let socket = bind_overlay(case.daemon_ip(), &mut daemon).await;
    let echo = case.echo();
    let mut buf = [0u8; 64];
    // 路径收敛之前报文会被丢掉，所以反复发，直到收到回应
    let (len, from) = tokio::time::timeout(WAIT, async {
        loop {
            daemon.assert_running();
            socket.send_to(b"through the tunnel", echo).await.unwrap();
            let wait = Duration::from_millis(500);
            if let Ok(received) = tokio::time::timeout(wait, socket.recv_from(&mut buf)).await {
                return received.unwrap();
            }
        }
    })
    .await
    .expect("经隧道等回应超时");
    assert_eq!(from, SocketAddr::V4(echo));
    assert_eq!(&buf[..len], b"through the tunnel");

    let expected = if case.relay_only { "Relay" } else { "Direct" };
    assert_eq!(daemon.last_path().as_deref(), Some(expected));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要管理员权限，Windows 上还要 wintun.dll。用 cargo test -p meshorad -- --ignored 跑"]
async fn udp_round_trip_directly() {
    round_trip(Case {
        relay_only: false,
        subnet: 99,
        suffix: 'd',
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要管理员权限，Windows 上还要 wintun.dll。用 cargo test -p meshorad -- --ignored 跑"]
async fn udp_round_trip_via_relay() {
    round_trip(Case {
        relay_only: true,
        subnet: 98,
        suffix: 'r',
    })
    .await;
}
