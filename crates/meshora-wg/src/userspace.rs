//! 把 [`Engine`] 接到真实 I/O 上的驱动：一个 UDP socket、一对通往虚拟网卡的 channel、一个定时器。
//!
//! 虚拟网卡在这里只是两个 channel —— 读到的报文从 [`TunChannels::from_tun`] 进来，
//! 要写的报文从 [`TunChannels::to_tun`] 出去。真正的网卡由 meshora-tun 接上；
//! 测试里直接拿 channel 当网卡，不需要 root。
//!
//! 经中继的报文走子模块 `relay` 里的客户端：每个中继一条长连接，第一次用到时建立。

mod relay;

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use meshora_types::{NodeKey, NodeSecret, Path};
use tokio::net::UdpSocket;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinHandle};
use tracing::{debug, warn};

use crate::engine::{Action, Engine, Link, Transmit};
use meshora_dataplane::{
    DataPlane, DataPlaneError, DatagramKind, EventSink, PeerSet, PeerStatus, classify,
};

/// 定时器的间隔。boringtun 的计时精度是秒级，250 毫秒足够。
const TICK: Duration = Duration::from_millis(250);

/// 通往虚拟网卡的两个方向。
pub struct TunChannels {
    /// 从虚拟网卡读到的 IP 报文。
    pub from_tun: mpsc::Receiver<Vec<u8>>,
    /// 要写进虚拟网卡的 IP 报文。
    pub to_tun: mpsc::Sender<Vec<u8>>,
}

/// 发往一个中继的报文队列。满了就丢，和 UDP 一样
const RELAY_QUEUE: usize = 256;

type RelayOutbox = mpsc::Sender<(NodeKey, Vec<u8>)>;

struct Shared {
    engine: Mutex<Engine>,
    socket: UdpSocket,
    /// 同一个 socket 的另一个句柄，给同步方法用。tokio 的 try_send_to 要等 reactor
    /// 先报告过可写才能发，刚绑定的 socket 第一次调用必然 WouldBlock
    sync_socket: std::net::UdpSocket,
    to_tun: mpsc::Sender<Vec<u8>>,
    events: Arc<dyn EventSink>,
    /// 连中继要用本机私钥认证自己（R6）。私钥仍然在数据面里，没出去
    secret: NodeSecret,
    runtime: Handle,
    relays: Mutex<HashMap<(NodeKey, SocketAddr), RelayOutbox>>,
    /// 中继连接任务，数据面停下时一并停掉
    background: Mutex<Vec<AbortHandle>>,
}

impl Shared {
    fn engine(&self) -> MutexGuard<'_, Engine> {
        // 引擎在持锁时 panic 过，状态就不可信了，不能接着用
        self.engine.lock().expect("数据面引擎在持锁时 panic 过")
    }

    /// 在异步任务里发：socket 暂时写不进去就等一等
    async fn transmit(self: &Arc<Self>, t: Transmit) {
        match t.link {
            Link::Direct(addr) => {
                if let Err(err) = self.socket.send_to(&t.datagram, addr).await {
                    debug!(%addr, %err, "发往直连地址失败");
                }
            }
            Link::Relay { relay, addr, peer } => self.relay_send(relay, addr, peer, t.datagram),
        }
    }

    /// 在同步方法里发：写不进去就丢，和 UDP 本身一样不保证送达
    fn try_transmit(self: &Arc<Self>, t: Transmit) {
        match t.link {
            Link::Direct(addr) => {
                if let Err(err) = self.sync_socket.send_to(&t.datagram, addr) {
                    debug!(%addr, %err, "发往直连地址失败");
                }
            }
            Link::Relay { relay, addr, peer } => self.relay_send(relay, addr, peer, t.datagram),
        }
    }

    /// 经中继把报文交给 `peer`。第一次用到这个中继时建立连接；连接还没建好、
    /// 正在重连或者队列满了，报文就丢掉 —— WireGuard 自己会重传握手
    fn relay_send(
        self: &Arc<Self>,
        relay: NodeKey,
        addr: SocketAddr,
        peer: NodeKey,
        datagram: Vec<u8>,
    ) {
        let mut relays = self.relays.lock().expect("中继表在持锁时 panic 过");
        let outbox = relays.entry((relay, addr)).or_insert_with(|| {
            let (tx, rx) = mpsc::channel(RELAY_QUEUE);
            let task = self
                .runtime
                .spawn(relay::run(Arc::clone(self), relay, addr, rx));
            self.background
                .lock()
                .expect("后台任务表在持锁时 panic 过")
                .push(task.abort_handle());
            tx
        });
        if outbox.try_send((peer, datagram)).is_err() {
            debug!(%relay, "中继队列满了，丢弃一个报文");
        }
    }

    async fn act(self: &Arc<Self>, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Transmit(t) => self.transmit(t).await,
                Action::WriteTun(packet) => {
                    // 网卡那一头关了，说明整个节点在退出
                    if self.to_tun.send(packet).await.is_err() {
                        debug!("虚拟网卡已关闭，丢弃一个报文");
                    }
                }
                Action::Event(event) => self.events.emit(event),
            }
        }
    }
}

