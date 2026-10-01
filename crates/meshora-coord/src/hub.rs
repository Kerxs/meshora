//! hub 模式：一个协调服务托管很多网络。谁都能来建一个，自己当网主；朋友凭网络码加入。
//!
//! 谁都能建，所以有限额（[`HubLimits`]）：每个公钥能建几个、每个来源 IP 一小时能建几个、
//! 每个网络多少人、总共多少个网络，长期没人上线的网络自动删掉。
//! 每个网络一个状态文件 `<网络 ID>.state`，放在数据目录里，重启后照旧。

use std::collections::HashMap;
use std::io;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use ipnet::Ipv4Net;
use meshora_proto::control::clean_name;
use meshora_types::{Invite, NetworkId, NodeKey};
use tracing::{info, warn};

use crate::network::{Network, State};
use crate::state_file::{Member, load_state, merge_joined};

/// hub 模式的配置。
#[derive(Clone, Debug)]
pub struct HubConfig {
    /// 数据目录：每个网络一个状态文件。
    pub dir: PathBuf,
    /// 限额。
    pub limits: HubLimits,
    /// 只有这些公钥能建网络；`None` 是谁都能建。客户端在本机当主机时只许自己建。
    pub creators: Option<Vec<NodeKey>>,
}

/// hub 模式的限额。谁都能建网络，资源得有个边。
#[derive(Clone, Debug)]
pub struct HubLimits {
    /// 一把公钥最多当几个网络的网主。
    pub networks_per_owner: usize,
    /// 一个网络最多多少人。
    pub members_per_network: usize,
    /// 一个来源 IP 一小时最多建几个网络（换公钥也绕不过去）。
    pub creations_per_source_per_hour: usize,
    /// 总共最多多少个网络。
    pub max_networks: usize,
    /// 多久没人上线的网络自动删掉。
    pub idle: Duration,
    /// 同一进程里的中继给每个节点的转发速率上限（字节 / 秒）。
    pub relay_rate: u64,
}

impl Default for HubLimits {
    fn default() -> Self {
        Self {
            networks_per_owner: 3,
            members_per_network: 64,
            creations_per_source_per_hour: 10,
            max_networks: 10_000,
            idle: Duration::from_secs(30 * 24 * 3600),
            relay_rate: 4 * 1024 * 1024,
        }
    }
}

/// 网络表
pub(crate) struct Hub {
    dir: PathBuf,
    pub limits: HubLimits,
    creators: Option<Vec<NodeKey>>,
    overlay: Ipv4Net,
    networks: HashMap<NetworkId, Arc<Network>>,
    /// 每个来源最近一小时建网络的时刻
    creations: HashMap<IpAddr, Vec<Instant>>,
}

