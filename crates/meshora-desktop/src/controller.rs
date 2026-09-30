//! 节点的启停和状态汇总。界面只管显示和转发按钮，决定都在这里做。
//!
//! 同一时刻最多一个节点在跑，由一个后台任务拥有：连接、断开都是先停掉旧任务、等它收拾干净
//! （虚拟网卡删掉、端口放开），再起新的。界面每秒来要一次 [`Overview`]。

use std::io;
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use meshora_control::{ControlError, Status};
use meshora_dataplane::PeerStatus;
use meshora_types::{NodeSecret, Path};
use meshorad::{NetworkCode, Node, Options, StartError};
use serde::Serialize;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::logs::LogBuffer;
use crate::store::{Settings, Store};

/// 先试这个端口，被占了（比如同时开着 meshorad）就让系统挑一个。固定端口方便在路由器上做端口转发
const PORT: u16 = 41641;
/// 虚拟网卡的名字，在系统的"网络连接"里看得到
const TUN_NAME: &str = "Meshora";
const MTU: u16 = 1280;
const KEEPALIVE: u16 = 25;
/// "让游戏的广播走 Meshora"打开时虚拟网卡的跃点数：1 比系统自动给任何网卡的都小
const BROADCAST_METRIC: u32 = 1;
/// 多久没握手就不算在线。WireGuard 的会话 180 秒不续就作废，有 keepalive 时两分钟内必然续上
const ONLINE_WINDOW: Duration = Duration::from_secs(180);
/// 界面状态多久刷新一次
const REFRESH: Duration = Duration::from_secs(1);
/// 停一个节点最多等多久
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

/// 出了什么问题。界面按种类给出不同的提示。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FailureKind {
    /// 协调服务的主机名解析不出来。
    Resolve,
    /// 连不上协调服务。
    Unreachable,
    /// 协调服务不认本机：还没被加进名单。
    Rejected,
    /// 虚拟网卡建不起来。
    Tun,
    /// UDP 端口绑不上。
    Bind,
    /// 连上之后控制面退出了。
    Stopped,
    /// 别的。
    Other,
}

/// 一次失败。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Failure {
    /// 种类。
    pub kind: FailureKind,
    /// 给人看的说明。
    pub message: String,
}

impl Failure {
    fn new(kind: FailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    fn from_start(err: StartError) -> Self {
        let kind = match &err {
            StartError::Register(ControlError::Rejected(_)) => FailureKind::Rejected,
            StartError::Register(_) => FailureKind::Unreachable,
            StartError::Tun(..) => FailureKind::Tun,
            StartError::Bind(..) => FailureKind::Bind,
            StartError::DataPlane(_) => FailureKind::Other,
        };
        Self::new(kind, err.to_string())
    }
}

/// 节点在哪个阶段。
#[derive(Clone, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    Connecting,
    Connected,
    Failed(Failure),
}

/// 本机在网里的样子。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Me {
    /// overlay 地址：朋友在游戏里输的就是它。
    pub ip: String,
    /// 网段前缀长度。
    pub prefix: u8,
    /// 虚拟网卡在系统里的名字。
    pub tun: String,
}

/// 一个 peer 在界面上的一行。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerRow {
    /// 公钥。
    pub id: String,
    /// overlay 地址。
    pub ip: String,
    /// 走哪条路：`direct`、`relay`，还没选出来是 `pending`。
    pub route: &'static str,
    /// 直连测到的往返时间（毫秒）。
    pub rtt_ms: Option<u64>,
    /// 最近 180 秒内握过手（WireGuard 的会话不续就作废的时长）。
    pub online: bool,
    /// 收到的字节数。
    pub rx: u64,
    /// 发出的字节数。
    pub tx: u64,
}

/// 界面要的一切，每秒一份。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    /// 客户端版本。
    pub version: &'static str,
    /// 本机公钥：网络主人要把它加进名单。
    pub id: String,
    /// 保存着的网络码。
    pub network: Option<String>,
    /// 见 [`Settings::prefer_broadcast`]。
    pub prefer_broadcast: bool,
    /// 见 [`Settings::auto_connect`]。
    pub auto_connect: bool,
    /// `idle`、`connecting`、`connected`、`failed`。
    pub phase: &'static str,
    /// 失败时的原因。
    pub error: Option<Failure>,
    /// 连上之后本机的地址。
    pub me: Option<Me>,
    /// 和协调服务的连接是否连着。
    pub coord_connected: bool,
    /// 网里的其他人。
    pub peers: Vec<PeerRow>,
}

