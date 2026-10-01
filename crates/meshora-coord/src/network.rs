//! 一个网络：成员、邀请码、在线的连接、端点、名字，以及管这些的规则。
//!
//! 单网络模式下整个协调服务只有一个（`id` 是 `None`）；hub 模式下每个网络一个，有网主。

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ipnet::Ipv4Net;
use meshora_proto::control::{
    AdminRequest, PeerInfo, Roster, RosterInvite, RosterMember, ServerMessage, clean_name,
};
use meshora_types::{Invite, NetworkId, NodeKey};
use tokio::sync::{Notify, mpsc};
use tracing::{info, warn};

use crate::state_file::{
    Limited, Member, Saved, Stamp, file_stamp, free_address, load_state, merge_joined, new_limited,
    save_state,
};

/// 让一条连接断开：带上原因（"你被移出了""网络解散了"），连接告诉对方之后断开
pub(crate) struct Kick {
    notify: Notify,
    reason: Mutex<String>,
}

impl Kick {
    pub fn new() -> Self {
        Self {
            notify: Notify::new(),
            reason: Mutex::new(String::new()),
        }
    }

    pub fn kick(&self, reason: &str) {
        *self.reason.lock().expect("踢人的原因在持锁时 panic 过") = reason.to_owned();
        self.notify.notify_one();
    }

    pub async fn kicked(&self) -> String {
        self.notify.notified().await;
        self.reason
            .lock()
            .expect("踢人的原因在持锁时 panic 过")
            .clone()
    }
}

pub(crate) struct Conn {
    pub id: u64,
    /// Welcome、转发的打洞请求、Pong、管理结果
    pub tx: mpsc::Sender<ServerMessage>,
    /// "有新的 NetMap 了"。连接的写任务被唤醒时按当时的状态现生成一份，所以多次变化
    /// 自然合并，最新的一份也绝不会丢 —— 此前用队列发，队列满了就跳过，
    /// 跳过的偏偏是最后一份的话，这个节点就一直拿着过时的 NetMap
    pub net_map: Arc<Notify>,
    /// "有新的成员清单了"：只有网主的连接会收到，生成方式同 NetMap
    pub roster: Arc<Notify>,
    pub kick: Arc<Kick>,
}

pub(crate) struct State {
    /// 所有成员，同时是地址表
    pub members: Vec<Member>,
    /// 长期有效的邀请码。`None` 表示不接受邀请加入
    pub invite: Option<Invite>,
    /// 带限制的邀请码
    pub limited: Vec<Limited>,
    pub online: HashMap<NodeKey, Conn>,
    pub endpoints: HashMap<NodeKey, Vec<SocketAddr>>,
    /// 成员给自己起的名字（整理过的）。只在内存里：节点每次连上都会重发
    pub names: HashMap<NodeKey, String>,
    /// 网主（hub 模式）
    pub owner: Option<NodeKey>,
    /// 网络的名字（hub 模式）
    pub name: String,
    /// 最近一次有人在线的时刻，闲置清理用
    pub seen: SystemTime,
    /// 网络被解散了：还拿着它的连接不要再往里登记
    pub deleted: bool,
}

impl State {
    pub fn address_of(&self, key: &NodeKey) -> Option<Ipv4Addr> {
        self.members
            .iter()
            .find(|member| member.key == *key)
            .map(|member| member.ip)
    }
}

/// 一个网络
pub(crate) struct Network {
    /// hub 模式下的网络 ID；单网络模式是 `None`
    pub id: Option<NetworkId>,
    pub overlay: Ipv4Net,
    /// 成员上限
    pub max_members: usize,
    /// 状态文件，有邀请码时才有
    pub state_file: Option<PathBuf>,
    /// 上次读或写状态文件之后它的样子。和现在的不一样，就是有人手改过
    state_stamp: Mutex<Option<Stamp>>,
    state: Mutex<State>,
}

impl Network {
    pub fn new(
        id: Option<NetworkId>,
        overlay: Ipv4Net,
        max_members: usize,
        state_file: Option<PathBuf>,
        state: State,
    ) -> Self {
        Self {
            id,
            overlay,
            max_members,
            state_stamp: Mutex::new(state_file.as_deref().and_then(file_stamp)),
            state_file,
            state: Mutex::new(state),
        }
    }

