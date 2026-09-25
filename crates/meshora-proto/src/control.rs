//! 控制通道上的消息：节点 ↔ 协调服务。
//!
//! 走在 Noise IK 加密的 TCP 连接上（见 [`crate::noise`]），每条消息是一个 Noise 帧，
//! 第一个字节是消息类型。
//!
//! 一次连接的流程：
//!
//! 1. 节点连上，完成 Noise IK 握手 —— 身份在这一步就证明了
//! 2. 节点发 [`ClientMessage::Hello`]。**协调服务在收到它之前不做任何有副作用的事**：
//!    IK 的第一个握手包可以被重放，收到加密的 Hello 才说明对面真在线（威胁模型 R1）
//! 3. 协调服务回 [`ServerMessage::Welcome`]（或 [`ServerMessage::Rejected`] 后断开），
//!    接着发 [`ServerMessage::NetMap`]；此后网里有变化就再发一份完整的 NetMap
//! 4. 节点的候选端点变了就发 [`ClientMessage::Endpoints`]

use std::net::{Ipv4Addr, SocketAddr};

use meshora_types::NodeKey;

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
}

/// 一个中继。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelayInfo {
    /// 中继的身份，连接时用它认证对方。
    pub key: NodeKey,
    /// 中继的地址。
    pub addr: SocketAddr,
}

/// 节点发给协调服务的消息。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientMessage {
    /// 握手后的第一条消息。身份已经由握手证明，这里什么也不用带。
    Hello,
    /// 本机的候选端点（完整列表）。
    Endpoints(Vec<SocketAddr>),
    /// 请协调服务转告 `peer`：现在向我的端点发包。两边同时发，才能在各自的 NAT 上凿出洞。
    CallMeMaybe {
        /// 要通知的节点。
        peer: NodeKey,
    },
    /// 保活。
    Ping,
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
}

mod tag {
    pub const HELLO: u8 = 1;
    pub const ENDPOINTS: u8 = 2;
    pub const CALL_ME_MAYBE: u8 = 3;
    pub const PING: u8 = 4;

    pub const WELCOME: u8 = 1;
    pub const NET_MAP: u8 = 2;
    pub const SERVER_CALL_ME_MAYBE: u8 = 3;
    pub const PONG: u8 = 4;
    pub const REJECTED: u8 = 5;
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
            Self::Hello => w.u8(tag::HELLO),
            Self::Endpoints(endpoints) => {
                w.u8(tag::ENDPOINTS);
                write_endpoints(&mut w, endpoints);
            }
            Self::CallMeMaybe { peer } => {
                w.u8(tag::CALL_ME_MAYBE);
                w.key(peer);
            }
            Self::Ping => w.u8(tag::PING),
        }
        w.finish()
    }

    /// 解码。
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(bytes);
        let message = match r.u8()? {
            tag::HELLO => Self::Hello,
            tag::ENDPOINTS => Self::Endpoints(read_endpoints(&mut r)?),
            tag::CALL_ME_MAYBE => Self::CallMeMaybe { peer: r.key()? },
            tag::PING => Self::Ping,
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
            ClientMessage::Hello,
            ClientMessage::Endpoints(vec![
                addr("192.168.1.10:41641"),
                addr("[2001:db8::7]:41641"),
            ]),
            ClientMessage::Endpoints(vec![]),
            ClientMessage::CallMeMaybe { peer: key(3) },
            ClientMessage::Ping,
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
        ];
        for message in messages {
            assert_eq!(ServerMessage::decode(&message.encode()), Ok(message));
        }
    }

    #[test]
    fn unknown_types_and_garbage_are_rejected() {
        assert_eq!(
            ClientMessage::decode(&[99]),
            Err(DecodeError::UnknownType(99))
        );
        assert_eq!(ClientMessage::decode(&[]), Err(DecodeError::Truncated));
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
