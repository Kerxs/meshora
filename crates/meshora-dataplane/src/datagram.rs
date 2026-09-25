//! 共享 UDP socket 上的报文分流。
//!
//! 控制报文（端点探测、打洞、链路探测）必须和 WireGuard 从同一个 socket 收发：
//! 打洞凿出来的 NAT 映射只对这个本地端口有效。于是同一个 socket 上混着两类报文，
//! 收到时得先分开 —— 靠的就是 [`classify`]。

pub use meshora_types::CONTROL_MAGIC;

// 魔数定义在 meshora-types（控制面也要用），这里钉住它在数据面这一侧的约束。
// 选它只有两条硬约束：
//
// - 前 4 字节按小端 u32 读出来不能落在 1..=4 —— 那是 WireGuard 的四种消息类型
// - 首字节不能在 0x00–0x03 —— 那是 STUN 的首字节范围（RFC 7983）。
//   按设计端点探测用的是自己的报文，但给以后改用标准 STUN 留一条路
//
// 魔数只用来分流，不是安全机制：谁都能发一个以它开头的报文。

const _: () = {
    assert!(!matches!(u32::from_le_bytes(CONTROL_MAGIC), 1..=4));
    assert!(CONTROL_MAGIC[0] > 0x03);
};

// WireGuard 消息的长度，括号里是各字段的字节数
/// 类型与保留 (4) + 发送方索引 (4) + 临时公钥 (32) + 加密的静态公钥 (32+16)
/// + 加密的时间戳 (12+16) + mac1 (16) + mac2 (16)
const HANDSHAKE_INITIATION_LEN: usize = 148;
/// 类型与保留 (4) + 发送方索引 (4) + 接收方索引 (4) + 临时公钥 (32)
/// + 加密的空载荷 (0+16) + mac1 (16) + mac2 (16)
const HANDSHAKE_RESPONSE_LEN: usize = 92;
/// 类型与保留 (4) + 接收方索引 (4) + nonce (24) + 加密的 cookie (16+16)
const COOKIE_REPLY_LEN: usize = 64;
/// 类型与保留 (4) + 接收方索引 (4) + 计数器 (8) + 认证标签 (16)，明文可以为空（keepalive）
const TRANSPORT_DATA_MIN_LEN: usize = 32;

/// 一个收到的 UDP 报文该交给谁。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatagramKind {
    /// WireGuard 报文，交给 WireGuard。
    WireGuard(WgMessage),
    /// 以 [`CONTROL_MAGIC`] 开头的 Meshora 控制报文，原样交给控制面
    /// （见 [`Event::ControlDatagram`](crate::Event::ControlDatagram)）。
    Control,
    /// 都不是，丢弃。
    Unknown,
}

/// WireGuard 的四种消息。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WgMessage {
    /// 握手发起（类型 1），固定 148 字节。
    HandshakeInitiation,
    /// 握手响应（类型 2），固定 92 字节。
    HandshakeResponse,
    /// Cookie 回复（类型 3），固定 64 字节。负载过高时用它要求对端先证明自己的地址。
    CookieReply,
    /// 数据报文（类型 4），至少 32 字节。keepalive 就是正好 32 字节的数据报文。
    TransportData,
}

