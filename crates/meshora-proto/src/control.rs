//! 控制通道上的消息：节点 ↔ 协调服务。
//!
//! 走在 Noise IK 加密的 TCP 连接上（见 [`crate::noise`]），每条消息是一个 Noise 帧，
//! 第一个字节是消息类型。
//!
//! 一次连接的流程：
//!
//! 1. 节点连上，完成 Noise IK 握手 —— 身份在这一步就证明了
//! 2. 节点发 [`ClientMessage::Hello`]。**协调服务在收到它之前不做任何有副作用的事**：
//!    IK 的第一个握手包可以被重放，收到加密的 Hello 才说明对面真在线（威胁模型 R1）。
//!    Hello 里可以带邀请码：不在名单里的节点凭它加入网络
//! 3. 协调服务回 [`ServerMessage::Welcome`]（或 [`ServerMessage::Rejected`] 后断开），
//!    接着发 [`ServerMessage::NetMap`]；此后网里有变化就再发一份完整的 NetMap
//! 4. 节点的候选端点变了就发 [`ClientMessage::Endpoints`]
//! 5. 节点连上后发一次 [`ClientMessage::SetName`]（给别人看的名字），改名时再发

use std::net::{Ipv4Addr, SocketAddr};

use meshora_types::{Invite, NetworkId, NodeKey};

use crate::codec::{DecodeError, Reader, Writer};

/// 网里的另一个节点。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerInfo {
    /// 它的身份。
    pub key: NodeKey,
    /// 它在 overlay 里的地址。
    pub overlay_ip: Ipv4Addr,
    /// 它上报的候选端点：局域网地址、探测到的公网地址。
    pub endpoints: Vec<SocketAddr>,
    /// 它给自己起的名字，空串是没起。谁都可以随便填，认人还得看地址和公钥。
    pub name: String,
}

/// 名字最多多少个字符。
pub const MAX_NAME_CHARS: usize = 32;

/// 把一个名字整理成能放进 NetMap 的样子：去掉控制字符（换行、制表符之类）和看不见的格式字符
/// （方向控制、零宽字符）、去掉首尾空白、最多 [`MAX_NAME_CHARS`] 个字符。
/// 协调服务收到时整理一遍，发的一方也可以先整理。
pub fn clean_name(name: &str) -> String {
    let visible: String = name
        .chars()
        .filter(|c| !c.is_control() && !is_format_char(*c))
        .collect();
    let cut: String = visible.trim().chars().take(MAX_NAME_CHARS).collect();
    cut.trim_end().to_owned()
}

/// 看不见、却能改变文字走向或者把字藏起来的字符：零宽字符、方向控制、字节序标记
fn is_format_char(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}'
    )
}

/// 一个中继。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelayInfo {
    /// 中继的身份，连接时用它认证对方。
    pub key: NodeKey,
    /// 中继的地址。
    pub addr: SocketAddr,
}

/// 网主管理自己的网络（hub 模式）。只有网主发得动，别人发了会收到错误。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdminRequest {
    /// 把一个成员移出网络。移不了网主自己。
    Kick {
        /// 要移出的成员。
        member: NodeKey,
    },
    /// 新建一个带限制的邀请码：只能用几次，或者几小时后过期（至少给一个）。
    NewInvite {
        /// 能用几次。
        uses: Option<u32>,
        /// 多少小时后过期。
        hours: Option<u32>,
    },
    /// 作废一个带限制的邀请码。
    RevokeInvite {
        /// 要作废的邀请码。
        code: Invite,
    },
    /// 换掉长期有效的邀请码：旧的网络码随之失效，已经加入的人不受影响。
    RotateInvite,
    /// 改网络的名字。
    Rename {
        /// 新名字，协调服务会整理（见 [`clean_name`]）。
        name: String,
    },
    /// 解散网络：所有成员被断开，网络删掉。
    Delete,
}

/// 网主看到的一个成员。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RosterMember {
    /// 公钥。
    pub key: NodeKey,
    /// overlay 地址。
    pub overlay_ip: Ipv4Addr,
    /// 它给自己起的名字。
    pub name: String,
    /// 此刻连着协调服务。
    pub online: bool,
    /// 是网主。
    pub owner: bool,
}

