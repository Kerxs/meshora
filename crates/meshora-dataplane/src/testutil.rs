//! 测试共用的小工具。

use std::net::IpAddr;

/// 一个最小的 IPv4 报文：20 字节头加载荷。boringtun 只看版本、长度和地址，校验和不管。
pub(crate) fn ipv4(src: IpAddr, dst: IpAddr, payload: &[u8]) -> Vec<u8> {
    let (IpAddr::V4(src), IpAddr::V4(dst)) = (src, dst) else {
        panic!("只造 IPv4 报文");
    };
    let total = 20 + payload.len();
    let mut packet = vec![0u8; total];
    packet[0] = 0x45;
    packet[2..4].copy_from_slice(&(total as u16).to_be_bytes());
    packet[8] = 64;
    packet[9] = 1; // ICMP
    packet[12..16].copy_from_slice(&src.octets());
    packet[16..20].copy_from_slice(&dst.octets());
    packet[20..].copy_from_slice(payload);
    packet
}