/// 判断一个收到的 UDP 报文该交给谁。
///
/// WireGuard 的判定规则和 boringtun 的 `Tunn::parse_incoming_packet` 一致：
/// 前 4 字节按小端 u32 读出来是消息类型（也就是说 3 个保留字节必须为零），再按类型核对长度。
///
/// 数据报文**只检查下限，不检查长度是不是 16 的倍数**。WireGuard 规范要求把明文填充到
/// 16 的倍数，但填充不会超过 MTU —— 贴着 MTU 的报文填不满；boringtun 更是干脆不填充。
/// 加上这条检查，被当成垃圾丢掉的会是大量正常报文。
pub fn classify(datagram: &[u8]) -> DatagramKind {
    let Some(&header) = datagram.first_chunk::<4>() else {
        return DatagramKind::Unknown;
    };
    if header == CONTROL_MAGIC {
        return DatagramKind::Control;
    }
    let message = match (u32::from_le_bytes(header), datagram.len()) {
        (1, HANDSHAKE_INITIATION_LEN) => WgMessage::HandshakeInitiation,
        (2, HANDSHAKE_RESPONSE_LEN) => WgMessage::HandshakeResponse,
        (3, COOKIE_REPLY_LEN) => WgMessage::CookieReply,
        (4, len) if len >= TRANSPORT_DATA_MIN_LEN => WgMessage::TransportData,
        _ => return DatagramKind::Unknown,
    };
    DatagramKind::WireGuard(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一个类型为 `ty`、长度为 `len` 的 WireGuard 报文（除类型外全为零）
    fn wg(ty: u8, len: usize) -> Vec<u8> {
        let mut datagram = vec![0; len];
        datagram[0] = ty;
        datagram
    }

    fn wg_kind(message: WgMessage) -> DatagramKind {
        DatagramKind::WireGuard(message)
    }

    #[test]
    fn handshake_messages_must_be_exact_length() {
        let cases = [
            (1, HANDSHAKE_INITIATION_LEN, WgMessage::HandshakeInitiation),
            (2, HANDSHAKE_RESPONSE_LEN, WgMessage::HandshakeResponse),
            (3, COOKIE_REPLY_LEN, WgMessage::CookieReply),
        ];
        for (ty, len, message) in cases {
            assert_eq!(classify(&wg(ty, len)), wg_kind(message), "类型 {ty}");
            assert_eq!(
                classify(&wg(ty, len - 1)),
                DatagramKind::Unknown,
                "类型 {ty} 短一字节"
            );
            assert_eq!(
                classify(&wg(ty, len + 1)),
                DatagramKind::Unknown,
                "类型 {ty} 长一字节"
            );
        }
    }

    #[test]
    fn transport_data_has_only_a_lower_bound() {
        let data = wg_kind(WgMessage::TransportData);
        assert_eq!(classify(&wg(4, 31)), DatagramKind::Unknown);
        assert_eq!(classify(&wg(4, 32)), data, "keepalive");
        // 不是 16 的倍数：贴着 MTU、填充被截断的报文就是这样
        assert_eq!(classify(&wg(4, 33)), data);
        assert_eq!(classify(&wg(4, 1452)), data);
    }

    #[test]
    fn reserved_bytes_must_be_zero() {
        for i in 1..4 {
            let mut datagram = wg(1, HANDSHAKE_INITIATION_LEN);
            datagram[i] = 1;
            assert_eq!(classify(&datagram), DatagramKind::Unknown, "保留字节 {i}");
        }
    }

    #[test]
    fn unknown_message_types() {
        assert_eq!(classify(&wg(0, 148)), DatagramKind::Unknown);
        assert_eq!(classify(&wg(5, 148)), DatagramKind::Unknown);
    }

    #[test]
    fn control_datagrams_start_with_magic() {
        assert_eq!(classify(&CONTROL_MAGIC), DatagramKind::Control);

        let mut datagram = CONTROL_MAGIC.to_vec();
        datagram.extend_from_slice(&[0; 144]);
        // 恰好 148 字节也不会被当成握手发起
        assert_eq!(classify(&datagram), DatagramKind::Control);
    }

    #[test]
    fn short_datagrams_are_unknown() {
        assert_eq!(classify(&[]), DatagramKind::Unknown);
        assert_eq!(classify(&CONTROL_MAGIC[..3]), DatagramKind::Unknown);
        assert_eq!(classify(&[4, 0, 0]), DatagramKind::Unknown);
    }

    #[test]
    fn stun_is_not_mistaken_for_wireguard() {
        // STUN Binding 请求的 20 字节头：类型 0x0001、长度 0、magic cookie、事务 ID
        let mut stun = vec![0x00, 0x01, 0x00, 0x00, 0x21, 0x12, 0xA4, 0x42];
        stun.extend_from_slice(&[0; 12]);
        assert_eq!(classify(&stun), DatagramKind::Unknown);
    }
}