impl Hub {
    /// 读数据目录里已有的网络。读不了的文件记一条警告、跳过 —— 一个坏文件不该让整个服务起不来
    pub fn load(config: HubConfig, overlay: Ipv4Net) -> io::Result<Self> {
        std::fs::create_dir_all(&config.dir)?;
        let mut networks = HashMap::new();
        for entry in std::fs::read_dir(&config.dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("state") {
                continue;
            }
            let Some(id) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<NetworkId>().ok())
            else {
                warn!(path = %path.display(), "文件名不是网络 ID，跳过");
                continue;
            };
            let stored = match load_state(&path) {
                Ok(stored) => stored,
                Err(err) => {
                    warn!(%err, "读不了网络的状态文件，跳过");
                    continue;
                }
            };
            let (Some(invite), Some(owner)) = (stored.invite, stored.owner) else {
                warn!(path = %path.display(), "状态文件里没有邀请码或网主，跳过");
                continue;
            };
            let members = match merge_joined(Vec::new(), stored.joined, overlay) {
                Ok(members) => members,
                Err(err) => {
                    warn!(%err, "网络的状态文件有错，跳过");
                    continue;
                }
            };
            let state = new_state(members, invite, owner, stored.name.unwrap_or_default());
            let mut state = state;
            state.limited = stored.limited;
            state.seen = stored.seen.unwrap_or_else(SystemTime::now);
            let network = Network::new(
                Some(id),
                overlay,
                config.limits.members_per_network,
                Some(path),
                state,
            );
            networks.insert(id, Arc::new(network));
        }
        info!(dir = %config.dir.display(), networks = networks.len(), "hub 模式：读入已有的网络");
        Ok(Self {
            dir: config.dir,
            limits: config.limits,
            creators: config.creators,
            overlay,
            networks,
            creations: HashMap::new(),
        })
    }

    pub fn get(&self, id: &NetworkId) -> Option<Arc<Network>> {
        self.networks.get(id).cloned()
    }

    pub fn networks(&self) -> impl Iterator<Item = &Arc<Network>> {
        self.networks.values()
    }

    pub fn len(&self) -> usize {
        self.networks.len()
    }

    /// 建一个新网络，`owner` 是网主、也是第一个成员
    pub fn create(
        &mut self,
        owner: NodeKey,
        name: &str,
        source: IpAddr,
    ) -> Result<(Arc<Network>, Invite), String> {
        if let Some(creators) = &self.creators
            && !creators.contains(&owner)
        {
            return Err("这台服务器不让别人建网络".into());
        }
        let now = Instant::now();
        let recent = self.creations.entry(source).or_default();
        recent.retain(|at| now.duration_since(*at) < Duration::from_secs(3600));
        if recent.len() >= self.limits.creations_per_source_per_hour {
            return Err("建网络太频繁了，过一会儿再试".into());
        }
        if self.networks.len() >= self.limits.max_networks {
            return Err("这台服务器上的网络已经满了".into());
        }
        let owned = self
            .networks
            .values()
            .filter(|n| n.state().owner == Some(owner))
            .count();
        if owned >= self.limits.networks_per_owner {
            return Err(format!(
                "你已经建了 {owned} 个网络，最多 {} 个：先解散一个不用的",
                self.limits.networks_per_owner
            ));
        }

        let id = NetworkId::generate();
        let invite = Invite::generate();
        let first = self
            .overlay
            .hosts()
            .next()
            .ok_or("overlay 网段里没有可用的地址")?;
        let mut name = clean_name(name);
        if name.is_empty() {
            name = "我的网络".into();
        }
        let members = vec![Member {
            key: owner,
            ip: first,
            joined: true,
        }];
        let network = Network::new(
            Some(id),
            self.overlay,
            self.limits.members_per_network,
            Some(self.dir.join(format!("{id}.state"))),
            new_state(members, invite, owner, name),
        );
        {
            let state = network.state();
            if let Err(err) = network.save(&state) {
                warn!(%err, "写不了新网络的状态文件");
                return Err("服务器存不下新网络，过一会儿再试".into());
            }
        }
        recent.push(now);
        let network = Arc::new(network);
        self.networks.insert(id, Arc::clone(&network));
        info!(network = %id, owner = %owner, "新建了一个网络");
        Ok((network, invite))
    }

    /// 从表里拿掉（解散、闲置清理之后）
    pub fn remove(&mut self, id: &NetworkId) {
        self.networks.remove(id);
    }

    /// 太久没人上线的网络
    pub fn idle(&self, now: SystemTime) -> Vec<Arc<Network>> {
        self.networks
            .values()
            .filter(|network| {
                let state = network.state();
                state.online.is_empty()
                    && now
                        .duration_since(state.seen)
                        .is_ok_and(|idle| idle >= self.limits.idle)
            })
            .cloned()
            .collect()
    }
}

fn new_state(members: Vec<Member>, invite: Invite, owner: NodeKey, name: String) -> State {
    State {
        members,
        invite: Some(invite),
        limited: Vec::new(),
        online: HashMap::new(),
        endpoints: HashMap::new(),
        names: HashMap::new(),
        owner: Some(owner),
        name,
        seen: SystemTime::now(),
        deleted: false,
    }
}
