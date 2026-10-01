//! 节点的启停和状态汇总。界面只管显示和转发按钮，决定都在这里做。
//!
//! 同一时刻最多一个节点在跑，由一个后台任务拥有：连接、断开都是先停掉旧任务、等它收拾干净
//! （虚拟网卡删掉、端口放开），再起新的。界面每秒来要一次 [`Overview`]。

use std::io;
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use meshora_control::{AdminHandle, AdminRequest, ControlError, Entry, NameSetter, Status};
use meshora_dataplane::PeerStatus;
use meshora_types::{NodeSecret, Path};
use meshorad::{Hosting, NetworkCode, Node, Options, StartError, TunOpener};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::host::{self, Host, HostInfo};
use crate::logs::LogBuffer;
use crate::store::{MAX_NAME_CHARS, Settings, Store};

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
/// 官方服务器：托管很多网络的协调服务，客户端"建网络"默认用它。`公钥@地址:端口`。
///
/// 部署在一台阿里云的服务器上（`scripts/deploy-hub.sh`），换服务器或换私钥时改这里。
/// 设置里的 [`Settings::official_server`] 可以填另一个顶替它。
pub const OFFICIAL_SERVER: Option<&str> =
    Some("3hxhTS9dwMrNCqPzJzqOheZMk8qYepYn9QMmxKhPkgU=@39.108.210.40:7443");

/// 设网络类别时，网卡刚建好、系统还没把它归到哪个网络，要隔一会儿再试。最多试这么久
#[cfg(windows)]
const PROFILE_PATIENCE: Duration = Duration::from_secs(60);

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
    /// 对方给自己起的名字，空串是没起。
    pub name: String,
    /// overlay 地址。
    pub ip: String,
    /// 走哪条路：`direct`、`relay`，还没选出来是 `pending`。
    pub route: &'static str,
    /// 直连测到的往返时间（毫秒）。
    pub rtt_ms: Option<u64>,
    /// 直连往返时间的抖动（毫秒）。
    pub jitter_ms: Option<u64>,
    /// 直连的丢包率（百分比）。
    pub loss_percent: Option<u8>,
    /// 最近 180 秒内握过手（WireGuard 的会话不续就作废的时长）。
    pub online: bool,
    /// 收到的字节数。
    pub rx: u64,
    /// 发出的字节数。
    pub tx: u64,
}

/// 网主看到的一个成员。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RosterMemberRow {
    /// 公钥。
    pub id: String,
    /// 名字。
    pub name: String,
    /// overlay 地址。
    pub ip: String,
    /// 此刻在线。
    pub online: bool,
    /// 是网主。
    pub owner: bool,
}

/// 网主看到的一个带限制的邀请码。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteRow {
    /// 带着这个邀请码的完整网络码，复制了直接能发。
    pub code: String,
    /// 只有邀请码本身（作废时用）。
    pub invite: String,
    /// 还能用几次。
    pub uses_left: Option<u32>,
    /// 过期时间（UNIX 秒）。
    pub expires: Option<u64>,
}

/// 网主看到的网络。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RosterView {
    /// 网络名。
    pub name: String,
    /// 带长期邀请码的网络码。
    pub code: Option<String>,
    /// 成员。
    pub members: Vec<RosterMemberRow>,
    /// 带限制的邀请码。
    pub invites: Vec<InviteRow>,
}

/// 在哪儿建网络。
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CreateAt {
    /// 官方服务器。
    Official,
    /// 自己的服务器：`公钥@地址:端口`。
    Server {
        /// 服务器地址。
        code: String,
    },
    /// 本机当主机：协调服务、中继跑在这台电脑上。
    ThisPc,
}

