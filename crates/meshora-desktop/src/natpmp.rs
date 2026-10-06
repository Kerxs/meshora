//! NAT-PMP（RFC 6886）和它的后继 PCP（RFC 6887）：请路由器开一个 UDP 端口。UPnP 开不了时的后备。
//! 两个协议用网关的同一个端口（5351），先试 PCP，网关不懂再试 NAT-PMP。
//!
//! 和 UPnP 比：不用组播发现（安卓上 UPnP 的组播搜索会被系统拒，报 Operation not permitted），
//! 直接往网关的 5351 端口发一个 12 字节的请求，回 16 字节。不少路由器（尤其是苹果、OpenWrt、一部分国产的）只开了它。
//!
//! 网关的地址拿不到系统的路由表时，按惯例猜：本机局域网地址所在网段的第一个地址（192.168.1.x → 192.168.1.1）。

use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::time::Duration;

/// 网关上 NAT-PMP 的端口
const PORT: u16 = 5351;
const OP_EXTERNAL: u8 = 0;
const OP_MAP_UDP: u8 = 1;

/// 问公网地址的请求
pub fn external_request() -> [u8; 2] {
    [0, OP_EXTERNAL]
}

/// 映射一个 UDP 端口的请求：`lifetime` 秒，0 是删掉
pub fn map_request(internal: u16, external: u16, lifetime: u32) -> [u8; 12] {
    let mut out = [0u8; 12];
    out[1] = OP_MAP_UDP;
    out[4..6].copy_from_slice(&internal.to_be_bytes());
    out[6..8].copy_from_slice(&external.to_be_bytes());
    out[8..12].copy_from_slice(&lifetime.to_be_bytes());
    out
}

/// 读公网地址的回应。结果码不是 0（成功）时是 `None`
pub fn parse_external(reply: &[u8]) -> Option<Ipv4Addr> {
    let r: &[u8; 12] = reply.get(..12)?.try_into().ok()?;
    if r[0] != 0 || r[1] != 128 + OP_EXTERNAL || u16::from_be_bytes([r[2], r[3]]) != 0 {
        return None;
    }
    Some(Ipv4Addr::new(r[8], r[9], r[10], r[11]))
}

/// 读映射的回应：路由器实际给的外部端口和租期
pub fn parse_map(reply: &[u8], internal: u16) -> Option<(u16, u32)> {
    let r: &[u8; 16] = reply.get(..16)?.try_into().ok()?;
    if r[0] != 0 || r[1] != 128 + OP_MAP_UDP || u16::from_be_bytes([r[2], r[3]]) != 0 {
        return None;
    }
    if u16::from_be_bytes([r[8], r[9]]) != internal {
        return None;
    }
    let external = u16::from_be_bytes([r[10], r[11]]);
    let lifetime = u32::from_be_bytes([r[12], r[13], r[14], r[15]]);
    (external != 0).then_some((external, lifetime))
}

/// 猜网关：这个局域网地址所在网段的第一个地址
pub fn guess_gateway(lan: Ipv4Addr, prefix_len: u8) -> Option<Ipv4Addr> {
    if !(8..=30).contains(&prefix_len) {
        return None;
    }
    let mask = u32::MAX << (32 - u32::from(prefix_len));
    Some(Ipv4Addr::from((u32::from(lan) & mask) + 1))
}

/// 发一个请求、等回应。按 RFC 从 250 毫秒起每次加倍，试 3 次（最多等不到 2 秒）
fn ask(gateway: Ipv4Addr, request: &[u8]) -> io::Result<Vec<u8>> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    socket.connect(SocketAddrV4::new(gateway, PORT))?;
    let mut wait = Duration::from_millis(250);
    let mut buf = [0u8; 64];
    for _ in 0..3 {
        socket.send(request)?;
        socket.set_read_timeout(Some(wait))?;
        match socket.recv(&mut buf) {
            Ok(len) => return Ok(buf[..len].to_vec()),
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                wait *= 2;
            }
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::new(io::ErrorKind::TimedOut, "网关没回 NAT-PMP"))
}

