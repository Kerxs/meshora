//! 局域网广播与组播：从虚拟网卡读到的哪些报文，要复制一份发给网里的每个节点。
//!
//! 局域网游戏找房间靠的不是 IP，而是往整个网段喊一声：
//!
//! | 游戏 | 目的地址 |
//! | --- | --- |
//! | Minecraft Java 版「对局域网开放」 | 组播 `224.0.2.60:4445` |
//! | 魔兽争霸 III 的局域网游戏 | UDP 6112 上的广播 |
//!
//! 这些目的地址不属于任何一个 peer 的网段，按单播的规矩找不到接收方，原本会被直接丢掉 ——
//! 结果就是互相 ping 得通，房间列表里却看不见对方。这里认出它们，交给引擎复制给所有节点。
//!
//! 只认 IPv4 的 UDP：游戏的发现报文都是 UDP；IGMP 之类的组播控制报文、IPv6 的邻居发现不该出网。

use std::net::Ipv4Addr;

/// 不转发的目的端口：操作系统自己的服务发现。
///
/// Windows 会在每块网卡上持续发这些报文，全部复制出去只是噪音；而且它们带着本机的计算机名、
/// 共享的设备和服务，不该在所有节点之间扩散。没有游戏靠它们找房间。
const QUIET_PORTS: [u16; 5] = [
    137,  // NetBIOS 名称服务
    138,  // NetBIOS 数据报服务
    1900, // SSDP
    5353, // mDNS
    5355, // LLMNR
];

const IPV4_HEADER_MIN: usize = 20;
const PROTO_UDP: u8 = 17;

/// 算出网段的定向广播地址（网段里主机位全 1 的那个）。
///
/// `/31`、`/32` 没有广播地址（RFC 3021），返回 `None`；前缀长度超过 32 也是 `None`。
pub(crate) fn directed_broadcast(addr: Ipv4Addr, prefix_len: u8) -> Option<Ipv4Addr> {
    if prefix_len >= 31 {
        return None;
    }
    let host_bits = u32::MAX >> prefix_len;
    Some(Ipv4Addr::from(u32::from(addr) | host_bits))
}