/// 跑在 tokio 上的用户态数据面，[`DataPlane`] 契约的实现。
///
/// 丢掉它就停掉所有后台任务。
pub struct UserspaceDataPlane {
    shared: Arc<Shared>,
    tasks: Vec<JoinHandle<()>>,
}

impl UserspaceDataPlane {
    /// 在当前 tokio 运行时里启动。
    ///
    /// `socket` 由调用方绑定好：它就是 WireGuard 和控制报文共用的那一个（不变量 4）。
    /// 私钥交进来之后就拿不出去了（不变量 1）。
    pub fn start(
        secret: &NodeSecret,
        socket: std::net::UdpSocket,
        tun: TunChannels,
        events: Arc<dyn EventSink>,
    ) -> io::Result<Self> {
        socket.set_nonblocking(true)?;
        let sync_socket = socket.try_clone()?;
        // 句柄各自设一遍：Windows 上复制出来的句柄不保证继承非阻塞模式
        sync_socket.set_nonblocking(true)?;
        let shared = Arc::new(Shared {
            engine: Mutex::new(Engine::new(secret)),
            socket: UdpSocket::from_std(socket)?,
            sync_socket,
            to_tun: tun.to_tun,
            events,
            secret: secret.clone(),
            runtime: Handle::current(),
            relays: Mutex::new(HashMap::new()),
            background: Mutex::new(Vec::new()),
        });
        let tasks = vec![
            tokio::spawn(receive_loop(Arc::clone(&shared))),
            tokio::spawn(tun_loop(Arc::clone(&shared), tun.from_tun)),
            tokio::spawn(timer_loop(Arc::clone(&shared))),
        ];
        Ok(Self { shared, tasks })
    }

    /// 本机身份。
    pub fn local_key(&self) -> NodeKey {
        self.shared.engine().local_key()
    }

    /// 共享 socket 绑定的本地地址。
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.shared.socket.local_addr()
    }
}

impl Drop for UserspaceDataPlane {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        // 中继连接任务各自持有共享状态，不停掉它们，状态就永远释放不了
        if let Ok(background) = self.shared.background.lock() {
            for task in background.iter() {
                task.abort();
            }
        }
    }
}

impl DataPlane for UserspaceDataPlane {
    fn apply(&self, peers: &PeerSet) -> Result<(), DataPlaneError> {
        self.shared.engine().apply(peers)
    }

    fn set_path(&self, peer: &NodeKey, path: Path) -> Result<(), DataPlaneError> {
        let out = self.shared.engine().set_path(peer, path, Instant::now())?;
        for t in out {
            self.shared.try_transmit(t);
        }
        Ok(())
    }

    fn send_control(&self, to: SocketAddr, datagram: &[u8]) -> Result<(), DataPlaneError> {
        if classify(datagram) != DatagramKind::Control {
            return Err(DataPlaneError::NotControlDatagram);
        }
        self.shared.sync_socket.send_to(datagram, to)?;
        Ok(())
    }

    fn status(&self) -> Vec<PeerStatus> {
        self.shared.engine().status()
    }
}

