//! 虚拟网卡。
//!
//! 数据面只要两件事：读出系统要发进 overlay 的 IP 报文，把解密出来的 IP 报文写回系统。
//! 这个 crate 把各平台的虚拟网卡包装成同一个样子：[`Tun::open`] 创建并配置好网卡，
//! [`Tun::spawn`] 启动搬运任务，交出一对 channel（[`Pipes`]）—— 正好接到数据面的 `TunChannels` 上。
//!
//! | 平台 | 实现 | 状态 |
//! | --- | --- | --- |
//! | Linux | `/dev/net/tun` + ioctl | 实测过 |
//! | Windows | wintun | CI 的 Windows 虚拟机上实测过 |
//!
//! 平台相关的 unsafe 代码集中在这个 crate 里，每一处都写了 SAFETY。

use std::io;
use std::net::Ipv4Addr;

use tokio::sync::mpsc;

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
mod linux;
#[cfg(target_os = "linux")]
use linux as platform;

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows;
#[cfg(windows)]
use windows as platform;

/// 搬运 channel 的容量（报文个数）。
const CHANNEL_CAPACITY: usize = 256;

/// 虚拟网卡的配置。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TunConfig {
    /// 网卡名。Linux 上最长 15 个字节。
    pub name: String,
    /// 本机在 overlay 里的地址。
    pub address: Ipv4Addr,
    /// overlay 网段的前缀长度，比如 `100.64.0.0/10` 就是 10。系统据此把整个网段路由到这块网卡。
    pub prefix_len: u8,
    /// MTU。
    pub mtu: u16,
}

/// 一块打开、配置好的虚拟网卡。
pub struct Tun {
    name: String,
    device: platform::Device,
}

impl Tun {
    /// 创建网卡、设好地址和 MTU、启用。需要管理员权限（Linux 上是 `CAP_NET_ADMIN`）。
    pub fn open(config: &TunConfig) -> io::Result<Self> {
        if config.prefix_len > 32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "IPv4 前缀长度不能超过 32",
            ));
        }
        let (device, name) = platform::open(config)?;
        Ok(Self { name, device })
    }

    /// 系统里这块网卡实际的名字。
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 启动读、写两个方向的搬运。
    ///
    /// 两个 channel 都丢掉之后，搬运停止、网卡关闭。必须在 tokio 运行时里调用。
    pub fn spawn(self) -> io::Result<Pipes> {
        platform::spawn(self.device, CHANNEL_CAPACITY)
    }
}

/// 网卡两个方向的 channel。
pub struct Pipes {
    /// 从网卡读到的 IP 报文：系统要发进 overlay 的。
    pub from_tun: mpsc::Receiver<Vec<u8>>,
    /// 要写进网卡的 IP 报文：从 overlay 收到、交给系统的。
    pub to_tun: mpsc::Sender<Vec<u8>>,
}