/// 连上之后的快照
#[derive(Clone, Debug, Default)]
struct Snapshot {
    me: Option<Me>,
    coord_connected: bool,
    peers: Vec<PeerRow>,
}

struct Runner {
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

struct State {
    settings: Settings,
    phase: Phase,
    snapshot: Snapshot,
    runner: Option<Runner>,
}

struct Shared {
    secret: NodeSecret,
    store: Store,
    logs: LogBuffer,
    state: Mutex<State>,
    /// 连接、断开、改设置排队做，免得两个操作交错着起停节点
    ops: tokio::sync::Mutex<()>,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn save(&self, settings: &Settings) {
        if let Err(err) = self.store.save_settings(settings) {
            warn!(%err, "保存设置失败");
        }
    }
}

/// 客户端的大脑。克隆出来的都是同一个。
#[derive(Clone)]
pub struct Controller {
    shared: Arc<Shared>,
}

impl Controller {
    /// 从这个目录读私钥和设置。私钥读不出来（文件坏了）就报错 —— 不能悄悄换一个身份。
    pub fn new(dir: impl Into<PathBuf>, logs: LogBuffer) -> io::Result<Self> {
        let store = Store::new(dir);
        let secret = store.load_or_create_key()?;
        let settings = store.load_settings();
        Ok(Self {
            shared: Arc::new(Shared {
                secret,
                store,
                logs,
                state: Mutex::new(State {
                    settings,
                    phase: Phase::Idle,
                    snapshot: Snapshot::default(),
                    runner: None,
                }),
                ops: tokio::sync::Mutex::new(()),
            }),
        })
    }

    /// 此刻的全部状态。
    pub fn overview(&self) -> Overview {
        let state = self.shared.state();
        let (phase, error) = match &state.phase {
            Phase::Idle => ("idle", None),
            Phase::Connecting => ("connecting", None),
            Phase::Connected => ("connected", None),
            Phase::Failed(failure) => ("failed", Some(failure.clone())),
        };
        Overview {
            version: env!("CARGO_PKG_VERSION"),
            id: self.shared.secret.public_key().to_string(),
            network: state.settings.network.clone(),
            prefer_broadcast: state.settings.prefer_broadcast,
            auto_connect: state.settings.auto_connect,
            phase,
            error,
            me: state.snapshot.me.clone(),
            coord_connected: state.snapshot.coord_connected,
            peers: state.snapshot.peers.clone(),
        }
    }

    /// 最近的日志。
    pub fn logs(&self) -> Vec<String> {
        self.shared.logs.lines()
    }

    /// 连到一个网络。`code` 为 `None` 时用保存着的网络码。
    ///
    /// 网络码不对时直接返回错误，什么都不动；否则保存网络码、在后台开始连接，马上返回 ——
    /// 连得怎么样看 [`overview`](Self::overview)。
    pub async fn connect(&self, code: Option<String>) -> Result<(), String> {
        let _op = self.shared.ops.lock().await;
        self.connect_locked(code).await
    }

    async fn connect_locked(&self, code: Option<String>) -> Result<(), String> {
        let text = match code {
            Some(code) => code,
            None => self
                .shared
                .state()
                .settings
                .network
                .clone()
                .ok_or("还没有网络码")?,
        };
        let network: NetworkCode = text.parse().map_err(|err| format!("{err}"))?;

        self.stop_runner().await;
        let (stop, stop_rx) = oneshot::channel();
        let mut state = self.shared.state();
        state.settings.network = Some(network.to_string());
        self.shared.save(&state.settings);
        let metric = state.settings.prefer_broadcast.then_some(BROADCAST_METRIC);
        state.phase = Phase::Connecting;
        state.snapshot = Snapshot::default();
        let task = tokio::spawn(run(Arc::clone(&self.shared), network, metric, stop_rx));
        state.runner = Some(Runner { stop, task });
        Ok(())
    }

    /// 断开。虚拟网卡随之删掉。
    pub async fn disconnect(&self) {
        let _op = self.shared.ops.lock().await;
        self.stop_runner().await;
        let mut state = self.shared.state();
        state.phase = Phase::Idle;
        state.snapshot = Snapshot::default();
    }