    pub fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("协调服务的状态在持锁时 panic 过")
    }

    pub fn is_member(&self, key: &NodeKey) -> bool {
        self.state().address_of(key).is_some()
    }

    /// 发给 `recipient` 的 NetMap：除它自己以外的所有成员
    pub fn net_map_for(&self, state: &State, recipient: &NodeKey) -> ServerMessage {
        let peers = state
            .members
            .iter()
            .filter(|member| member.key != *recipient)
            .map(|member| PeerInfo {
                key: member.key,
                overlay_ip: member.ip,
                endpoints: state
                    .endpoints
                    .get(&member.key)
                    .cloned()
                    .unwrap_or_default(),
                name: state.names.get(&member.key).cloned().unwrap_or_default(),
            })
            .collect();
        ServerMessage::NetMap { peers }
    }

    /// 给网主看的成员和邀请码清单
    pub fn roster(&self, state: &State) -> ServerMessage {
        let now = SystemTime::now();
        ServerMessage::Roster(Roster {
            name: state.name.clone(),
            invite: state.invite,
            members: state
                .members
                .iter()
                .map(|member| RosterMember {
                    key: member.key,
                    overlay_ip: member.ip,
                    name: state.names.get(&member.key).cloned().unwrap_or_default(),
                    online: state.online.contains_key(&member.key),
                    owner: state.owner == Some(member.key),
                })
                .collect(),
            invites: state
                .limited
                .iter()
                .filter(|l| !l.expired(now))
                .map(|l| RosterInvite {
                    code: l.code,
                    uses_left: l.uses_left,
                    expires: l
                        .expires
                        .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_secs()),
                })
                .collect(),
        })
    }

    /// 刚发来第一条消息的节点能不能进：在网里的直接进；不在的核对邀请码，对了就加进来。
    /// 不能进时返回给它看的原因
    pub fn admit(&self, key: &NodeKey, invite: Option<Invite>) -> Result<Ipv4Addr, String> {
        let mut state = self.state();
        if state.deleted {
            return Err("这个网络已经被网主解散了".into());
        }
        // 状态文件刚被人改过、还没轮到重读：先读进来。不然下面加人时整个重写文件，
        // 会用内存里的旧内容把人手改的覆盖掉（比如刚换掉的邀请码又被写回旧的）
        if self.id.is_none()
            && let Some(path) = &self.state_file
            && file_stamp(path) != *self.stamp()
        {
            self.apply_state_file(&mut state, path);
        }
        if let Some(ip) = state.address_of(key) {
            return Ok(ip);
        }
        let (ours, theirs) = match (state.invite, invite) {
            (Some(ours), Some(theirs)) => (ours, theirs),
            (None, Some(_)) => return Err("这个网络不接受凭邀请码加入".into()),
            (_, None) => return Err("这把公钥不在协调服务的节点名单里".into()),
        };
        // 长期有效的对上了，或者对上了一个还能用的带限制的
        let now = SystemTime::now();
        let limited = if ours.matches(&theirs) {
            None
        } else {
            match state.limited.iter().position(|l| l.code.matches(&theirs)) {
                Some(index) if !state.limited[index].expired(now) => Some(index),
                Some(_) => {
                    return Err("邀请码已经过期或者用完了：向建网络的人要一个新的网络码".into());
                }
                None => {
                    return Err("邀请码不对，可能已经换过了：向建网络的人要一个新的网络码".into());
                }
            }
        };
        if state.members.len() >= self.max_members {
            return Err(format!("网络已满（最多 {} 个成员）", self.max_members));
        }
        let ip = free_address(&state.members, self.overlay)
            .ok_or_else(|| format!("overlay 网段 {} 没有空闲地址了", self.overlay))?;
        let before = state.limited.clone();
        if let Some(index) = limited {
            // 用掉一次；用完的在写文件时自然清掉
            if let Some(uses) = &mut state.limited[index].uses_left {
                *uses -= 1;
            }
        }
        state.members.push(Member {
            key: *key,
            ip,
            joined: true,
        });
        // 先写进文件再算加入：否则重启之后这个地址可能分给别人
        if let Err(err) = self.save(&state) {
            state.members.pop();
            state.limited = before;
            warn!(%err, "写状态文件失败，拒绝新成员");
            return Err("协调服务存不下新成员，请联系建网络的人".into());
        }
        state.limited.retain(|l| !l.expired(now));
        info!(network = ?self.id, node = %key, %ip, limited = limited.is_some(), "新成员凭邀请码加入");
        // 其他人的 NetMap 里要多出这个新成员
        self.changed(&state, Some(key));
        Ok(ip)
    }

    fn stamp(&self) -> MutexGuard<'_, Option<Stamp>> {
        self.state_stamp
            .lock()
            .expect("协调服务的状态在持锁时 panic 过")
    }

    /// 写状态文件（有的话），记下写完之后它的样子 —— 自己写的不算"有人改过"
    pub fn save(&self, state: &State) -> std::io::Result<()> {
        let (Some(path), Some(invite)) = (&self.state_file, &state.invite) else {
            return Ok(());
        };
        save_state(
            path,
            &Saved {
                invite,
                limited: &state.limited,
                members: &state.members,
                owner: state.owner.as_ref(),
                name: &state.name,
                seen: self.id.map(|_| state.seen),
            },
        )?;
        *self.stamp() = file_stamp(path);
        Ok(())
    }

    /// 状态文件被人改了（单网络模式）：重新读，按变化增删成员、换邀请码
    pub fn reload_if_changed(&self) {
        let Some(path) = &self.state_file else {
            return;
        };
        if file_stamp(path) != *self.stamp() {
            let mut state = self.state();
            self.apply_state_file(&mut state, path);
        }
    }

    /// 把状态文件的内容套到现在的成员表上。文件有错就保持原样
    fn apply_state_file(&self, state: &mut State, path: &Path) {
        // 不管读没读成都记下：坏掉的文件不必每轮都再报一次，改好了自然又会变
        *self.stamp() = file_stamp(path);
        let stored = match load_state(path) {
            Ok(stored) => stored,
            Err(err) => {
                warn!(%err, "状态文件读不了或者写错了，保持原样");
                return;
            }
        };
        let fixed = state
            .members
            .iter()
            .filter(|m| !m.joined)
            .copied()
            .collect();
        let members = match merge_joined(fixed, stored.joined, self.overlay) {
            Ok(members) if members.len() <= self.max_members => members,
            Ok(_) => {
                warn!(max = self.max_members, "状态文件里的成员超过上限，保持原样");
                return;
            }
            Err(err) => {
                warn!(%err, "状态文件有错，保持原样");
                return;
            }
        };
        let (invite, regenerated) = match stored.invite {
            Some(invite) => (invite, false),
            None => (Invite::generate(), true),
        };

        let removed: Vec<NodeKey> = state
            .members
            .iter()
            .map(|member| member.key)
            .filter(|key| !members.iter().any(|member| member.key == *key))
            .collect();
        let changed = members != state.members;
        let invite_changed = state.invite != Some(invite);
        state.members = members;
        state.invite = Some(invite);
        state.limited = stored.limited;
        for key in &removed {
            Self::remove(state, key, "你已经被移出这个网络");
            info!(node = %key, "状态文件里没有它了，移出网络");
        }
        if changed {
            self.changed(state, None);
        }
        if invite_changed {
            info!("邀请码换了，旧的网络码作废。新的网络码末尾是 #{invite}");
        }
        if regenerated && let Err(err) = self.save(state) {
            warn!(%err, path = %path.display(), "新邀请码写不进状态文件");
        }
    }

    /// 从内存里拿掉一个成员的连接、端点、名字，在线的话让它断开
    fn remove(state: &mut State, key: &NodeKey, reason: &str) {
        state.endpoints.remove(key);
        state.names.remove(key);
        if let Some(conn) = state.online.remove(key) {
            conn.kick.kick(reason);
        }
    }

    /// 网里有变化：每个在线节点发一份新的 NetMap，网主再多一份成员清单
    pub fn changed(&self, state: &State, except: Option<&NodeKey>) {
        for (key, conn) in &state.online {
            if Some(key) != except {
                conn.net_map.notify_one();
            }
        }
        self.roster_changed(state);
    }

    /// 只有成员清单变了（谁上线了、邀请码变了）：只通知网主
    pub fn roster_changed(&self, state: &State) {
        if let Some(owner) = &state.owner
            && let Some(conn) = state.online.get(owner)
        {
            conn.roster.notify_one();
        }
    }

    /// 网主的管理请求。`Ok` 里可能有一个新邀请码（新建、换码）
    pub fn admin(&self, actor: &NodeKey, request: AdminRequest) -> Result<Option<Invite>, String> {
        let mut state = self.state();
        if state.owner.as_ref() != Some(actor) {
            return Err("只有网主能管理这个网络".into());
        }
        if state.deleted {
            return Err("这个网络已经解散了".into());
        }
        let result = match request {
            AdminRequest::Kick { member } => {
                if member == *actor {
                    return Err("网主不能把自己移出去；不想要这个网络了就解散它".into());
                }
                let before = state.members.len();
                state.members.retain(|m| m.key != member);
                if state.members.len() == before {
                    return Err("网里没有这个人".into());
                }
                Self::remove(&mut state, &member, "你已经被移出这个网络");
                info!(network = ?self.id, node = %member, "网主把它移出了网络");
                self.changed(&state, None);
                None
            }
            AdminRequest::NewInvite { uses, hours } => {
                let valid_for = hours.map(|h| Duration::from_secs(u64::from(h) * 3600));
                let entry = new_limited(uses, valid_for).ok_or("次数和时长至少要限制一个")?;
                if state.limited.len() >= MAX_LIMITED {
                    return Err(format!("限时邀请码最多 {MAX_LIMITED} 个，先作废一些"));
                }
                state.limited.push(entry);
                self.roster_changed(&state);
                Some(entry.code)
            }
            AdminRequest::RevokeInvite { code } => {
                let before = state.limited.len();
                state.limited.retain(|l| !l.code.matches(&code));
                if state.limited.len() == before {
                    return Err("没有这个邀请码".into());
                }
                self.roster_changed(&state);
                None
            }
            AdminRequest::RotateInvite => {
                let invite = Invite::generate();
                state.invite = Some(invite);
                self.roster_changed(&state);
                Some(invite)
            }
            AdminRequest::Rename { name } => {
                let name = clean_name(&name);
                if name.is_empty() {
                    return Err("网络名不能是空的".into());
                }
                state.name = name;
                self.roster_changed(&state);
                None
            }
            AdminRequest::Delete => {
                self.delete_locked(&mut state);
                return Ok(None);
            }
        };
        if let Err(err) = self.save(&state) {
            warn!(%err, "写状态文件失败");
            return Err("协调服务存不下这次改动".into());
        }
        Ok(result)
    }

    /// 解散：所有在线的人断开，删掉状态文件
    pub fn delete(&self) {
        let mut state = self.state();
        self.delete_locked(&mut state);
    }

    fn delete_locked(&self, state: &mut State) {
        state.deleted = true;
        for (_, conn) in state.online.drain() {
            conn.kick.kick("这个网络已经被网主解散了");
        }
        if let Some(path) = &self.state_file
            && let Err(err) = std::fs::remove_file(path)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            warn!(%err, path = %path.display(), "删不掉解散了的网络的状态文件");
        }
        info!(network = ?self.id, "网络解散了");
    }

    /// 记下"刚才还有人在线"（hub 模式下的闲置清理看它）。只在没人在线时写盘
    pub fn touch(&self, state: &mut State) {
        state.seen = SystemTime::now();
        if self.id.is_some()
            && state.online.is_empty()
            && let Err(err) = self.save(state)
        {
            warn!(%err, "写状态文件失败");
        }
    }
}

/// 一个网络最多同时有多少个带限制的邀请码
const MAX_LIMITED: usize = 32;
