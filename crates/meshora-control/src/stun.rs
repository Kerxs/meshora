//! 最小的 STUN 客户端（RFC 5389）：只发 Binding 请求，只读回应里的映射地址。
//!
//! 直连模式（不用服务器）靠它问公共 STUN 服务器"你看到我从哪来"，得到本机在公网上的端点，
//! 写进连接码给对方。请求经数据面的共享 socket 发（[`DataPlane::send_stun`]），
//! 问出来的才是 WireGuard 那个端口的映射。
//!
//! 不做认证（公共服务器本来就没有共享密钥）：只认自己发过的事务 ID、只认发过请求的服务器地址。
//! 伪造的回应最多让连接码里多一个错的候选端点，打不通而已 —— 和协调服务的探测一样。
//!
//! [`DataPlane::send_stun`]: meshora_dataplane::DataPlane::send_stun

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use meshora_dataplane::{STUN_HEADER_LEN, STUN_MAGIC_COOKIE};

/// 事务 ID 的长度。
pub const TX_LEN: usize = 12;

/// 一个 STUN 事务 ID。
pub type TxId = [u8; TX_LEN];

const BINDING_REQUEST: u16 = 0x0001;
const BINDING_SUCCESS: u16 = 0x0101;
const MAPPED_ADDRESS: u16 = 0x0001;
const XOR_MAPPED_ADDRESS: u16 = 0x0020;
const FAMILY_V4: u8 = 0x01;
const FAMILY_V6: u8 = 0x02;

/// 一个不带属性的 Binding 请求。
pub fn request(tx: &TxId) -> [u8; STUN_HEADER_LEN] {
    let mut out = [0u8; STUN_HEADER_LEN];
    out[0..2].copy_from_slice(&BINDING_REQUEST.to_be_bytes());
    // 长度 0：没有属性
    out[4..8].copy_from_slice(&STUN_MAGIC_COOKIE);
    out[8..].copy_from_slice(tx);
    out
}

/// 读一个 Binding 成功回应：事务 ID 和映射地址。不是成功回应、没有地址属性时返回 `None`。
///
/// 优先读 XOR-MAPPED-ADDRESS；只有老式的 MAPPED-ADDRESS 时也认（RFC 3489 的服务器）。
pub fn parse_response(datagram: &[u8]) -> Option<(TxId, SocketAddr)> {
    let header = datagram.first_chunk::<STUN_HEADER_LEN>()?;
    if u16::from_be_bytes([header[0], header[1]]) != BINDING_SUCCESS
        || header[4..8] != STUN_MAGIC_COOKIE
    {
        return None;
    }
    let length = usize::from(u16::from_be_bytes([header[2], header[3]]));
    let body = datagram.get(STUN_HEADER_LEN..STUN_HEADER_LEN + length)?;
    let mut tx = TxId::default();
    tx.copy_from_slice(&header[8..]);

    let mut plain = None;
    let mut rest = body;
    while let Some(attr) = rest.first_chunk::<4>() {
        let kind = u16::from_be_bytes([attr[0], attr[1]]);
        let len = usize::from(u16::from_be_bytes([attr[2], attr[3]]));
        let value = rest.get(4..4 + len)?;
        match kind {
            XOR_MAPPED_ADDRESS => return Some((tx, address(value, Some(&tx))?)),
            MAPPED_ADDRESS => plain = address(value, None),
            _ => {}
        }
        // 属性按 4 字节对齐，填充不计入长度
        let padded = (4 + len).div_ceil(4) * 4;
        rest = rest.get(padded..).unwrap_or_default();
    }
    plain.map(|addr| (tx, addr))
}