/// 网主看到的一个带限制的邀请码。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RosterInvite {
    /// 邀请码。
    pub code: Invite,
    /// 还能用几次，`None` 是不限次数。
    pub uses_left: Option<u32>,
    /// 过期时间（UNIX 秒），`None` 是不过期。
    pub expires: Option<u64>,
}

/// 网主看到的整个网络：成员、邀请码。有变化就推一份新的。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Roster {
    /// 网络的名字。
    pub name: String,
    /// 长期有效的邀请码。
    pub invite: Option<Invite>,
    /// 全部成员。
    pub members: Vec<RosterMember>,
    /// 带限制的邀请码。
    pub invites: Vec<RosterInvite>,
}

/// 节点发给协调服务的消息。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientMessage {
    /// 握手后的第一条消息。身份已经由握手证明。
    ///
    /// 不在名单里的节点带上邀请码，协调服务核对无误就把它加进网络。已经在名单里的节点
    /// 带不带都一样。不带时只有一个字节，和没有邀请码之前的节点发的一模一样。
    Hello {
        /// 邀请码。
        invite: Option<Invite>,
    },
    /// 本机的候选端点（完整列表）。
    Endpoints(Vec<SocketAddr>),
    /// 请协调服务转告 `peer`：现在向我的端点发包。两边同时发，才能在各自的 NAT 上凿出洞。
    CallMeMaybe {
        /// 要通知的节点。
        peer: NodeKey,
    },
    /// 保活。
    Ping,
    /// 本机给别人看的名字（整个替换，空串是不起名字）。协调服务会整理它，见 [`clean_name`]。
    SetName(String),
    /// 握手后的第一条消息（代替 [`Hello`](Self::Hello)）：进 hub 里的某个网络。
    Join {
        /// 哪个网络。
        network: NetworkId,
        /// 邀请码。已经在网里的带不带都一样。
        invite: Option<Invite>,
    },
    /// 握手后的第一条消息（代替 [`Hello`](Self::Hello)）：在 hub 里新建一个网络，自己当网主。
    /// 协调服务回 [`ServerMessage::Created`]，接着和加入一样回 Welcome、NetMap。
    Create {
        /// 网络的名字。
        name: String,
    },
    /// 网主管理网络。回 [`ServerMessage::AdminReply`]。
    Admin(AdminRequest),
}

/// 协调服务发给节点的消息。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerMessage {
    /// 注册成功。
    Welcome {
        /// 分给这个节点的 overlay 地址。
        overlay_ip: Ipv4Addr,
        /// overlay 网段的前缀长度。
        prefix_len: u8,
        /// 端点探测用的 UDP 地址。向它发控制报文，回应里带着"看到你从哪来"。
        probe: Option<SocketAddr>,
        /// 可用的中继。
        relays: Vec<RelayInfo>,
    },
    /// 网里的其他节点，完整列表（声明式：没列出的就是不在了）。
    NetMap {
        /// 其他节点。
        peers: Vec<PeerInfo>,
    },
    /// `peer` 请你现在向它的这些端点发包。
    CallMeMaybe {
        /// 发起打洞的节点。
        peer: NodeKey,
        /// 它当前的候选端点。
        endpoints: Vec<SocketAddr>,
    },
    /// 保活的回应。
    Pong,
    /// 拒绝注册，随后断开。
    Rejected {
        /// 原因，给人看的。
        reason: String,
    },
    /// 网络建好了（回应 [`ClientMessage::Create`]）。紧接着是 Welcome。
    Created {
        /// 新网络的 ID。
        network: NetworkId,
        /// 新网络长期有效的邀请码。
        invite: Invite,
    },
    /// 给网主的成员和邀请码清单。
    Roster(Roster),
    /// 管理请求的结果：成功时可能带一个新邀请码（新建、换码），失败时是原因。
    AdminReply(Result<Option<Invite>, String>),
}

mod tag {
    pub const HELLO: u8 = 1;
    pub const ENDPOINTS: u8 = 2;
    pub const CALL_ME_MAYBE: u8 = 3;
    pub const PING: u8 = 4;
    pub const SET_NAME: u8 = 5;
    pub const JOIN: u8 = 6;
    pub const CREATE: u8 = 7;
    pub const ADMIN: u8 = 8;

