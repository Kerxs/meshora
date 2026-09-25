//! 节点之间（以及节点与探测端点之间）的控制报文。
//!
//! 走 WireGuard 共用的那个 UDP socket，数据面按开头的魔数把它们分出来交给控制面。
//! 数据面不做任何认证，所以每一条都必须在这里认证（威胁模型 R3）：
//!
//! ```text
//! "MSHR" (4) | 版本 (1) | 发送方公钥 (32) | Noise K 消息：临时公钥 (32) + 密文 + 认证标签 (16)
//! ```
//!
//! 每条报文是一次 Noise K 单向握手：发送方用自己的静态私钥和接收方的静态公钥封装。
//! 发送方公钥明文放在头部，接收方据此**先判断认不认识这个节点，再做 DH** ——
//! 不认识的直接丢，不让陌生人的报文消耗算力。头部整个作为 Noise 的 prologue，改一个字节都解不开。

use std::fmt;
use std::net::SocketAddr;

use meshora_types::{CONTROL_MAGIC, NodeKey, NodeSecret};

use crate::codec::{DecodeError, Reader, Writer};

/// 控制报文格式的版本。
pub const VERSION: u8 = 1;
/// 头部长度：魔数、版本、发送方公钥。
const HEADER_LEN: usize = 4 + 1 + 32;
const PATTERN: &str = "Noise_K_25519_ChaChaPoly_BLAKE2s";

/// 一次事务的编号：Pong 必须带着对应 Ping 的编号，否则不算数。
pub type TxId = [u8; 12];

/// 控制报文的内容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscoMessage {
    /// 探路：对方收到就回 Pong。
    Ping {
        /// 事务编号，随机生成。
        tx: TxId,
    },
    /// 对 Ping 的回应。
    Pong {
        /// 对应 Ping 的事务编号。
        tx: TxId,
        /// 回应方看到的 Ping 来源地址 —— 也就是发送方在对方眼里的公网地址。
        /// 端点探测（类 STUN）靠的就是它。
        observed: SocketAddr,
    },
}

mod tag {
    pub const PING: u8 = 1;
    pub const PONG: u8 = 2;
}

impl DiscoMessage {
    fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Self::Ping { tx } => {
                w.u8(tag::PING);
                w.bytes(tx);
            }
            Self::Pong { tx, observed } => {
                w.u8(tag::PONG);
                w.bytes(tx);
                w.socket_addr(observed);
            }
        }
        w.finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(bytes);
        let message = match r.u8()? {
            tag::PING => Self::Ping { tx: r.array()? },
            tag::PONG => Self::Pong {
                tx: r.array()?,
                observed: r.socket_addr()?,
            },
            other => return Err(DecodeError::UnknownType(other)),
        };
        r.finish()?;
        Ok(message)
    }
}

fn header(sender: &NodeKey) -> [u8; HEADER_LEN] {
    let mut header = [0u8; HEADER_LEN];
    header[..4].copy_from_slice(&CONTROL_MAGIC);
    header[4] = VERSION;
    header[5..].copy_from_slice(sender.as_bytes());
    header
}

/// 封装一条发给 `recipient` 的控制报文。
pub fn seal(sender: &NodeSecret, recipient: &NodeKey, message: &DiscoMessage) -> Vec<u8> {
    let header = header(&sender.public_key());
    let payload = message.encode();
    let mut noise = snow::Builder::new(PATTERN.parse().expect("Noise 模式名是常量"))
        .local_private_key(sender.as_bytes())
        .and_then(|b| b.remote_public_key(recipient.as_bytes()))
        .and_then(|b| b.prologue(&header))
        .and_then(|b| b.build_initiator())
        .expect("参数都是定长的密钥和常量，不会失败");
    let mut out = vec![0u8; HEADER_LEN + 32 + payload.len() + 16];
    out[..HEADER_LEN].copy_from_slice(&header);
    let len = noise
        .write_message(&payload, &mut out[HEADER_LEN..])
        .expect("缓冲区按 Noise K 的长度算好了");
    out.truncate(HEADER_LEN + len);
    out
}