    /// 断开并忘掉网络码。
    pub async fn forget(&self) {
        let _op = self.shared.ops.lock().await;
        self.stop_runner().await;
        let mut state = self.shared.state();
        state.phase = Phase::Idle;
        state.snapshot = Snapshot::default();
        state.settings.network = None;
        self.shared.save(&state.settings);
    }

    /// 改"让游戏的广播走 Meshora"。正连着的话按新设置重连 —— 跃点数是建网卡时设的。
    pub async fn set_prefer_broadcast(&self, on: bool) -> Result<(), String> {
        let _op = self.shared.ops.lock().await;
        let active = {
            let mut state = self.shared.state();
            if state.settings.prefer_broadcast == on {
                return Ok(());
            }
            state.settings.prefer_broadcast = on;
            self.shared.save(&state.settings);
            matches!(state.phase, Phase::Connecting | Phase::Connected)
        };
        if active {
            self.connect_locked(None).await?;
        }
        Ok(())
    }

    /// 改"打开时自动连接"。
    pub fn set_auto_connect(&self, on: bool) {
        let mut state = self.shared.state();
        state.settings.auto_connect = on;
        self.shared.save(&state.settings);
    }

    /// 客户端刚打开：按设置自动连接上次的网络。
    pub async fn start_up(&self) {
        let wanted = {
            let state = self.shared.state();
            state.settings.auto_connect && state.settings.network.is_some()
        };
        if wanted && let Err(err) = self.connect(None).await {
            warn!(%err, "自动连接失败");
        }
    }