    pub const WELCOME: u8 = 1;
    pub const NET_MAP: u8 = 2;
    pub const SERVER_CALL_ME_MAYBE: u8 = 3;
    pub const PONG: u8 = 4;
    pub const REJECTED: u8 = 5;
    pub const CREATED: u8 = 6;
    pub const ROSTER: u8 = 7;
    pub const ADMIN_REPLY: u8 = 8;

    pub const KICK: u8 = 1;
    pub const NEW_INVITE: u8 = 2;
    pub const REVOKE_INVITE: u8 = 3;
    pub const ROTATE_INVITE: u8 = 4;
    pub const RENAME: u8 = 5;
    pub const DELETE: u8 = 6;
}

fn write_invite(w: &mut Writer, invite: &Invite) {
    w.bytes(invite.as_bytes());
}

fn read_invite(r: &mut Reader) -> Result<Invite, DecodeError> {
    Ok(Invite::from_bytes(r.array()?))
}

fn read_network(r: &mut Reader) -> Result<NetworkId, DecodeError> {
    Ok(NetworkId::from_bytes(r.array()?))
}

impl AdminRequest {
    fn write(&self, w: &mut Writer) {
        match self {
            Self::Kick { member } => {
                w.u8(tag::KICK);
                w.key(member);
            }
            Self::NewInvite { uses, hours } => {
                w.u8(tag::NEW_INVITE);
                w.option(uses.as_ref(), |w, n| w.u32(*n));
                w.option(hours.as_ref(), |w, n| w.u32(*n));
            }
            Self::RevokeInvite { code } => {
                w.u8(tag::REVOKE_INVITE);
                write_invite(w, code);
            }
            Self::RotateInvite => w.u8(tag::ROTATE_INVITE),
            Self::Rename { name } => {
                w.u8(tag::RENAME);
                w.string(name);
            }
            Self::Delete => w.u8(tag::DELETE),
        }
    }

    fn read(r: &mut Reader) -> Result<Self, DecodeError> {
        Ok(match r.u8()? {
            tag::KICK => Self::Kick { member: r.key()? },
            tag::NEW_INVITE => Self::NewInvite {
                uses: r.option(|r| r.u32())?,
                hours: r.option(|r| r.u32())?,
            },
            tag::REVOKE_INVITE => Self::RevokeInvite {
                code: read_invite(r)?,
            },
            tag::ROTATE_INVITE => Self::RotateInvite,
            tag::RENAME => Self::Rename { name: r.string()? },
            tag::DELETE => Self::Delete,
            other => return Err(DecodeError::UnknownType(other)),
        })
    }
}

impl Roster {
    fn write(&self, w: &mut Writer) {
        w.string(&self.name);
        w.option(self.invite.as_ref(), write_invite);
        w.list(&self.members, |w, m| {
            w.key(&m.key);
            w.ipv4(m.overlay_ip);
            w.string(&m.name);
            w.u8(u8::from(m.online) | (u8::from(m.owner) << 1));
        });
        w.list(&self.invites, |w, i| {
            write_invite(w, &i.code);
            w.option(i.uses_left.as_ref(), |w, n| w.u32(*n));
            w.option(i.expires.as_ref(), |w, n| w.u64(*n));
        });
    }

    fn read(r: &mut Reader) -> Result<Self, DecodeError> {
        Ok(Self {
            name: r.string()?,
            invite: r.option(read_invite)?,
            members: r.list(|r| {
                let key = r.key()?;
                let overlay_ip = r.ipv4()?;
                let name = r.string()?;
                let flags = r.u8()?;
                if flags > 3 {
                    return Err(DecodeError::Invalid("成员标记只有两位"));
                }
                Ok(RosterMember {
                    key,
                    overlay_ip,
                    name,
                    online: flags & 1 != 0,
                    owner: flags & 2 != 0,
                })
            })?,
            invites: r.list(|r| {
                Ok(RosterInvite {
                    code: read_invite(r)?,
                    uses_left: r.option(|r| r.u32())?,
                    expires: r.option(|r| r.u64())?,
                })
            })?,
        })
    }
}