/// 拆开一条控制报文，交出发送方和内容。
///
/// `known` 判断发送方是不是认识的节点。返回 `false` 的报文不做任何密码学运算就被丢掉。
pub fn open(
    local: &NodeSecret,
    datagram: &[u8],
    known: impl FnOnce(&NodeKey) -> bool,
) -> Result<(NodeKey, DiscoMessage), DiscoError> {
    let Some((header, noise_message)) = datagram.split_first_chunk::<HEADER_LEN>() else {
        return Err(DiscoError::NotDisco);
    };
    if header[..4] != CONTROL_MAGIC {
        return Err(DiscoError::NotDisco);
    }
    if header[4] != VERSION {
        return Err(DiscoError::UnknownVersion(header[4]));
    }
    let sender = NodeKey::from_bytes(header[5..].try_into().expect("长度已经检查过"));
    if !known(&sender) {
        return Err(DiscoError::UnknownSender(sender));
    }

    let mut noise = snow::Builder::new(PATTERN.parse().expect("Noise 模式名是常量"))
        .local_private_key(local.as_bytes())
        .and_then(|b| b.remote_public_key(sender.as_bytes()))
        .and_then(|b| b.prologue(header))
        .and_then(|b| b.build_responder())
        .expect("参数都是定长的密钥和常量，不会失败");
    let mut payload = vec![0u8; noise_message.len()];
    let len = noise
        .read_message(noise_message, &mut payload)
        .map_err(|_| DiscoError::Crypto)?;
    let message = DiscoMessage::decode(&payload[..len]).map_err(DiscoError::Decode)?;
    Ok((sender, message))
}

/// 拆不开的原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscoError {
    /// 不是控制报文（太短或者魔数不对）。
    NotDisco,
    /// 不认识的格式版本。
    UnknownVersion(u8),
    /// 发送方不是认识的节点，没有尝试解密。
    UnknownSender(NodeKey),
    /// 认证失败：被篡改、发错了人，或者冒充了发送方。
    Crypto,
    /// 解密成功但内容不合法。
    Decode(DecodeError),
}

impl fmt::Display for DiscoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotDisco => f.write_str("不是控制报文"),
            Self::UnknownVersion(v) => write!(f, "不认识的控制报文版本 {v}"),
            Self::UnknownSender(key) => write!(f, "发送方 {key} 不是认识的节点"),
            Self::Crypto => f.write_str("控制报文认证失败"),
            Self::Decode(err) => write!(f, "控制报文内容不合法：{err}"),
        }
    }
}

impl std::error::Error for DiscoError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn ping() -> DiscoMessage {
        DiscoMessage::Ping { tx: [7; 12] }
    }

    #[test]
    fn round_trip() {
        let a = NodeSecret::generate();
        let b = NodeSecret::generate();
        let pong = DiscoMessage::Pong {
            tx: [7; 12],
            observed: "198.51.100.4:41641".parse().unwrap(),
        };
        for message in [ping(), pong] {
            let datagram = seal(&a, &b.public_key(), &message);
            assert_eq!(&datagram[..4], &CONTROL_MAGIC);
            let opened = open(&b, &datagram, |k| *k == a.public_key()).unwrap();
            assert_eq!(opened, (a.public_key(), message));
        }
    }

    #[test]
    fn unknown_sender_is_dropped_before_any_crypto() {
        let a = NodeSecret::generate();
        let b = NodeSecret::generate();
        let datagram = seal(&a, &b.public_key(), &ping());
        assert_eq!(
            open(&b, &datagram, |_| false),
            Err(DiscoError::UnknownSender(a.public_key()))
        );
    }

    #[test]
    fn only_the_recipient_can_open_it() {
        let a = NodeSecret::generate();
        let b = NodeSecret::generate();
        let eve = NodeSecret::generate();
        let datagram = seal(&a, &b.public_key(), &ping());
        assert_eq!(open(&eve, &datagram, |_| true), Err(DiscoError::Crypto));
    }

    #[test]
    fn cannot_forge_the_sender() {
        // Eve 封装一条报文，头部却声称是 A 发的
        let a = NodeSecret::generate();
        let b = NodeSecret::generate();
        let eve = NodeSecret::generate();
        let mut datagram = seal(&eve, &b.public_key(), &ping());
        datagram[5..HEADER_LEN].copy_from_slice(a.public_key().as_bytes());
        assert_eq!(open(&b, &datagram, |_| true), Err(DiscoError::Crypto));
    }

    #[test]
    fn any_flipped_byte_breaks_it() {
        let a = NodeSecret::generate();
        let b = NodeSecret::generate();
        let datagram = seal(&a, &b.public_key(), &ping());
        for i in 0..datagram.len() {
            let mut tampered = datagram.clone();
            tampered[i] ^= 0x01;
            assert!(
                open(&b, &tampered, |_| true).is_err(),
                "改了第 {i} 个字节还能打开"
            );
        }
    }

    #[test]
    fn rejects_short_and_foreign_datagrams() {
        let b = NodeSecret::generate();
        assert_eq!(open(&b, b"MSHR", |_| true), Err(DiscoError::NotDisco));
        let mut wireguard = vec![0u8; 148];
        wireguard[0] = 1;
        assert_eq!(open(&b, &wireguard, |_| true), Err(DiscoError::NotDisco));

        let a = NodeSecret::generate();
        let mut datagram = seal(&a, &b.public_key(), &ping());
        datagram[4] = 9;
        assert_eq!(
            open(&b, &datagram, |_| true),
            Err(DiscoError::UnknownVersion(9))
        );
    }
}