    /// 停掉后台任务并等它收拾完
    async fn stop_runner(&self) {
        let runner = self.shared.state().runner.take();
        if let Some(Runner { stop, mut task }) = runner {
            let _ = stop.send(());
            if tokio::time::timeout(STOP_TIMEOUT, &mut task).await.is_err() {
                warn!("节点没能按时停下，强行结束");
                task.abort();
            }
        }
    }
}

/// 后台任务：启动节点，然后每秒刷新一次快照，直到被叫停或者控制面退出
async fn run(
    shared: Arc<Shared>,
    network: NetworkCode,
    metric: Option<u32>,
    mut stop: oneshot::Receiver<()>,
) {
    let started = tokio::select! {
        started = start(&shared.secret, &network, metric) => started,
        _ = &mut stop => return,
    };
    let mut node = match started {
        Ok(node) => node,
        Err(failure) => {
            warn!(message = %failure.message, "连接失败");
            shared.state().phase = Phase::Failed(failure);
            return;
        }
    };
    info!(network = %network.host, ip = %node.welcome().overlay_ip, "已连上");
    {
        let mut state = shared.state();
        state.phase = Phase::Connected;
        state.snapshot = snapshot(&node);
    }

    let mut tick = tokio::time::interval(REFRESH);
    loop {
        tokio::select! {
            _ = &mut stop => return,
            _ = tick.tick() => {}
        }
        if !node.is_running() {
            let message = match node.wait().await {
                Ok(()) => "节点停了".to_string(),
                Err(err) => format!("和网络的连接断了：{err}"),
            };
            warn!(%message);
            let mut state = shared.state();
            state.phase = Phase::Failed(Failure::new(FailureKind::Stopped, message));
            state.snapshot = Snapshot::default();
            return;
        }
        shared.state().snapshot = snapshot(&node);
    }
}

async fn start(
    secret: &NodeSecret,
    network: &NetworkCode,
    metric: Option<u32>,
) -> Result<Node, Failure> {
    let coord = network.resolve().await.map_err(|err| {
        Failure::new(
            FailureKind::Resolve,
            format!("找不到协调服务 {}：{err}", network.host),
        )
    })?;
    let options = |port| Options {
        secret: secret.clone(),
        coord,
        coord_key: network.coord_key,
        invite: network.invite,
        port,
        tun: TUN_NAME.into(),
        mtu: MTU,
        metric,
        keepalive: NonZeroU16::new(KEEPALIVE),
        relay_only: false,
    };
    let started = match meshorad::start(options(PORT)).await {
        Err(StartError::Bind(_, err)) if err.kind() == io::ErrorKind::AddrInUse => {
            info!(port = PORT, "端口被占用，让系统挑一个");
            meshorad::start(options(0)).await
        }
        other => other,
    };
    started.map_err(Failure::from_start)
}

fn snapshot(node: &Node) -> Snapshot {
    let welcome = node.welcome();
    let status = node.status();
    let status = status.borrow();
    Snapshot {
        me: Some(Me {
            ip: welcome.overlay_ip.to_string(),
            prefix: welcome.prefix_len,
            tun: node.tun_name().to_owned(),
        }),
        coord_connected: status.coord_connected,
        peers: rows(&status, &node.peers(), Instant::now()),
    }
}

/// 把控制面和数据面各自知道的拼成界面上的一行行：地址、路径、延迟来自控制面，
/// 握手和流量来自数据面
fn rows(status: &Status, peers: &[PeerStatus], now: Instant) -> Vec<PeerRow> {
    status
        .peers
        .iter()
        .map(|view| {
            let data = peers.iter().find(|peer| peer.key == view.key);
            PeerRow {
                id: view.key.to_string(),
                ip: view.overlay_ip.to_string(),
                route: match view.path {
                    Some(Path::Direct(_)) => "direct",
                    Some(Path::Relay { .. }) => "relay",
                    None => "pending",
                },
                rtt_ms: view
                    .rtt
                    .map(|rtt| u64::try_from(rtt.as_millis()).unwrap_or(u64::MAX)),
                online: data
                    .and_then(|peer| peer.last_handshake)
                    .is_some_and(|at| now.saturating_duration_since(at) < ONLINE_WINDOW),
                rx: data.map_or(0, |peer| peer.rx_bytes),
                tx: data.map_or(0, |peer| peer.tx_bytes),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddr};

    use meshora_control::PeerView;
    use meshora_types::NodeKey;
    use tokio::net::TcpListener;

    use super::*;
    use crate::store::tests::TempDir;

    fn key(byte: u8) -> NodeKey {
        NodeKey::from_bytes([byte; 32])
    }

    #[test]
    fn rows_combine_control_and_data_plane() {
        let now = Instant::now();
        let relay = Path::Relay {
            relay: key(9),
            addr: SocketAddr::from(([203, 0, 113, 9], 7444)),
        };
        let status = Status {
            coord_connected: true,
            peers: vec![
                PeerView {
                    key: key(1),
                    overlay_ip: Ipv4Addr::new(100, 64, 0, 1),
                    path: Some(Path::Direct(SocketAddr::from(([192, 0, 2, 1], 41641)))),
                    rtt: Some(Duration::from_micros(12_700)),
                },
                PeerView {
                    key: key(2),
                    overlay_ip: Ipv4Addr::new(100, 64, 0, 2),
                    path: Some(relay),
                    rtt: None,
                },
                PeerView {
                    key: key(3),
                    overlay_ip: Ipv4Addr::new(100, 64, 0, 3),
                    path: None,
                    rtt: None,
                },
            ],
        };
        let data = [
            PeerStatus {
                key: key(1),
                path: None,
                last_handshake: Some(now),
                rx_bytes: 10,
                tx_bytes: 20,
            },
            PeerStatus {
                key: key(2),
                path: Some(relay),
                last_handshake: Some(now),
                rx_bytes: 0,
                tx_bytes: 0,
            },
        ];
        let later = now + ONLINE_WINDOW;
        let rows_now = rows(&status, &data, now);
        assert_eq!(
            rows_now[0],
            PeerRow {
                id: key(1).to_string(),
                ip: "100.64.0.1".into(),
                route: "direct",
                rtt_ms: Some(12),
                online: true,
                rx: 10,
                tx: 20,
            }
        );
        assert_eq!(rows_now[1].route, "relay");
        assert_eq!(rows_now[1].rtt_ms, None);
        assert_eq!(rows_now[2].route, "pending");
        assert!(!rows_now[2].online, "数据面还不知道它");
        assert!(
            rows(&status, &data, later).iter().all(|row| !row.online),
            "太久没握手就不算在线"
        );
    }

    fn controller(dir: &TempDir) -> Controller {
        Controller::new(&dir.0, LogBuffer::default()).unwrap()
    }

    async fn phase_settles(controller: &Controller) -> Overview {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let overview = controller.overview();
                if overview.phase != "connecting" {
                    return overview;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("一直停在连接中")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_bad_network_code_changes_nothing() {
        let dir = TempDir::new("bad-code");
        let controller = controller(&dir);
        assert!(controller.connect(Some("hello".into())).await.is_err());
        let overview = controller.overview();
        assert_eq!(overview.phase, "idle");
        assert_eq!(overview.network, None);
        assert!(controller.connect(None).await.is_err(), "没保存过网络码");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_unreachable_coordinator_is_reported() {
        let dir = TempDir::new("unreachable");
        let controller = controller(&dir);
        // 先占一个端口再放掉：那里多半没人听
        let addr = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap();
        let code = NetworkCode::new(key(7), addr).to_string();
        controller.connect(Some(code.clone())).await.unwrap();

        let overview = phase_settles(&controller).await;
        assert_eq!(overview.phase, "failed");
        assert_eq!(overview.error.unwrap().kind, FailureKind::Unreachable);
        assert_eq!(
            overview.network.as_deref(),
            Some(code.as_str()),
            "网络码存下了"
        );
        assert_eq!(
            Store::new(&dir.0).load_settings().network.as_deref(),
            Some(code.as_str())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_node_outside_the_member_list_is_told_so() {
        let dir = TempDir::new("rejected");
        let controller = controller(&dir);
        let coord_secret = NodeSecret::generate();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(meshora_coord::serve(
            meshora_coord::Config {
                secret: coord_secret.clone(),
                // 名单里只有别人
                nodes: vec![key(1)],
                state: None,
                overlay: "100.64.0.0/10".parse().unwrap(),
                probe: None,
                relays: vec![],
            },
            listener,
            None,
        ));

        let code = NetworkCode::new(coord_secret.public_key(), addr).to_string();
        controller.connect(Some(code)).await.unwrap();
        let overview = phase_settles(&controller).await;
        assert_eq!(overview.error.unwrap().kind, FailureKind::Rejected);

        controller.forget().await;
        let overview = controller.overview();
        assert_eq!(overview.phase, "idle");
        assert_eq!(overview.network, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_network_code_with_an_invite_gets_past_registration() {
        let dir = TempDir::new("invite");
        let controller = controller(&dir);
        let coord_secret = NodeSecret::generate();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        // 名单是空的：只能凭邀请码进
        let coordinator = meshora_coord::Coordinator::new(meshora_coord::Config {
            secret: coord_secret.clone(),
            nodes: vec![],
            state: Some(dir.0.join("coord.state")),
            overlay: "100.64.0.0/10".parse().unwrap(),
            probe: None,
            relays: vec![],
        })
        .unwrap();
        let invite = coordinator.invite().unwrap();
        tokio::spawn(coordinator.serve(listener, None));

        let mut code = NetworkCode::new(coord_secret.public_key(), addr);
        code.invite = Some(invite);
        controller.connect(Some(code.to_string())).await.unwrap();
        let overview = phase_settles(&controller).await;
        // 过了注册这一关。之后建不建得起网卡取决于有没有管理员权限和 wintun.dll：
        // 普通用户跑测试时停在建网卡，CI 的 Windows 机器上能一直连通
        match overview.error {
            None => assert_eq!(overview.phase, "connected"),
            Some(failure) => assert_eq!(failure.kind, FailureKind::Tun, "{}", failure.message),
        }
        assert!(
            overview.network.unwrap().ends_with(&format!("#{invite}")),
            "存下的网络码带着邀请码，下次自动连也用得上"
        );
        controller.forget().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn disconnecting_while_connecting_goes_idle() {
        let dir = TempDir::new("disconnect");
        let controller = controller(&dir);
        // 一个只接受连接、从不说话的"协调服务"：连接会一直卡在握手上
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                held.push(stream);
            }
        });
        controller
            .connect(Some(NetworkCode::new(key(7), addr).to_string()))
            .await
            .unwrap();
        assert_eq!(controller.overview().phase, "connecting");
        controller.disconnect().await;
        assert_eq!(controller.overview().phase, "idle");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn settings_are_saved() {
        let dir = TempDir::new("controller-settings");
        let controller = controller(&dir);
        controller.set_prefer_broadcast(false).await.unwrap();
        controller.set_auto_connect(false);
        let saved = Store::new(&dir.0).load_settings();
        assert!(!saved.prefer_broadcast);
        assert!(!saved.auto_connect);
        let overview = controller.overview();
        assert!(!overview.prefer_broadcast);
        assert_eq!(overview.id.len(), 44);
    }
}