async fn receive_loop(shared: Arc<Shared>) {
    let mut buf = vec![0u8; u16::MAX as usize];
    loop {
        let (len, from) = match shared.socket.recv_from(&mut buf).await {
            Ok(received) => received,
            // Windows 上发往已关闭端口的报文会让下一次 recv 报 ConnectionReset，
            // 那只是对端的 ICMP 回音，不是这个 socket 坏了
            Err(err) => {
                debug!(%err, "接收出错，继续");
                continue;
            }
        };
        let actions = shared
            .engine()
            .inbound(&buf[..len], Link::Direct(from), Instant::now());
        shared.act(actions).await;
    }
}

async fn tun_loop(shared: Arc<Shared>, mut from_tun: mpsc::Receiver<Vec<u8>>) {
    while let Some(packet) = from_tun.recv().await {
        let transmit = shared.engine().outbound(&packet, Instant::now());
        if let Some(t) = transmit {
            shared.transmit(t).await;
        }
    }
    warn!("虚拟网卡已关闭，不再读取出站报文");
}

async fn timer_loop(shared: Arc<Shared>) {
    let mut interval = tokio::time::interval(TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        let out = shared.engine().timers(Instant::now());
        for t in out {
            shared.transmit(t).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::*;
    use crate::testutil::ipv4;
    use meshora_dataplane::{CONTROL_MAGIC, Event, PeerConfig};

    const WAIT: Duration = Duration::from_secs(5);

    struct Node {
        key: NodeKey,
        ip: IpAddr,
        addr: SocketAddr,
        dataplane: UserspaceDataPlane,
        /// 往它的"虚拟网卡"里塞报文
        tun_in: mpsc::Sender<Vec<u8>>,
        /// 它写进"虚拟网卡"的报文
        tun_out: mpsc::Receiver<Vec<u8>>,
        events: mpsc::UnboundedReceiver<Event>,
    }

    async fn node(last_octet: u8) -> Node {
        let secret = NodeSecret::generate();
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = socket.local_addr().unwrap();
        let (tun_in, from_tun) = mpsc::channel(64);
        let (to_tun, tun_out) = mpsc::channel(64);
        let (events_tx, events) = mpsc::unbounded_channel();
        let sink = move |event| {
            let _ = events_tx.send(event);
        };
        Node {
            key: secret.public_key(),
            ip: IpAddr::from([100, 64, 0, last_octet]),
            addr,
            dataplane: UserspaceDataPlane::start(
                &secret,
                socket,
                TunChannels { from_tun, to_tun },
                Arc::new(sink),
            )
            .unwrap(),
            tun_in,
            tun_out,
            events,
        }
    }

    fn peer_of(node: &Node) -> PeerConfig {
        PeerConfig {
            key: node.key,
            allowed_ips: vec![ipnet::IpNet::from(node.ip)],
            keepalive: None,
        }
    }

    async fn next_packet(node: &mut Node) -> Vec<u8> {
        tokio::time::timeout(WAIT, node.tun_out.recv())
            .await
            .expect("等报文超时")
            .expect("虚拟网卡 channel 关了")
    }

    async fn next_event(node: &mut Node) -> Event {
        tokio::time::timeout(WAIT, node.events.recv())
            .await
            .expect("等事件超时")
            .expect("事件 channel 关了")
    }

    #[tokio::test]
    async fn ping_and_pong_over_loopback_udp() {
        let mut a = node(1).await;
        let mut b = node(2).await;
        a.dataplane
            .apply(&PeerSet::new([peer_of(&b)]).unwrap())
            .unwrap();
        b.dataplane
            .apply(&PeerSet::new([peer_of(&a)]).unwrap())
            .unwrap();
        a.dataplane.set_path(&b.key, Path::Direct(b.addr)).unwrap();
        b.dataplane.set_path(&a.key, Path::Direct(a.addr)).unwrap();

        let ping = ipv4(a.ip, b.ip, b"ping");
        a.tun_in.send(ping.clone()).await.unwrap();
        assert_eq!(next_packet(&mut b).await, ping);

        let pong = ipv4(b.ip, a.ip, b"pong");
        b.tun_in.send(pong.clone()).await.unwrap();
        assert_eq!(next_packet(&mut a).await, pong);

        // 两边都报告了握手完成，路径是对方的直连地址
        assert!(matches!(
            next_event(&mut a).await,
            Event::HandshakeCompleted { peer, via: Path::Direct(addr) } if peer == b.key && addr == b.addr
        ));
        assert!(matches!(
            next_event(&mut b).await,
            Event::HandshakeCompleted { peer, via: Path::Direct(addr) } if peer == a.key && addr == a.addr
        ));

        let status = a.dataplane.status();
        assert_eq!(status.len(), 1);
        assert!(status[0].last_handshake.is_some());
        assert!(status[0].tx_bytes > 0 && status[0].rx_bytes > 0);
    }

    /// 反复从 `from` 发，直到 `to` 收到：经中继时，头几个报文可能赶在中继连接建好之前被丢掉
    async fn deliver(from: &Node, to: &mut Node, packet: Vec<u8>) -> Vec<u8> {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                from.tun_in.send(packet.clone()).await.unwrap();
                if let Ok(Some(received)) =
                    tokio::time::timeout(Duration::from_millis(200), to.tun_out.recv()).await
                {
                    return received;
                }
            }
        })
        .await
        .expect("经中继没能送达")
    }

    #[tokio::test]
    async fn ping_and_pong_through_a_relay() {
        let mut a = node(1).await;
        let mut b = node(2).await;
        let relay_secret = NodeSecret::generate();
        let relay = relay_secret.public_key();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay_addr = listener.local_addr().unwrap();
        tokio::spawn(meshora_relay::serve(
            meshora_relay::Config {
                secret: relay_secret,
                nodes: vec![a.key, b.key],
            },
            listener,
        ));

        a.dataplane
            .apply(&PeerSet::new([peer_of(&b)]).unwrap())
            .unwrap();
        b.dataplane
            .apply(&PeerSet::new([peer_of(&a)]).unwrap())
            .unwrap();
        let via = Path::Relay {
            relay,
            addr: relay_addr,
        };
        a.dataplane.set_path(&b.key, via).unwrap();
        b.dataplane.set_path(&a.key, via).unwrap();

        let ping = ipv4(a.ip, b.ip, b"via relay");
        assert_eq!(deliver(&a, &mut b, ping.clone()).await, ping);
        let pong = ipv4(b.ip, a.ip, b"and back");
        assert_eq!(deliver(&b, &mut a, pong.clone()).await, pong);

        // 握手是经中继完成的
        let event = next_event(&mut a).await;
        assert!(
            matches!(event, Event::HandshakeCompleted { peer, via: Path::Relay { .. } } if peer == b.key),
            "{event:?}"
        );
    }

    #[tokio::test]
    async fn control_datagrams_share_the_wireguard_socket() {
        let a = node(1).await;
        let mut b = node(2).await;
        let mut datagram = CONTROL_MAGIC.to_vec();
        datagram.extend_from_slice(b"ping?");
        a.dataplane.send_control(b.addr, &datagram).unwrap();

        // 来源地址就是 A 的 WireGuard socket —— 同一个端口
        assert_eq!(
            next_event(&mut b).await,
            Event::ControlDatagram {
                from: a.addr,
                datagram
            }
        );
    }

    #[tokio::test]
    async fn send_control_refuses_anything_without_the_magic() {
        let a = node(1).await;
        let b = node(2).await;
        let mut fake_wireguard = vec![0u8; 32];
        fake_wireguard[0] = 4;
        let result = a.dataplane.send_control(b.addr, &fake_wireguard);
        assert!(matches!(result, Err(DataPlaneError::NotControlDatagram)));
    }

    #[tokio::test]
    async fn dropping_the_dataplane_stops_its_tasks() {
        let a = node(1).await;
        let tun_in = a.tun_in.clone();
        drop(a.dataplane);
        // 读网卡的任务停了，channel 的接收端随之释放
        tokio::time::timeout(WAIT, tun_in.closed())
            .await
            .expect("任务没有停");
    }
}