/// 前缀长度换成子网掩码：10 → 255.192.0.0
fn netmask(prefix_len: u8) -> Ipv4Addr {
    let bits = u32::MAX
        .checked_shl(32 - u32::from(prefix_len))
        .unwrap_or(0);
    Ipv4Addr::from(bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn netmask_from_prefix_length() {
        assert_eq!(netmask(0), Ipv4Addr::new(0, 0, 0, 0));
        assert_eq!(netmask(10), Ipv4Addr::new(255, 192, 0, 0));
        assert_eq!(netmask(24), Ipv4Addr::new(255, 255, 255, 0));
        assert_eq!(netmask(32), Ipv4Addr::new(255, 255, 255, 255));
    }

    #[test]
    fn rejects_prefix_longer_than_32() {
        let config = TunConfig {
            name: "meshora0".into(),
            address: Ipv4Addr::new(100, 64, 0, 1),
            prefix_len: 33,
            mtu: 1280,
        };
        let err = Tun::open(&config).err().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    /// 真的创建一块网卡，报文两个方向都走一遍。两个平台用同一个测试：
    ///
    /// - Linux 上要 `CAP_NET_ADMIN`
    /// - Windows 上要管理员权限，并且 wintun.dll 放在测试程序旁边（`target/debug/deps/`）
    ///
    /// 两个方向都用 UDP：Windows 防火墙默认挡进来的 ping，但放行对已发出的 UDP 的回应
    mod privileged {
        use std::net::{SocketAddr, SocketAddrV4};
        use std::time::Duration;

        use tokio::net::UdpSocket;

        use super::*;

        const WAIT: Duration = Duration::from_secs(10);
        /// 测试用网段：RFC 2544 保留给网络测试的 198.18.0.0/15
        const TUN_ADDR: Ipv4Addr = Ipv4Addr::new(198, 18, 77, 1);
        const PEER: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(198, 18, 77, 2), 9999);

        fn checksum(data: &[u8]) -> u16 {
            let mut sum: u32 = data
                .chunks(2)
                .map(|pair| u32::from(u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)])))
                .sum();
            while sum > 0xffff {
                sum = (sum & 0xffff) + (sum >> 16);
            }
            !(sum as u16)
        }

        /// 从 src 到 dst 的 IPv4 UDP 报文，IP 和 UDP 的校验和都算好 —— 系统会检查
        fn udp_packet(src: SocketAddrV4, dst: SocketAddrV4, payload: &[u8]) -> Vec<u8> {
            let udp_len = (8 + payload.len()) as u16;
            let mut udp = Vec::new();
            udp.extend_from_slice(&src.port().to_be_bytes());
            udp.extend_from_slice(&dst.port().to_be_bytes());
            udp.extend_from_slice(&udp_len.to_be_bytes());
            udp.extend_from_slice(&[0, 0]);
            udp.extend_from_slice(payload);

            // UDP 校验和覆盖一个"伪首部"：两端地址、协议号、UDP 长度
            let mut pseudo = Vec::new();
            pseudo.extend_from_slice(&src.ip().octets());
            pseudo.extend_from_slice(&dst.ip().octets());
            pseudo.extend_from_slice(&[0, 17]);
            pseudo.extend_from_slice(&udp_len.to_be_bytes());
            pseudo.extend_from_slice(&udp);
            // 算出来是 0 要写成 0xffff：UDP 校验和为 0 表示"没有校验和"
            let sum = match checksum(&pseudo) {
                0 => 0xffff,
                sum => sum,
            };
            udp[6..8].copy_from_slice(&sum.to_be_bytes());

            let mut packet = vec![0u8; 20];
            packet[0] = 0x45;
            packet[2..4].copy_from_slice(&(20 + udp_len).to_be_bytes());
            packet[8] = 64;
            packet[9] = 17;
            packet[12..16].copy_from_slice(&src.ip().octets());
            packet[16..20].copy_from_slice(&dst.ip().octets());
            let sum = checksum(&packet);
            packet[10..12].copy_from_slice(&sum.to_be_bytes());
            packet.extend_from_slice(&udp);
            packet
        }

        /// 从网卡读报文，直到出现发往 to 的 UDP 报文。新网卡上还会有 IPv6 邻居发现、
        /// Windows 的各种广播之类的报文，都跳过
        async fn next_udp_to(from_tun: &mut mpsc::Receiver<Vec<u8>>, to: SocketAddrV4) -> Vec<u8> {
            loop {
                let packet = from_tun.recv().await.expect("网卡 channel 关了");
                if packet.len() >= 28
                    && packet[0] == 0x45
                    && packet[9] == 17
                    && packet[16..20] == to.ip().octets()
                    && packet[22..24] == to.port().to_be_bytes()
                {
                    return packet;
                }
            }
        }

        /// 绑定到网卡的地址上。Windows 刚设好地址时还在做重复地址检测，要等一会儿才能绑
        async fn bind_when_ready(addr: SocketAddrV4) -> UdpSocket {
            tokio::time::timeout(WAIT, async {
                loop {
                    match UdpSocket::bind(addr).await {
                        Ok(socket) => return socket,
                        Err(err) if err.kind() == io::ErrorKind::AddrNotAvailable => {
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        }
                        Err(err) => panic!("绑定 {addr} 失败：{err}"),
                    }
                }
            })
            .await
            .expect("网卡的地址一直没法绑定")
        }

        #[tokio::test]
        #[ignore = "需要管理员权限，Windows 上还要 wintun.dll。用 cargo test -p meshora-tun -- --ignored 跑"]
        async fn packets_flow_both_ways() {
            let name = format!("mshtest{}", std::process::id() % 100_000);
            let tun = Tun::open(&TunConfig {
                name: name.clone(),
                address: TUN_ADDR,
                prefix_len: 24,
                mtu: 1280,
            })
            .unwrap();
            assert_eq!(tun.name(), name);
            let Pipes {
                mut from_tun,
                to_tun,
            } = tun.spawn().unwrap();

            let socket = bind_when_ready(SocketAddrV4::new(TUN_ADDR, 0)).await;
            let SocketAddr::V4(local) = socket.local_addr().unwrap() else {
                panic!("绑的是 IPv4 地址");
            };

            // 系统发往这个网段的报文，从网卡里读得出来。网卡刚起来时路由可能还没就绪，
            // 头几个报文可能丢，所以没读到就再发
            let packet = tokio::time::timeout(WAIT, async {
                loop {
                    socket.send_to(b"hello", PEER).await.unwrap();
                    let wait = Duration::from_millis(500);
                    if let Ok(packet) =
                        tokio::time::timeout(wait, next_udp_to(&mut from_tun, PEER)).await
                    {
                        return packet;
                    }
                }
            })
            .await
            .expect("等网卡上的报文超时");
            assert_eq!(packet[12..16], local.ip().octets());
            assert_eq!(packet[20..22], local.port().to_be_bytes());
            assert_eq!(&packet[28..], b"hello");

            // 写进网卡的报文，系统收得到：以对端的身份回一个 UDP 报文
            to_tun
                .send(udp_packet(PEER, local, b"meshora"))
                .await
                .unwrap();
            let mut buf = [0u8; 64];
            let (len, from) = tokio::time::timeout(WAIT, socket.recv_from(&mut buf))
                .await
                .expect("等回应超时")
                .unwrap();
            assert_eq!(from, SocketAddr::V4(PEER));
            assert_eq!(&buf[..len], b"meshora");
        }
    }
}