/// 网主在界面上能做的事。
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AdminAction {
    /// 把一个成员移出。
    Kick {
        /// 成员的公钥。
        id: String,
    },
    /// 新建一个带限制的邀请码。
    NewInvite {
        /// 能用几次。
        uses: Option<u32>,
        /// 几小时后过期。
        hours: Option<u32>,
    },
    /// 作废一个带限制的邀请码。
    RevokeInvite {
        /// 邀请码。
        invite: String,
    },
    /// 换掉长期邀请码。
    RotateInvite,
    /// 改网络名。
    Rename {
        /// 新名字。
        name: String,
    },
    /// 解散网络。
    Delete,
}

/// 界面要的一切，每秒一份。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    /// 客户端版本。
    pub version: &'static str,
    /// 跑在哪：`windows`、`android`。界面按它藏掉别的平台用不上的东西（标题栏、本机当主机、Windows 的网络设置）
    pub platform: &'static str,
    /// 本机公钥：网络主人要把它加进名单。
    pub id: String,
    /// 见 [`Settings::name`]。
    pub name: String,
    /// 保存着的网络码。
    pub network: Option<String>,
    /// 见 [`Settings::prefer_broadcast`]。
    pub prefer_broadcast: bool,
    /// 见 [`Settings::auto_connect`]。
    pub auto_connect: bool,
    /// 见 [`Settings::private_network`]。
    pub private_network: bool,
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
    /// 见 [`Settings::onboarded`]。
    pub onboarded: bool,
    /// 能建网络的官方服务器地址（设置里填的优先），还没有是 `None`。
    pub official_server: Option<String>,
    /// 自己添加的服务器。
    pub servers: Vec<String>,
    /// 本机是这个网络的网主时：成员和邀请码。
    pub roster: Option<RosterView>,
    /// 本机当主机时：朋友连不连得进来、公网地址。
    pub hosting: Option<HostInfo>,
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

/// 丢掉就停下的后台任务
struct Background(JoinHandle<()>);

impl Drop for Background {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct State {
    settings: Settings,
    phase: Phase,
    snapshot: Snapshot,
    runner: Option<Runner>,
    /// 正在设网卡的网络类别的任务。换一个设置、断开时就停掉旧的
    profile: Option<Background>,
    /// 节点跑着时改名字用
    renamer: Option<NameSetter>,
    /// 节点跑着时管理网络用
    admin: Option<AdminHandle>,
    /// 本机当主机时跑着的协调服务和中继。断开时留着（朋友之间照样通），离开网络时停掉
    host: Option<Arc<Host>>,
}

struct Shared {
    secret: NodeSecret,
    /// 怎么建虚拟网卡：安卓上请 VpnService 建，别处是 `None`（节点自己建）
    open_tun: Option<TunOpener>,
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
        Self::with_tun_opener(dir, logs, None)
    }

