//! 中继通道上的帧：节点 ↔ 中继。
//!
//! 走在 Noise IK 加密的 TCP 连接上（[`crate::noise`]，通道类型 [`Relay`](crate::noise::Channel::Relay)）。
//! 节点用自己的节点密钥握手，中继因此确切知道每条连接属于谁（威胁模型 R6）——
//! 谁也没法冒领发给别人的报文。
//!
//! 中继转发的是 WireGuard 报文：它看得到"谁发给谁、多大"，看不到内容。
//!
//! 一条连接的流程：握手 → 节点发 [`ClientFrame::Hello`]（中继在收到它之前不登记这条连接，R1）
//! → 此后节点发 [`ClientFrame::Send`]，中继把它变成 [`ServerFrame::Recv`] 交给目标节点。

use meshora_types::NodeKey;

use crate::codec::{DecodeError, Reader, Writer};

/// 节点发给中继的帧。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientFrame {
    /// 握手后的第一帧。
    Hello,
    /// 请把这个报文交给 `dst`。
    Send {
        /// 目标节点。
        dst: NodeKey,
        /// WireGuard 报文。
        datagram: Vec<u8>,
    },
    /// 保活。
    Ping,
}

/// 中继发给节点的帧。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerFrame {
    /// `src` 发给你的报文。`src` 是中继认证过的连接主人。
    Recv {
        /// 发送方节点。
        src: NodeKey,
        /// WireGuard 报文。
        datagram: Vec<u8>,
    },
    /// 保活的回应。
    Pong,
}

mod tag {
    pub const HELLO: u8 = 1;
    pub const SEND: u8 = 2;
    pub const PING: u8 = 3;

    pub const RECV: u8 = 1;
    pub const PONG: u8 = 2;
}

impl ClientFrame {
    /// 编码。报文不带长度前缀：它就是这一帧剩下的全部字节。
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Self::Hello => w.u8(tag::HELLO),
            Self::Send { dst, datagram } => {
                w.u8(tag::SEND);
                w.key(dst);
                w.bytes(datagram);
            }
            Self::Ping => w.u8(tag::PING),
        }
        w.finish()
    }

    /// 解码。
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(bytes);
        let frame = match r.u8()? {
            tag::HELLO => Self::Hello,
            tag::SEND => Self::Send {
                dst: r.key()?,
                datagram: r.rest().to_vec(),
            },
            tag::PING => Self::Ping,
            other => return Err(DecodeError::UnknownType(other)),
        };
        r.finish()?;
        Ok(frame)
    }
}

impl ServerFrame {
    /// 编码。
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Self::Recv { src, datagram } => {
                w.u8(tag::RECV);
                w.key(src);
                w.bytes(datagram);
            }
            Self::Pong => w.u8(tag::PONG),
        }
        w.finish()
    }

    /// 解码。
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(bytes);
        let frame = match r.u8()? {
            tag::RECV => Self::Recv {
                src: r.key()?,
                datagram: r.rest().to_vec(),
            },
            tag::PONG => Self::Pong,
            other => return Err(DecodeError::UnknownType(other)),
        };
        r.finish()?;
        Ok(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let key = NodeKey::from_bytes([3; 32]);
        for frame in [
            ClientFrame::Hello,
            ClientFrame::Send {
                dst: key,
                datagram: vec![4, 0, 0, 0, 9, 9],
            },
            ClientFrame::Send {
                dst: key,
                datagram: vec![],
            },
            ClientFrame::Ping,
        ] {
            assert_eq!(ClientFrame::decode(&frame.encode()), Ok(frame));
        }
        for frame in [
            ServerFrame::Recv {
                src: key,
                datagram: vec![1; 148],
            },
            ServerFrame::Pong,
        ] {
            assert_eq!(ServerFrame::decode(&frame.encode()), Ok(frame));
        }
    }

    #[test]
    fn truncated_and_unknown_frames_are_rejected() {
        assert_eq!(
            ClientFrame::decode(&[tag::SEND, 1, 2]),
            Err(DecodeError::Truncated)
        );
        assert_eq!(ClientFrame::decode(&[9]), Err(DecodeError::UnknownType(9)));
        assert_eq!(
            ServerFrame::decode(&[tag::PONG, 0]),
            Err(DecodeError::TrailingBytes)
        );
        assert_eq!(ServerFrame::decode(&[]), Err(DecodeError::Truncated));
    }
}