/// 请 `gateway` 把 UDP `port` 映射出去，交回外面看到的端点。阻塞，最多几秒
pub fn map(gateway: Ipv4Addr, port: u16, lifetime: u32) -> io::Result<SocketAddr> {
    let external = parse_external(&ask(gateway, &external_request())?)
        .ok_or_else(|| io::Error::other("网关不肯说公网地址"))?;
    let (mapped, _) = parse_map(&ask(gateway, &map_request(port, port, lifetime))?, port)
        .ok_or_else(|| io::Error::other("网关不肯开这个端口"))?;
    Ok(SocketAddr::V4(SocketAddrV4::new(external, mapped)))
}

/// 撤掉映射（租期 0）。尽力而为
pub fn unmap(gateway: Ipv4Addr, port: u16) {
    let _ = ask(gateway, &map_request(port, 0, 0));
}

// ---------- PCP（RFC 6887） ----------

const PCP_VERSION: u8 = 2;
const PCP_OP_MAP: u8 = 1;
const PCP_UDP: u8 = 17;

/// IPv4 地址写成 IPv4 映射的 IPv6（::ffff:a.b.c.d），PCP 里地址一律 16 字节
fn mapped_v6(ip: Ipv4Addr) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[10] = 0xff;
    out[11] = 0xff;
    out[12..].copy_from_slice(&ip.octets());
    out
}

/// MAP 请求：头 24 字节（版本、操作码、租期、本机地址），MAP 部分 36 字节（随机数、协议、内部端口、
/// 希望的外部端口和地址）。`lifetime` 为 0 是删掉
pub fn pcp_map_request(nonce: [u8; 12], client: Ipv4Addr, port: u16, lifetime: u32) -> [u8; 60] {
    let mut out = [0u8; 60];
    out[0] = PCP_VERSION;
    out[1] = PCP_OP_MAP;
    out[4..8].copy_from_slice(&lifetime.to_be_bytes());
    out[8..24].copy_from_slice(&mapped_v6(client));
    out[24..36].copy_from_slice(&nonce);
    out[36] = PCP_UDP;
    out[40..42].copy_from_slice(&port.to_be_bytes());
    out[42..44].copy_from_slice(&port.to_be_bytes());
    // 希望的外部地址：IPv4 的"随便哪个"（::ffff:0.0.0.0）
    out[44..60].copy_from_slice(&mapped_v6(Ipv4Addr::UNSPECIFIED));
    out
}

/// 读 MAP 的回应：随机数、内部端口要对得上，结果码是 0（成功）。交回外部端点和租期
pub fn parse_pcp_map(reply: &[u8], nonce: [u8; 12], port: u16) -> Option<(SocketAddr, u32)> {
    let r: &[u8; 60] = reply.get(..60)?.try_into().ok()?;
    if r[0] != PCP_VERSION || r[1] != 0x80 | PCP_OP_MAP || r[3] != 0 {
        return None;
    }
    if r[24..36] != nonce || r[36] != PCP_UDP || u16::from_be_bytes([r[40], r[41]]) != port {
        return None;
    }
    let lifetime = u32::from_be_bytes([r[4], r[5], r[6], r[7]]);
    let external = u16::from_be_bytes([r[42], r[43]]);
    // 外部地址得是 IPv4 映射的那种
    if r[44..54] != [0u8; 10] || r[54..56] != [0xff, 0xff] || external == 0 {
        return None;
    }
    let ip = Ipv4Addr::new(r[56], r[57], r[58], r[59]);
    Some((SocketAddr::V4(SocketAddrV4::new(ip, external)), lifetime))
}

/// 用 PCP 请 `gateway` 把 UDP `port` 映射出去。`client` 是本机的局域网地址（PCP 要求写在请求里）。
/// 交回外面看到的端点和这次用的随机数（删映射时要用同一个）
pub fn pcp_map(
    gateway: Ipv4Addr,
    client: Ipv4Addr,
    port: u16,
    lifetime: u32,
) -> io::Result<(SocketAddr, [u8; 12])> {
    let mut nonce = [0u8; 12];
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut nonce);
    let reply = ask(gateway, &pcp_map_request(nonce, client, port, lifetime))?;
    let (outside, _) = parse_pcp_map(&reply, nonce, port)
        .ok_or_else(|| io::Error::other("网关不懂 PCP，或者不肯开这个端口"))?;
    Ok((outside, nonce))
}