    /// 和 [`new`](Self::new) 一样，只是虚拟网卡由 `open_tun` 来建（安卓上是 VpnService）。
    pub fn with_tun_opener(
        dir: impl Into<PathBuf>,
        logs: LogBuffer,
        open_tun: Option<TunOpener>,
    ) -> io::Result<Self> {
        let store = Store::new(dir);
        let secret = store.load_or_create_key()?;
        let settings = store.load_settings();
        Ok(Self {
            shared: Arc::new(Shared {
                secret,
                open_tun,
                store,
                logs,
                state: Mutex::new(State {
                    settings,
                    phase: Phase::Idle,
                    snapshot: Snapshot::default(),
                    runner: None,
                    profile: None,
                    renamer: None,
                    admin: None,
                    host: None,
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
            platform: std::env::consts::OS,
            id: self.shared.secret.public_key().to_string(),
            name: state.settings.name.clone(),
            network: state.settings.network.clone(),
            prefer_broadcast: state.settings.prefer_broadcast,
            auto_connect: state.settings.auto_connect,
            private_network: state.settings.private_network,
            phase,
            error,
            me: state.snapshot.me.clone(),
            coord_connected: state.snapshot.coord_connected,
            peers: state.snapshot.peers.clone(),
            onboarded: state.settings.onboarded,
            official_server: official_server(&state.settings),
            servers: state.settings.servers.clone(),
            roster: roster_view(&state),
            hosting: state.host.as_ref().map(|host| host.info.clone()),
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
        // 换了一个网络：不再是本机当主机的那个，本机的主机停掉
        if state.settings.network.as_deref() != Some(network.to_string().as_str()) {
            state.settings.hosting = false;
            state.host = None;
        }
        state.settings.network = Some(network.to_string());
        self.shared.save(&state.settings);
        let hosting = state.settings.hosting;
        let metric = state.settings.prefer_broadcast.then_some(BROADCAST_METRIC);
        let name = state.settings.name.clone();
        state.phase = Phase::Connecting;
        state.snapshot = Snapshot::default();
        let entry = network.entry();
        let task = tokio::spawn(run(
            Arc::clone(&self.shared),
            network,
            entry,
            metric,
            name,
            hosting,
            stop_rx,
        ));
        state.runner = Some(Runner { stop, task });
        Ok(())
    }

    /// 在一台服务器上建一个网络，自己当网主。建好之后网络码存下来，和加入的网络一样用。
    ///
    /// 和 [`connect`](Self::connect) 一样马上返回，建得怎么样看 [`overview`](Self::overview)。
    pub async fn create(&self, at: CreateAt, name: String) -> Result<(), String> {
        let _op = self.shared.ops.lock().await;
        let here = at == CreateAt::ThisPc;
        let text = match at {
            CreateAt::Official => official_server(&self.shared.state().settings)
                .ok_or("官方服务器还没上线：先用你自己的服务器，或者在本机当主机")?,
            CreateAt::Server { code } => code,
            // 真正的地址等主机起来才知道，先占个位
            CreateAt::ThisPc => NetworkCode::new(
                self.shared.secret.public_key(),
                ([127, 0, 0, 1], host::COORD_PORT).into(),
            )
            .to_string(),
        };
        let server: NetworkCode = text.trim().parse().map_err(|err| format!("{err}"))?;
        if server.network.is_some() || server.invite.is_some() {
            return Err("这是一个网络的网络码，不是服务器地址：要加入它，用\"加入网络\"".into());
        }
        self.stop_runner().await;
        let (stop, stop_rx) = oneshot::channel();
        let mut state = self.shared.state();
        state.settings.hosting = here;
        if !here {
            state.host = None;
        }
        self.shared.save(&state.settings);
        let metric = state.settings.prefer_broadcast.then_some(BROADCAST_METRIC);
        let me = state.settings.name.clone();
        state.phase = Phase::Connecting;
        state.snapshot = Snapshot::default();
        let entry = Entry::Create { name };
        let task = tokio::spawn(run(
            Arc::clone(&self.shared),
            server,
            entry,
            metric,
            me,
            here,
            stop_rx,
        ));
        state.runner = Some(Runner { stop, task });
        Ok(())
    }

    /// 网主管理网络。新建、换掉的邀请码以完整网络码的形式返回，复制了直接能发。
    pub async fn admin(&self, action: AdminAction) -> Result<Option<String>, String> {
        let (handle, code) = {
            let state = self.shared.state();
            let handle = state.admin.clone().ok_or("还没连上网络")?;
            let code = state
                .settings
                .network
                .as_deref()
                .and_then(|text| text.parse::<NetworkCode>().ok())
                .ok_or("还没有网络码")?;
            (handle, code)
        };
        let delete = action == AdminAction::Delete;
        let rotate = action == AdminAction::RotateInvite;
        let request = match action {
            AdminAction::Kick { id } => AdminRequest::Kick {
                member: id.parse().map_err(|_| "成员的 ID 不对".to_string())?,
            },
            AdminAction::NewInvite { uses, hours } => AdminRequest::NewInvite { uses, hours },
            AdminAction::RevokeInvite { invite } => AdminRequest::RevokeInvite {
                code: invite.parse().map_err(|_| "邀请码不对".to_string())?,
            },
            AdminAction::RotateInvite => AdminRequest::RotateInvite,
            AdminAction::Rename { name } => AdminRequest::Rename { name },
            AdminAction::Delete => AdminRequest::Delete,
        };
        let invite = handle.request(request).await?;
        if delete {
            // 网络没了：断开、忘掉它
            self.forget().await;
            return Ok(None);
        }
        let full = invite.map(|invite| {
            let mut full = code.clone();
            full.invite = Some(invite);
            full.to_string()
        });
        if rotate && let Some(full) = &full {
            // 长期邀请码换了：存着的网络码也跟着换，下次分享的就是新的
            let mut state = self.shared.state();
            state.settings.network = Some(full.clone());
            self.shared.save(&state.settings);
        }
        Ok(full)
    }

    /// 走完了首次打开的引导。
    pub fn set_onboarded(&self) {
        let mut state = self.shared.state();
        state.settings.onboarded = true;
        self.shared.save(&state.settings);
    }

    /// 记下一台自己的服务器，建网络时可以选。
    pub fn add_server(&self, code: &str) -> Result<String, String> {
        let server: NetworkCode = code.trim().parse().map_err(|err| format!("{err}"))?;
        if server.network.is_some() || server.invite.is_some() {
            return Err("这是一个网络的网络码，不是服务器地址".into());
        }
        let text = server.to_string();
        let mut state = self.shared.state();
        if !state.settings.servers.contains(&text) {
            state.settings.servers.push(text.clone());
            self.shared.save(&state.settings);
        }
        Ok(text)
    }

    /// 忘掉一台自己的服务器。
    pub fn remove_server(&self, code: &str) {
        let mut state = self.shared.state();
        state.settings.servers.retain(|s| s != code);
        self.shared.save(&state.settings);
    }

    /// 断开。虚拟网卡随之删掉。
    pub async fn disconnect(&self) {
        let _op = self.shared.ops.lock().await;
        self.stop_runner().await;
        let mut state = self.shared.state();
        state.phase = Phase::Idle;
        state.snapshot = Snapshot::default();
    }

    /// 断开并忘掉网络码。本机当主机的话，主机也停掉（路由器上的端口映射一并删掉）。
    pub async fn forget(&self) {
        let _op = self.shared.ops.lock().await;
        self.stop_runner().await;
        let mut state = self.shared.state();
        state.phase = Phase::Idle;
        state.snapshot = Snapshot::default();
        state.settings.network = None;
        state.settings.hosting = false;
        state.host = None;
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

    /// 改"把 Meshora 设为专用网络"。连着的话马上按新设置改网卡，不用重连；
    /// 关掉时把网卡改回公用网络。
    pub fn set_private_network(&self, on: bool) {
        let mut state = self.shared.state();
        if state.settings.private_network == on {
            return;
        }
        state.settings.private_network = on;
        self.shared.save(&state.settings);
        if state.phase == Phase::Connected
            && let Some(me) = &state.snapshot.me
        {
            let tun = me.tun.clone();
            state.profile = Some(set_profile(tun, on));
        }
    }

    /// 改给网里别人看的名字。连着的话马上生效，不用重连。返回整理过的名字。
    pub fn set_name(&self, name: &str) -> String {
        let name: String = name.trim().chars().take(MAX_NAME_CHARS).collect();
        let mut state = self.shared.state();
        if state.settings.name != name {
            state.settings.name.clone_from(&name);
            self.shared.save(&state.settings);
            if let Some(renamer) = &state.renamer {
                renamer.set(name.clone());
            }
        }
        name
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
        let runner = {
            let mut state = self.shared.state();
            state.profile = None;
            state.renamer = None;
            state.admin = None;
            state.runner.take()
        };
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
    entry: Entry,
    metric: Option<u32>,
    name: String,
    hosting: bool,
    mut stop: oneshot::Receiver<()>,
) {
    // 本机当主机：先把本机的协调服务、中继起起来
    let host = if hosting {
        let started = tokio::select! {
            started = ensure_host(&shared) => started,
            _ = &mut stop => return,
        };
        match started {
            Ok(host) => Some(host),
            Err(message) => {
                warn!(%message, "本机当主机没起来");
                shared.state().phase = Phase::Failed(Failure::new(FailureKind::Other, message));
                return;
            }
        }
    } else {
        None
    };
    // 建网络：先在服务器上建好、把网络码存下来，再像加入一样连进去。
    // 本机当主机时，连的是局域网地址，存下来给朋友的是公网地址
    let (network, entry) = match entry {
        Entry::Create { name: title } => {
            let (dial, share) = match &host {
                Some(host) => (host.local.clone(), host.share.clone()),
                None => (network.clone(), network.server()),
            };
            let created = tokio::select! {
                created = create_remote(&shared, &dial, &share, &title, &name) => created,
                _ = &mut stop => return,
            };
            match created {
                Ok(code) => {
                    let entry = code.entry();
                    (code, entry)
                }
                Err(failure) => {
                    warn!(message = %failure.message, "建网络失败");
                    shared.state().phase = Phase::Failed(failure);
                    return;
                }
            }
        }
        other => (network, other),
    };
    // 本机当主机：自己用局域网地址连，网络 ID、邀请码照旧
    let (dial, extra) = match &host {
        Some(host) => {
            let mut dial = host.local.clone();
            dial.network = network.network;
            dial.invite = network.invite;
            (dial, host.hosting.clone())
        }
        None => (network.clone(), Hosting::default()),
    };
    let started = tokio::select! {
        started = start(&shared.secret, &dial, entry, metric, name, extra, shared.open_tun.clone()) => started,
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
        state.renamer = Some(node.name_setter());
        state.admin = Some(node.admin());
        // 关着的时候什么都不动：网卡是新建的，本来就是系统默认的类别
        if state.settings.private_network {
            state.profile = Some(set_profile(node.tun_name().to_owned(), true));
        }
    }

    let mut tick = tokio::time::interval(REFRESH);
    loop {
        tokio::select! {
            _ = &mut stop => return,
            _ = tick.tick() => {}
        }
        if !node.is_running() {
            let failure = match node.wait().await {
                Ok(()) => Failure::new(FailureKind::Stopped, "节点停了"),
                // 连着连着被协调服务拒了：多半是被移出了网络
                Err(err @ ControlError::Rejected(_)) => {
                    Failure::new(FailureKind::Rejected, err.to_string())
                }
                Err(err) => Failure::new(FailureKind::Stopped, format!("和网络的连接断了：{err}")),
            };
            warn!(message = %failure.message);
            let mut state = shared.state();
            state.phase = Phase::Failed(failure);
            state.snapshot = Snapshot::default();
            return;
        }
        shared.state().snapshot = snapshot(&node);
    }
}

/// 在服务器上建一个网络，存下它的网络码（服务器地址 + 网络 ID + 邀请码）
async fn create_remote(
    shared: &Shared,
    server: &NetworkCode,
    share: &NetworkCode,
    title: &str,
    me: &str,
) -> Result<NetworkCode, Failure> {
    let coord = server.resolve().await.map_err(|err| {
        Failure::new(
            FailureKind::Resolve,
            format!("找不到服务器 {}：{err}", server.host),
        )
    })?;
    let config = meshora_control::Config {
        secret: shared.secret.clone(),
        coord,
        coord_key: server.coord_key,
        entry: Entry::Create {
            name: title.to_owned(),
        },
        local_port: PORT,
        keepalive: None,
        relay_only: false,
        name: me.to_owned(),
        hosting: Default::default(),
    };
    let (id, invite) = meshora_control::create_network(&config, title)
        .await
        .map_err(|err| match err {
            ControlError::Rejected(reason) => Failure::new(FailureKind::Rejected, reason),
            err => Failure::new(FailureKind::Unreachable, format!("建网络失败：{err}")),
        })?;
    let mut code = share.server();
    code.network = Some(id);
    code.invite = Some(invite);
    info!(network = %id, "网络建好了");
    let mut state = shared.state();
    state.settings.network = Some(code.to_string());
    shared.save(&state.settings);
    Ok(code)
}

async fn start(
    secret: &NodeSecret,
    network: &NetworkCode,
    entry: Entry,
    metric: Option<u32>,
    name: String,
    hosting: Hosting,
    open_tun: Option<TunOpener>,
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
        entry: entry.clone(),
        port,
        tun: TUN_NAME.into(),
        mtu: MTU,
        metric,
        keepalive: NonZeroU16::new(KEEPALIVE),
        relay_only: false,
        name: name.clone(),
        hosting: hosting.clone(),
        open_tun: open_tun.clone(),
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

/// 在后台把网卡设成专用网络（或者设回公用网络），网卡还没归到网络时隔两秒再试
fn set_profile(tun: String, private: bool) -> Background {
    Background(tokio::spawn(async move {
        #[cfg(windows)]
        {
            let started = Instant::now();
            loop {
                let name = tun.clone();
                let result = tokio::task::spawn_blocking(move || {
                    meshora_tun::set_network_private(&name, private)
                })
                .await;
                match result {
                    Ok(Ok(())) => {
                        let category = if private {
                            "专用网络"
                        } else {
                            "公用网络"
                        };
                        info!(%tun, "网卡已设为{category}");
                        return;
                    }
                    Ok(Err(err)) if started.elapsed() < PROFILE_PATIENCE => {
                        tracing::debug!(%err, "网卡还没归到网络，稍后再试");
                    }
                    Ok(Err(err)) => {
                        warn!(%err, "没能改网卡的网络类别");
                        return;
                    }
                    Err(err) => {
                        warn!(%err, "改网络类别的任务出错");
                        return;
                    }
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
        #[cfg(not(windows))]
        info!(%tun, private, "只有 Windows 有网络类别，不用设");
    }))
}

/// 本机的主机：已经跑着就用它，没有就起一个
async fn ensure_host(shared: &Shared) -> Result<Arc<Host>, String> {
    if let Some(host) = shared.state().host.clone() {
        return Ok(host);
    }
    let dir = shared.store.dir().join("host");
    let host = Arc::new(host::start(&dir, shared.secret.public_key()).await?);
    shared.state().host = Some(Arc::clone(&host));
    Ok(host)
}

/// 能用的官方服务器地址：设置里填的优先
fn official_server(settings: &Settings) -> Option<String> {
    settings
        .official_server
        .clone()
        .or_else(|| OFFICIAL_SERVER.map(str::to_owned))
}

/// 网主看到的网络：控制面收到的成员清单，邀请码拼成完整的网络码
fn roster_view(state: &State) -> Option<RosterView> {
    let roster = state.admin.as_ref()?.roster().borrow().clone()?;
    let code: NetworkCode = state.settings.network.as_deref()?.parse().ok()?;
    let with = |invite| {
        let mut full = code.clone();
        full.invite = Some(invite);
        full.to_string()
    };
    Some(RosterView {
        name: roster.name,
        code: roster.invite.map(with),
        members: roster
            .members
            .into_iter()
            .map(|m| RosterMemberRow {
                id: m.key.to_string(),
                name: m.name,
                ip: m.overlay_ip.to_string(),
                online: m.online,
                owner: m.owner,
            })
            .collect(),
        invites: roster
            .invites
            .into_iter()
            .map(|i| InviteRow {
                code: with(i.code),
                invite: i.code.to_string(),
                uses_left: i.uses_left,
                expires: i.expires,
            })
            .collect(),
    })
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

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
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
                name: view.name.clone(),
                ip: view.overlay_ip.to_string(),
                route: match view.path {
                    Some(Path::Direct(_)) => "direct",
                    Some(Path::Relay { .. }) => "relay",
                    None => "pending",
                },
                rtt_ms: view.rtt.map(millis),
                jitter_ms: view.jitter.map(millis),
                loss_percent: view.loss_percent,
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
    fn the_official_server_is_a_server_address() {
        // 写错了的话，"建网络 · 官方服务器"一点就报错
        let code: NetworkCode = OFFICIAL_SERVER.unwrap().parse().unwrap();
        assert!(code.network.is_none() && code.invite.is_none());
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
                    name: "小明".into(),
                    path: Some(Path::Direct(SocketAddr::from(([192, 0, 2, 1], 41641)))),
                    rtt: Some(Duration::from_micros(12_700)),
                    jitter: Some(Duration::from_micros(2_300)),
                    loss_percent: Some(3),
                },
                PeerView {
                    key: key(2),
                    overlay_ip: Ipv4Addr::new(100, 64, 0, 2),
                    name: String::new(),
                    path: Some(relay),
                    rtt: None,
                    jitter: None,
                    loss_percent: None,
                },
                PeerView {
                    key: key(3),
                    overlay_ip: Ipv4Addr::new(100, 64, 0, 3),
                    name: String::new(),
                    path: None,
                    rtt: None,
                    jitter: None,
                    loss_percent: None,
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
                name: "小明".into(),
                ip: "100.64.0.1".into(),
                route: "direct",
                rtt_ms: Some(12),
                jitter_ms: Some(2),
                loss_percent: Some(3),
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
                hub: None,
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
            hub: None,
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
    async fn a_network_created_on_a_server_is_kept_even_if_the_adapter_fails() {
        let dir = TempDir::new("create");
        let controller = controller(&dir);
        let coord_secret = NodeSecret::generate();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(meshora_coord::serve(
            meshora_coord::Config {
                secret: coord_secret.clone(),
                nodes: vec![],
                state: None,
                overlay: "100.64.0.0/10".parse().unwrap(),
                probe: None,
                relays: vec![],
                hub: Some(meshora_coord::HubConfig {
                    dir: dir.0.join("hub"),
                    limits: meshora_coord::HubLimits::default(),
                    creators: None,
                }),
            },
            listener,
            None,
        ));
        let server = NetworkCode::new(coord_secret.public_key(), addr).to_string();

        // 不是服务器地址（带着邀请码）的不收
        assert!(
            controller
                .create(
                    CreateAt::Server {
                        code: format!("{server}#{}", meshora_types::Invite::generate())
                    },
                    "x".into()
                )
                .await
                .is_err()
        );

        controller
            .create(
                CreateAt::Server {
                    code: server.clone(),
                },
                "周末开黑".into(),
            )
            .await
            .unwrap();
        let overview = phase_settles(&controller).await;
        // 网卡建不建得起看权限；网络码不管怎样都已经存下：服务器地址 + 网络 ID + 邀请码
        match &overview.error {
            None => assert_eq!(overview.phase, "connected"),
            Some(failure) => assert_eq!(failure.kind, FailureKind::Tun, "{}", failure.message),
        }
        let saved: NetworkCode = overview.network.unwrap().parse().unwrap();
        assert_eq!(saved.server().to_string(), server);
        assert!(saved.network.is_some() && saved.invite.is_some());
        assert_eq!(
            Store::new(&dir.0).load_settings().network,
            Some(saved.to_string()),
            "存进了设置"
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
        controller.set_private_network(true);
        assert_eq!(controller.set_name("  小明的电脑  "), "小明的电脑");
        assert_eq!(
            controller.set_name(&"长".repeat(40)).chars().count(),
            MAX_NAME_CHARS
        );
        controller.set_name("小明的电脑");
        let saved = Store::new(&dir.0).load_settings();
        assert_eq!(saved.name, "小明的电脑");
        assert_eq!(controller.overview().name, "小明的电脑");
        assert!(!saved.prefer_broadcast);
        assert!(!saved.auto_connect);
        assert!(saved.private_network);
        assert!(controller.overview().private_network);
        let overview = controller.overview();
        assert!(!overview.prefer_broadcast);
        assert_eq!(overview.id.len(), 44);
    }
}