/// 读一个地址属性的值。`xor` 是事务 ID 时按 XOR-MAPPED-ADDRESS 解：端口和地址和 cookie（IPv6 还要接上事务 ID）异或过
fn address(value: &[u8], xor: Option<&TxId>) -> Option<SocketAddr> {
    let family = *value.get(1)?;
    let port = u16::from_be_bytes([*value.get(2)?, *value.get(3)?]);
    let mut mask = [0u8; 16];
    if let Some(tx) = xor {
        mask[..4].copy_from_slice(&STUN_MAGIC_COOKIE);
        mask[4..].copy_from_slice(tx);
    }
    let port = port ^ u16::from_be_bytes([mask[0], mask[1]]);
    let ip = match family {
        FAMILY_V4 => {
            let raw: [u8; 4] = value.get(4..8)?.try_into().ok()?;
            let octets: [u8; 4] = std::array::from_fn(|i| raw[i] ^ mask[i]);
            IpAddr::V4(Ipv4Addr::from(octets))
        }
        FAMILY_V6 => {
            let raw: [u8; 16] = value.get(4..20)?.try_into().ok()?;
            let octets: [u8; 16] = std::array::from_fn(|i| raw[i] ^ mask[i]);
            IpAddr::V6(Ipv6Addr::from(octets))
        }
        _ => return None,
    };
    Some(SocketAddr::new(ip, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 5769 第 2.2 节的 IPv4 回应：事务 ID、SOFTWARE、XOR-MAPPED-ADDRESS 照抄，
    /// 映射地址是 192.0.2.1:32853。MESSAGE-INTEGRITY 和 FINGERPRINT 这里不校验，值填零
    fn rfc5769_ipv4_response() -> Vec<u8> {
        let mut m = vec![0x01, 0x01, 0x00, 0x3c, 0x21, 0x12, 0xa4, 0x42];
        m.extend_from_slice(&[
            0xb7, 0xe7, 0xa7, 0x01, 0xbc, 0x34, 0xd6, 0x86, 0xfa, 0x87, 0xdf, 0xae,
        ]);
        // SOFTWARE "test vector"，11 字节，补一个空格对齐
        m.extend_from_slice(&[0x80, 0x22, 0x00, 0x0b]);
        m.extend_from_slice(b"test vector ");
        // XOR-MAPPED-ADDRESS
        m.extend_from_slice(&[0x00, 0x20, 0x00, 0x08, 0x00, 0x01, 0xa1, 0x47]);
        m.extend_from_slice(&[0xe1, 0x12, 0xa6, 0x43]);
        // MESSAGE-INTEGRITY（20 字节）
        m.extend_from_slice(&[0x00, 0x08, 0x00, 0x14]);
        m.extend_from_slice(&[0; 20]);
        // FINGERPRINT
        m.extend_from_slice(&[0x80, 0x28, 0x00, 0x04, 0, 0, 0, 0]);
        assert_eq!(m.len(), STUN_HEADER_LEN + 0x3c);
        m
    }

    #[test]
    fn reads_the_rfc5769_ipv4_vector() {
        let (tx, addr) = parse_response(&rfc5769_ipv4_response()).unwrap();
        assert_eq!(
            tx,
            [
                0xb7, 0xe7, 0xa7, 0x01, 0xbc, 0x34, 0xd6, 0x86, 0xfa, 0x87, 0xdf, 0xae
            ]
        );
        assert_eq!(addr, "192.0.2.1:32853".parse().unwrap());
    }

    #[test]
    fn reads_the_rfc5769_ipv6_vector() {
        // RFC 5769 第 2.3 节：映射地址 [2001:db8:1234:5678:11:2233:4455:6677]:32853
        let tx = [
            0xb7, 0xe7, 0xa7, 0x01, 0xbc, 0x34, 0xd6, 0x86, 0xfa, 0x87, 0xdf, 0xae,
        ];
        let mut m = vec![0x01, 0x01, 0x00, 0x18, 0x21, 0x12, 0xa4, 0x42];
        m.extend_from_slice(&tx);
        m.extend_from_slice(&[0x00, 0x20, 0x00, 0x14, 0x00, 0x02, 0xa1, 0x47]);
        m.extend_from_slice(&[
            0x01, 0x13, 0xa9, 0xfa, 0xa5, 0xd3, 0xf1, 0x79, 0xbc, 0x25, 0xf4, 0xb5, 0xbe, 0xd2,
            0xb9, 0xd9,
        ]);
        let (_, addr) = parse_response(&m).unwrap();
        assert_eq!(
            addr,
            "[2001:db8:1234:5678:11:2233:4455:6677]:32853"
                .parse()
                .unwrap()
        );
    }

    #[test]
    fn the_request_is_what_the_dataplane_lets_through() {
        let tx = [7; TX_LEN];
        let req = request(&tx);
        assert!(meshora_dataplane::is_stun_binding_request(&req));
        assert_eq!(&req[8..], &tx);
    }

    #[test]
    fn falls_back_to_the_plain_mapped_address() {
        let tx = [1; TX_LEN];
        let mut m = vec![0x01, 0x01, 0x00, 0x0c, 0x21, 0x12, 0xa4, 0x42];
        m.extend_from_slice(&tx);
        m.extend_from_slice(&[
            0x00, 0x01, 0x00, 0x08, 0x00, 0x01, 0x1f, 0x90, 203, 0, 113, 9,
        ]);
        assert_eq!(
            parse_response(&m),
            Some((tx, "203.0.113.9:8080".parse().unwrap()))
        );
    }

    #[test]
    fn rejects_requests_errors_and_truncation() {
        let tx = [2; TX_LEN];
        assert_eq!(parse_response(&request(&tx)), None, "请求不是回应");
        let mut error = rfc5769_ipv4_response();
        error[1] = 0x11;
        assert_eq!(parse_response(&error), None, "出错回应没有地址");
        let full = rfc5769_ipv4_response();
        assert_eq!(
            parse_response(&full[..full.len() - 1]),
            None,
            "长度字段对不上"
        );
        assert_eq!(parse_response(&full[..10]), None);
    }
}