/// 用 PCP 撤掉映射（同一个随机数、租期 0）。尽力而为
pub fn pcp_unmap(gateway: Ipv4Addr, client: Ipv4Addr, port: u16, nonce: [u8; 12]) {
    let _ = ask(gateway, &pcp_map_request(nonce, client, port, 0));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_laid_out_as_the_rfc_says() {
        assert_eq!(external_request(), [0, 0]);
        assert_eq!(
            map_request(41641, 41641, 7200),
            [0, 1, 0, 0, 0xa2, 0xa9, 0xa2, 0xa9, 0, 0, 0x1c, 0x20]
        );
    }

    #[test]
    fn replies_are_read() {
        let ext = [0, 128, 0, 0, 0, 0, 0, 9, 203, 0, 113, 9];
        assert_eq!(parse_external(&ext), Some(Ipv4Addr::new(203, 0, 113, 9)));
        let mut failed = ext;
        failed[3] = 3; // 网关没联网
        assert_eq!(parse_external(&failed), None);

        let map = [
            0, 129, 0, 0, 0, 0, 0, 9, 0xa2, 0xa9, 0xa2, 0xaa, 0, 0, 0x1c, 0x20,
        ];
        assert_eq!(
            parse_map(&map, 41641),
            Some((41642, 7200)),
            "路由器可以给另一个外部端口"
        );
        assert_eq!(parse_map(&map, 5000), None, "不是我们问的那个端口");
        assert_eq!(parse_map(&map[..10], 41641), None);
    }

    #[test]
    fn a_pcp_map_request_is_laid_out_as_the_rfc_says() {
        let nonce = [7u8; 12];
        let req = pcp_map_request(nonce, Ipv4Addr::new(192, 168, 1, 127), 41641, 7200);
        assert_eq!(&req[..4], &[2, 1, 0, 0]);
        assert_eq!(&req[4..8], &7200u32.to_be_bytes());
        assert_eq!(
            &req[8..24],
            &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 192, 168, 1, 127]
        );
        assert_eq!(&req[24..36], &nonce);
        assert_eq!(req[36], 17);
        assert_eq!(&req[40..44], &[0xa2, 0xa9, 0xa2, 0xa9]);
    }

    #[test]
    fn a_pcp_map_reply_is_read() {
        let nonce = [7u8; 12];
        let mut reply = [0u8; 60];
        reply[0] = 2;
        reply[1] = 0x81;
        reply[4..8].copy_from_slice(&7200u32.to_be_bytes());
        reply[24..36].copy_from_slice(&nonce);
        reply[36] = 17;
        reply[40..42].copy_from_slice(&41641u16.to_be_bytes());
        reply[42..44].copy_from_slice(&50000u16.to_be_bytes());
        reply[54] = 0xff;
        reply[55] = 0xff;
        reply[56..60].copy_from_slice(&[203, 0, 113, 9]);
        assert_eq!(
            parse_pcp_map(&reply, nonce, 41641),
            Some(("203.0.113.9:50000".parse().unwrap(), 7200))
        );
        assert_eq!(
            parse_pcp_map(&reply, [8u8; 12], 41641),
            None,
            "随机数对不上"
        );
        let mut refused = reply;
        refused[3] = 8; // NO_RESOURCES
        assert_eq!(parse_pcp_map(&refused, nonce, 41641), None);
    }

    #[test]
    fn the_gateway_is_guessed_from_the_lan() {
        assert_eq!(
            guess_gateway(Ipv4Addr::new(192, 168, 1, 127), 24),
            Some(Ipv4Addr::new(192, 168, 1, 1))
        );
        assert_eq!(
            guess_gateway(Ipv4Addr::new(10, 3, 7, 9), 16),
            Some(Ipv4Addr::new(10, 3, 0, 1))
        );
        assert_eq!(guess_gateway(Ipv4Addr::new(10, 0, 0, 1), 32), None);
    }
}