fn write_endpoints(w: &mut Writer, endpoints: &[SocketAddr]) {
    w.list(endpoints, |w, addr| w.socket_addr(addr));
}

fn read_endpoints(r: &mut Reader) -> Result<Vec<SocketAddr>, DecodeError> {
    r.list(|r| r.socket_addr())
}

impl ClientMessage {
    /// 编码。
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Self::Hello { invite } => {
                w.u8(tag::HELLO);
                if let Some(invite) = invite {
                    w.bytes(invite.as_bytes());
                }
            }
            Self::Endpoints(endpoints) => {
                w.u8(tag::ENDPOINTS);
                write_endpoints(&mut w, endpoints);
            }
            Self::CallMeMaybe { peer } => {
                w.u8(tag::CALL_ME_MAYBE);
                w.key(peer);
            }
            Self::Ping => w.u8(tag::PING),
            Self::SetName(name) => {
                w.u8(tag::SET_NAME);
                w.string(name);
            }
            Self::Join { network, invite } => {
                w.u8(tag::JOIN);
                w.bytes(network.as_bytes());
                w.option(invite.as_ref(), write_invite);
            }
            Self::Create { name } => {
                w.u8(tag::CREATE);
                w.string(name);
            }
            Self::Admin(request) => {
                w.u8(tag::ADMIN);
                request.write(&mut w);
            }
        }
        w.finish()
    }

    /// 解码。
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(bytes);
        let message = match r.u8()? {
            tag::HELLO => Self::Hello {
                invite: match r.rest() {
                    [] => None,
                    rest => Some(Invite::from_bytes(
                        rest.try_into()
                            .map_err(|_| DecodeError::Invalid("邀请码应为 16 个字节"))?,
                    )),
                },
            },
            tag::ENDPOINTS => Self::Endpoints(read_endpoints(&mut r)?),
            tag::CALL_ME_MAYBE => Self::CallMeMaybe { peer: r.key()? },
            tag::PING => Self::Ping,
            tag::SET_NAME => Self::SetName(r.string()?),
            tag::JOIN => Self::Join {
                network: read_network(&mut r)?,
                invite: r.option(read_invite)?,
            },
            tag::CREATE => Self::Create { name: r.string()? },
            tag::ADMIN => Self::Admin(AdminRequest::read(&mut r)?),
            other => return Err(DecodeError::UnknownType(other)),
        };
        r.finish()?;
        Ok(message)
    }
}