/// 这个从虚拟网卡读到的报文是不是要复制给网里每个节点的局域网广播 / 组播。
///
/// `lan_broadcast` 是 overlay 网段的定向广播地址（见 [`directed_broadcast`]）；
/// 有的游戏按网卡的掩码自己算出广播地址再发，而不是发往 `255.255.255.255`。
pub(crate) fn is_lan_broadcast(packet: &[u8], lan_broadcast: Option<Ipv4Addr>) -> bool {
    if packet.len() < IPV4_HEADER_MIN || packet[0] >> 4 != 4 {
        return false;
    }
    let header_len = usize::from(packet[0] & 0x0f) * 4;
    if header_len < IPV4_HEADER_MIN || packet.len() < header_len || packet[9] != PROTO_UDP {
        return false;
    }

    let dst = Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]);
    let broadcast = dst == Ipv4Addr::BROADCAST || dst.is_multicast() || Some(dst) == lan_broadcast;
    if !broadcast {
        return false;
    }

    // 分片偏移为 0 的报文带着 UDP 头，可以看目的端口；后续分片没有端口可看，照常复制 ——
    // 否则收到的一方凑不齐分片。操作系统的服务发现报文都很小，不会分片
    let fragment_offset = u16::from_be_bytes([packet[6], packet[7]]) & 0x1fff;
    if fragment_offset == 0 {
        let Some(port) = packet.get(header_len + 2..header_len + 4) else {
            return false;
        };
        let dst_port = u16::from_be_bytes([port[0], port[1]]);
        if QUIET_PORTS.contains(&dst_port) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一个 IPv4 UDP 报文：20 字节 IP 头 + 8 字节 UDP 头 + 载荷。校验和不管。
    fn udp(dst: Ipv4Addr, dst_port: u16) -> Vec<u8> {
        let mut packet = vec![0u8; 28 + 4];
        packet[0] = 0x45;
        let total = packet.len() as u16;
        packet[2..4].copy_from_slice(&total.to_be_bytes());
        packet[8] = 64;
        packet[9] = PROTO_UDP;
        packet[12..16].copy_from_slice(&[100, 64, 0, 1]);
        packet[16..20].copy_from_slice(&dst.octets());
        packet[20..22].copy_from_slice(&50000u16.to_be_bytes());
        packet[22..24].copy_from_slice(&dst_port.to_be_bytes());
        packet
    }

    const LAN: Option<Ipv4Addr> = Some(Ipv4Addr::new(100, 127, 255, 255));

    #[test]
    fn directed_broadcast_of_the_overlay() {
        assert_eq!(
            directed_broadcast(Ipv4Addr::new(100, 64, 0, 5), 10),
            Some(Ipv4Addr::new(100, 127, 255, 255))
        );
        assert_eq!(
            directed_broadcast(Ipv4Addr::new(192, 168, 1, 20), 24),
            Some(Ipv4Addr::new(192, 168, 1, 255))
        );
        assert_eq!(directed_broadcast(Ipv4Addr::new(10, 0, 0, 1), 31), None);
        assert_eq!(directed_broadcast(Ipv4Addr::new(10, 0, 0, 1), 32), None);
        assert_eq!(directed_broadcast(Ipv4Addr::new(10, 0, 0, 1), 33), None);
        assert_eq!(
            directed_broadcast(Ipv4Addr::new(10, 0, 0, 1), 0),
            Some(Ipv4Addr::BROADCAST)
        );
    }

    #[test]
    fn game_discovery_is_flooded() {
        // Minecraft Java 版的局域网公告
        assert!(is_lan_broadcast(
            &udp(Ipv4Addr::new(224, 0, 2, 60), 4445),
            LAN
        ));
        // 魔兽争霸 III
        assert!(is_lan_broadcast(&udp(Ipv4Addr::BROADCAST, 6112), LAN));
        // 按网卡掩码算出来的定向广播
        assert!(is_lan_broadcast(
            &udp(Ipv4Addr::new(100, 127, 255, 255), 6112),
            LAN
        ));
    }

    #[test]
    fn directed_broadcast_needs_the_lan_to_be_known() {
        assert!(!is_lan_broadcast(
            &udp(Ipv4Addr::new(100, 127, 255, 255), 6112),
            None
        ));
    }

    #[test]
    fn unicast_is_not_flooded() {
        assert!(!is_lan_broadcast(
            &udp(Ipv4Addr::new(100, 64, 0, 2), 6112),
            LAN
        ));
        // 别的网段的广播地址不是我们的
        assert!(!is_lan_broadcast(
            &udp(Ipv4Addr::new(192, 168, 1, 255), 6112),
            LAN
        ));
    }

    #[test]
    fn os_service_discovery_stays_home() {
        for (dst, port) in [
            (Ipv4Addr::new(224, 0, 0, 251), 5353),     // mDNS
            (Ipv4Addr::new(224, 0, 0, 252), 5355),     // LLMNR
            (Ipv4Addr::new(239, 255, 255, 250), 1900), // SSDP
            (Ipv4Addr::new(100, 127, 255, 255), 137),  // NetBIOS 名称服务
            (Ipv4Addr::BROADCAST, 138),                // NetBIOS 数据报
        ] {
            assert!(!is_lan_broadcast(&udp(dst, port), LAN), "{dst}:{port}");
        }
    }

    #[test]
    fn only_udp_is_flooded() {
        let mut igmp = udp(Ipv4Addr::new(224, 0, 0, 22), 0);
        igmp[9] = 2;
        assert!(!is_lan_broadcast(&igmp, LAN));
        let mut icmp = udp(Ipv4Addr::BROADCAST, 0);
        icmp[9] = 1;
        assert!(!is_lan_broadcast(&icmp, LAN));
    }

    #[test]
    fn later_fragments_follow_the_first() {
        // 后续分片：偏移不为 0，没有 UDP 头，也要复制
        let mut fragment = vec![0u8; 20 + 8];
        fragment[0] = 0x45;
        fragment[6..8].copy_from_slice(&185u16.to_be_bytes());
        fragment[9] = PROTO_UDP;
        fragment[16..20].copy_from_slice(&Ipv4Addr::BROADCAST.octets());
        assert!(is_lan_broadcast(&fragment, LAN));
    }

    #[test]
    fn malformed_packets_are_not_flooded() {
        assert!(!is_lan_broadcast(&[], LAN));
        assert!(!is_lan_broadcast(&[0x45; 10], LAN));
        // IPv6
        let mut v6 = udp(Ipv4Addr::BROADCAST, 6112);
        v6[0] = 0x60;
        assert!(!is_lan_broadcast(&v6, LAN));
        // 头长字段（60 字节）比整个报文（32 字节）还长
        let mut long_header = udp(Ipv4Addr::BROADCAST, 6112);
        long_header[0] = 0x4f;
        assert!(!is_lan_broadcast(&long_header, LAN));
        // 只有 IP 头、放不下 UDP 端口
        let mut truncated = udp(Ipv4Addr::BROADCAST, 6112);
        truncated.truncate(21);
        assert!(!is_lan_broadcast(&truncated, LAN));
    }
}