impl ServerMessage {
    /// 编码。
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Self::Welcome {
                overlay_ip,
                prefix_len,
                probe,
                relays,
            } => {
                w.u8(tag::WELCOME);
                w.ipv4(*overlay_ip);
                w.u8(*prefix_len);
                match probe {
                    Some(addr) => {
                        w.u8(1);
                        w.socket_addr(addr);
                    }
                    None => w.u8(0),
                }
                w.list(relays, |w, relay| {
                    w.key(&relay.key);
                    w.socket_addr(&relay.addr);
                });
            }
            Self::NetMap { peers } => {
                w.u8(tag::NET_MAP);
                w.list(peers, |w, peer| {
                    w.key(&peer.key);
                    w.ipv4(peer.overlay_ip);
                    write_endpoints(w, &peer.endpoints);
                    w.string(&peer.name);
                });
            }
            Self::CallMeMaybe { peer, endpoints } => {
                w.u8(tag::SERVER_CALL_ME_MAYBE);
                w.key(peer);
                write_endpoints(&mut w, endpoints);
            }
            Self::Pong => w.u8(tag::PONG),
            Self::Rejected { reason } => {
                w.u8(tag::REJECTED);
                w.string(reason);
            }
            Self::Created { network, invite } => {
                w.u8(tag::CREATED);
                w.bytes(network.as_bytes());
                write_invite(&mut w, invite);
            }
            Self::Roster(roster) => {
                w.u8(tag::ROSTER);
                roster.write(&mut w);
            }
            Self::AdminReply(result) => {
                w.u8(tag::ADMIN_REPLY);
                match result {
                    Ok(invite) => {
                        w.u8(0);
                        w.option(invite.as_ref(), write_invite);
                    }
                    Err(reason) => {
                        w.u8(1);
                        w.string(reason);
                    }
                }
            }
        }
        w.finish()
    }

    /// 解码。
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(bytes);
        let message = match r.u8()? {
            tag::WELCOME => {
                let overlay_ip = r.ipv4()?;
                let prefix_len = r.u8()?;
                if prefix_len > 32 {
                    return Err(DecodeError::Invalid("前缀长度超过 32"));
                }
                let probe = match r.u8()? {
                    0 => None,
                    1 => Some(r.socket_addr()?),
                    _ => return Err(DecodeError::Invalid("探测地址的标记只能是 0 或 1")),
                };
                let relays = r.list(|r| {
                    Ok(RelayInfo {
                        key: r.key()?,
                        addr: r.socket_addr()?,
                    })
                })?;
                Self::Welcome {
                    overlay_ip,
                    prefix_len,
                    probe,
                    relays,
                }
            }
            tag::NET_MAP => Self::NetMap {
                peers: r.list(|r| {
                    Ok(PeerInfo {
                        key: r.key()?,
                        overlay_ip: r.ipv4()?,
                        endpoints: read_endpoints(r)?,
                        name: r.string()?,
                    })
                })?,
            },
            tag::SERVER_CALL_ME_MAYBE => Self::CallMeMaybe {
                peer: r.key()?,
                endpoints: read_endpoints(&mut r)?,
            },
            tag::PONG => Self::Pong,
            tag::REJECTED => Self::Rejected {
                reason: r.string()?,
            },
            tag::CREATED => Self::Created {
                network: read_network(&mut r)?,
                invite: read_invite(&mut r)?,
            },
            tag::ROSTER => Self::Roster(Roster::read(&mut r)?),
            tag::ADMIN_REPLY => Self::AdminReply(match r.u8()? {
                0 => Ok(r.option(read_invite)?),
                1 => Err(r.string()?),
                _ => return Err(DecodeError::Invalid("管理结果的标记只能是 0 或 1")),
            }),
            other => return Err(DecodeError::UnknownType(other)),
        };
        r.finish()?;
        Ok(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u8) -> NodeKey {
        NodeKey::from_bytes([n; 32])
    }

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn client_messages_round_trip() {
        let messages = [
            ClientMessage::Hello { invite: None },
            ClientMessage::Hello {
                invite: Some(Invite::from_bytes([5; 16])),
            },
            ClientMessage::Endpoints(vec![
                addr("192.168.1.10:41641"),
                addr("[2001:db8::7]:41641"),
            ]),
            ClientMessage::Endpoints(vec![]),
            ClientMessage::CallMeMaybe { peer: key(3) },
            ClientMessage::Ping,
            ClientMessage::SetName("阿杰的台式机".into()),
            ClientMessage::SetName(String::new()),
            ClientMessage::Join {
                network: NetworkId::from_bytes([4; 16]),
                invite: Some(Invite::from_bytes([5; 16])),
            },
            ClientMessage::Join {
                network: NetworkId::from_bytes([4; 16]),
                invite: None,
            },
            ClientMessage::Create {
                name: "周末开黑".into(),
            },
            ClientMessage::Admin(AdminRequest::Kick { member: key(2) }),
            ClientMessage::Admin(AdminRequest::NewInvite {
                uses: Some(1),
                hours: None,
            }),
            ClientMessage::Admin(AdminRequest::NewInvite {
                uses: None,
                hours: Some(24),
            }),
            ClientMessage::Admin(AdminRequest::RevokeInvite {
                code: Invite::from_bytes([6; 16]),
            }),
            ClientMessage::Admin(AdminRequest::RotateInvite),
            ClientMessage::Admin(AdminRequest::Rename {
                name: "新名字".into(),
            }),
            ClientMessage::Admin(AdminRequest::Delete),
        ];
        for message in messages {
            assert_eq!(ClientMessage::decode(&message.encode()), Ok(message));
        }
    }

    #[test]
    fn server_messages_round_trip() {
        let messages = [
            ServerMessage::Welcome {
                overlay_ip: Ipv4Addr::new(100, 64, 0, 2),
                prefix_len: 10,
                probe: Some(addr("203.0.113.5:7443")),
                relays: vec![RelayInfo {
                    key: key(9),
                    addr: addr("203.0.113.5:7444"),
                }],
            },
            ServerMessage::Welcome {
                overlay_ip: Ipv4Addr::new(100, 64, 0, 3),
                prefix_len: 10,
                probe: None,
                relays: vec![],
            },
            ServerMessage::NetMap {
                peers: vec![PeerInfo {
                    key: key(1),
                    overlay_ip: Ipv4Addr::new(100, 64, 0, 1),
                    endpoints: vec![addr("198.51.100.4:41641")],
                    name: "小明".into(),
                }],
            },
            ServerMessage::CallMeMaybe {
                peer: key(1),
                endpoints: vec![addr("198.51.100.4:41641")],
            },
            ServerMessage::Pong,
            ServerMessage::Rejected {
                reason: "这把公钥不在白名单里".into(),
            },
            ServerMessage::Created {
                network: NetworkId::from_bytes([4; 16]),
                invite: Invite::from_bytes([5; 16]),
            },
            ServerMessage::Roster(Roster {
                name: "周末开黑".into(),
                invite: Some(Invite::from_bytes([5; 16])),
                members: vec![
                    RosterMember {
                        key: key(1),
                        overlay_ip: Ipv4Addr::new(100, 64, 0, 1),
                        name: "阿杰".into(),
                        online: true,
                        owner: true,
                    },
                    RosterMember {
                        key: key(2),
                        overlay_ip: Ipv4Addr::new(100, 64, 0, 2),
                        name: String::new(),
                        online: false,
                        owner: false,
                    },
                ],
                invites: vec![RosterInvite {
                    code: Invite::from_bytes([6; 16]),
                    uses_left: Some(1),
                    expires: Some(1_790_000_000),
                }],
            }),
            ServerMessage::AdminReply(Ok(None)),
            ServerMessage::AdminReply(Ok(Some(Invite::from_bytes([7; 16])))),
            ServerMessage::AdminReply(Err("只有网主能管理网络".into())),
        ];
        for message in messages {
            assert_eq!(ServerMessage::decode(&message.encode()), Ok(message));
        }
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean_name("  小明  "), "小明");
        assert_eq!(clean_name("a\nb\tc"), "abc");
        // 方向控制字符能把显示的字倒过来，零宽字符能藏字
        assert_eq!(clean_name("evil\u{202E}gnp.exe"), "evilgnp.exe");
        assert_eq!(clean_name("x\u{200B}y"), "xy");
        let long = "长".repeat(50);
        assert_eq!(clean_name(&long).chars().count(), MAX_NAME_CHARS);
        // 截断之后末尾的空白也去掉
        assert_eq!(
            clean_name(&format!("{} b", "a".repeat(MAX_NAME_CHARS - 1))),
            "a".repeat(MAX_NAME_CHARS - 1)
        );
        assert_eq!(clean_name("\u{0}\u{1}"), "");
    }

    #[test]
    fn unknown_types_and_garbage_are_rejected() {
        assert_eq!(
            ClientMessage::decode(&[99]),
            Err(DecodeError::UnknownType(99))
        );
        assert_eq!(ClientMessage::decode(&[]), Err(DecodeError::Truncated));
        // 没有邀请码的 Hello 就是一个字节：老节点发的也是这个
        assert_eq!(
            ClientMessage::decode(&[tag::HELLO]),
            Ok(ClientMessage::Hello { invite: None })
        );
        assert_eq!(
            ClientMessage::decode(&[tag::HELLO, 1, 2, 3]),
            Err(DecodeError::Invalid("邀请码应为 16 个字节"))
        );
        assert_eq!(
            ClientMessage::decode(&[tag::PING, 0]),
            Err(DecodeError::TrailingBytes)
        );

        let mut welcome = ServerMessage::Welcome {
            overlay_ip: Ipv4Addr::new(100, 64, 0, 2),
            prefix_len: 10,
            probe: None,
            relays: vec![],
        }
        .encode();
        welcome[5] = 33;
        assert_eq!(
            ServerMessage::decode(&welcome),
            Err(DecodeError::Invalid("前缀长度超过 32"))
        );
    }
}
