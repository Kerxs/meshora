//! Linux：`/dev/net/tun`，地址和 MTU 用 ioctl 设置。
//!
//! 不依赖 `ip` 之类的外部命令。unsafe 只有 ioctl 本身和构造 C 结构体两处。

use std::ffi::c_short;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, UdpSocket};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::sync::Arc;

use tokio::io::unix::AsyncFd;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::{Pipes, TunConfig, netmask};

pub(crate) struct Device {
    file: File,
}

pub(crate) fn open(config: &TunConfig) -> io::Result<(Device, String)> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open("/dev/net/tun")?;

    // IFF_NO_PI：不要内核在每个报文前加 4 字节的协议头，读出来的就是 IP 报文
    let mut req = request(&config.name)?;
    req.ifr_ifru.ifru_flags = (libc::IFF_TUN | libc::IFF_NO_PI) as c_short;
    ioctl(file.as_raw_fd(), libc::TUNSETIFF as _, &mut req)?;
    let name = name_of(&req);

    // 设地址要对一个 AF_INET socket 做 ioctl，随便绑一个 UDP socket 就行
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    let fd = socket.as_raw_fd();

    let mut req = request(&name)?;
    req.ifr_ifru.ifru_addr = sockaddr(config.address);
    ioctl(fd, libc::SIOCSIFADDR as _, &mut req)?;

    let mut req = request(&name)?;
    req.ifr_ifru.ifru_netmask = sockaddr(netmask(config.prefix_len));
    ioctl(fd, libc::SIOCSIFNETMASK as _, &mut req)?;

    let mut req = request(&name)?;
    req.ifr_ifru.ifru_mtu = config.mtu.into();
    ioctl(fd, libc::SIOCSIFMTU as _, &mut req)?;

    let mut req = request(&name)?;
    ioctl(fd, libc::SIOCGIFFLAGS as _, &mut req)?;
    // SAFETY: SIOCGIFFLAGS 填写的正是 ifru_flags 这个成员
    let flags = unsafe { req.ifr_ifru.ifru_flags };
    req.ifr_ifru.ifru_flags = flags | (libc::IFF_UP | libc::IFF_RUNNING) as c_short;
    ioctl(fd, libc::SIOCSIFFLAGS as _, &mut req)?;

    Ok((Device { file }, name))
}

pub(crate) fn spawn(device: Device, capacity: usize) -> io::Result<Pipes> {
    let fd = Arc::new(AsyncFd::new(device.file)?);
    let (read_tx, read_rx) = mpsc::channel(capacity);
    let (write_tx, write_rx) = mpsc::channel(capacity);
    tokio::spawn(read_loop(Arc::clone(&fd), read_tx));
    tokio::spawn(write_loop(fd, write_rx));
    Ok(Pipes {
        from_tun: read_rx,
        to_tun: write_tx,
    })
}

async fn read_loop(fd: Arc<AsyncFd<File>>, tx: mpsc::Sender<Vec<u8>>) {
    let mut buf = vec![0u8; u16::MAX as usize];
    loop {
        let mut guard = tokio::select! {
            _ = tx.closed() => return,
            ready = fd.readable() => match ready {
                Ok(guard) => guard,
                Err(err) => {
                    warn!(%err, "等待虚拟网卡可读时出错");
                    return;
                }
            },
        };
        match guard.try_io(|inner| {
            let mut file: &File = inner.get_ref();
            file.read(&mut buf)
        }) {
            Ok(Ok(len)) => {
                if tx.send(buf[..len].to_vec()).await.is_err() {
                    return;
                }
            }
            Ok(Err(err)) => {
                warn!(%err, "读虚拟网卡出错");
                return;
            }
            // 被别人抢先读空了，接着等
            Err(_would_block) => continue,
        }
    }
}

async fn write_loop(fd: Arc<AsyncFd<File>>, mut rx: mpsc::Receiver<Vec<u8>>) {
    while let Some(packet) = rx.recv().await {
        loop {
            let Ok(mut guard) = fd.writable().await else {
                return;
            };
            match guard.try_io(|inner| {
                let mut file: &File = inner.get_ref();
                file.write(&packet)
            }) {
                Ok(Ok(_)) => break,
                Ok(Err(err)) => {
                    debug!(%err, "写虚拟网卡失败，丢弃一个报文");
                    break;
                }
                Err(_would_block) => continue,
            }
        }
    }
}

/// 一个填好网卡名的 ifreq
fn request(name: &str) -> io::Result<libc::ifreq> {
    if name.len() >= libc::IFNAMSIZ || name.as_bytes().contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("网卡名 {name:?} 太长（最多 15 个字节）或含有 NUL"),
        ));
    }
    // SAFETY: ifreq 是纯 C 结构体（名字数组加一个联合体），全零是合法的初始值
    let mut req: libc::ifreq = unsafe { std::mem::zeroed() };
    for (dst, src) in req.ifr_name.iter_mut().zip(name.as_bytes()) {
        *dst = *src as libc::c_char;
    }
    Ok(req)
}

fn name_of(req: &libc::ifreq) -> String {
    let bytes: Vec<u8> = req
        .ifr_name
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn sockaddr(addr: Ipv4Addr) -> libc::sockaddr {
    let sin = libc::sockaddr_in {
        sin_family: libc::AF_INET as libc::sa_family_t,
        sin_port: 0,
        sin_addr: libc::in_addr {
            s_addr: u32::from_ne_bytes(addr.octets()),
        },
        sin_zero: [0; 8],
    };
    // SAFETY: sockaddr_in 和 sockaddr 都是 16 字节的纯数据结构，
    // 内核按 sa_family 字段把它当 sockaddr_in 解读 —— C 里本来就是这么用的
    unsafe { std::mem::transmute::<libc::sockaddr_in, libc::sockaddr>(sin) }
}

fn ioctl(fd: RawFd, request: libc::Ioctl, req: &mut libc::ifreq) -> io::Result<()> {
    // SAFETY: 这里用到的请求（TUNSETIFF、SIOCSIF*、SIOCGIFFLAGS）读写的都是一个 ifreq，
    // req 在调用期间有效且可写
    let ret = unsafe { libc::ioctl(fd, request, req as *mut libc::ifreq) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::Tun;

    const WAIT: Duration = Duration::from_secs(5);
    /// 测试用网段：RFC 2544 保留给网络测试的 198.18.0.0/15
    const TUN_ADDR: [u8; 4] = [198, 18, 77, 1];
    const PEER_ADDR: [u8; 4] = [198, 18, 77, 2];

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

    /// 从 src 到 dst 的 ICMP echo request，校验和都算好 —— 内核会检查
    fn icmp_echo(src: [u8; 4], dst: [u8; 4]) -> Vec<u8> {
        let mut icmp = vec![8, 0, 0, 0, 0x12, 0x34, 0, 1];
        icmp.extend_from_slice(b"meshora");
        let sum = checksum(&icmp);
        icmp[2..4].copy_from_slice(&sum.to_be_bytes());

        let total = 20 + icmp.len();
        let mut packet = vec![0u8; 20];
        packet[0] = 0x45;
        packet[2..4].copy_from_slice(&(total as u16).to_be_bytes());
        packet[8] = 64;
        packet[9] = 1;
        packet[12..16].copy_from_slice(&src);
        packet[16..20].copy_from_slice(&dst);
        let sum = checksum(&packet);
        packet[10..12].copy_from_slice(&sum.to_be_bytes());
        packet.extend_from_slice(&icmp);
        packet
    }

    /// 从网卡读报文，直到出现满足条件的那个（新网卡上还会有 IPv6 邻居发现之类的报文）
    async fn wait_for(
        from_tun: &mut mpsc::Receiver<Vec<u8>>,
        wanted: impl Fn(&[u8]) -> bool,
    ) -> Vec<u8> {
        tokio::time::timeout(WAIT, async {
            loop {
                let packet = from_tun.recv().await.expect("网卡 channel 关了");
                if wanted(&packet) {
                    return packet;
                }
            }
        })
        .await
        .expect("等报文超时")
    }

    #[tokio::test]
    #[ignore = "需要 CAP_NET_ADMIN，用 cargo test -p meshora-tun -- --ignored 跑"]
    async fn packets_flow_both_ways() {
        let name = format!("mshtest{}", std::process::id() % 100_000);
        let tun = Tun::open(&TunConfig {
            name: name.clone(),
            address: Ipv4Addr::from(TUN_ADDR),
            prefix_len: 24,
            mtu: 1280,
        })
        .unwrap();
        assert_eq!(tun.name(), name);
        let Pipes {
            mut from_tun,
            to_tun,
        } = tun.spawn().unwrap();

        // 系统发往这个网段的报文，从网卡里读得出来
        let socket = UdpSocket::bind((Ipv4Addr::from(TUN_ADDR), 0)).unwrap();
        socket
            .send_to(b"hello", (Ipv4Addr::from(PEER_ADDR), 9999))
            .unwrap();
        let packet = wait_for(&mut from_tun, |p| {
            p.len() >= 28 && p[0] == 0x45 && p[9] == 17 && p[16..20] == PEER_ADDR
        })
        .await;
        assert_eq!(&packet[28..], b"hello");

        // 写进网卡的报文，系统收得到：ping 一下网卡自己的地址，回应从网卡出来
        to_tun.send(icmp_echo(PEER_ADDR, TUN_ADDR)).await.unwrap();
        let reply = wait_for(&mut from_tun, |p| {
            p.len() >= 28 && p[9] == 1 && p[20] == 0 && p[16..20] == PEER_ADDR
        })
        .await;
        assert_eq!(&reply[12..16], &TUN_ADDR);
        assert_eq!(&reply[28..], b"meshora");
    }

    #[test]
    fn rejects_overlong_interface_name() {
        let err = request("an-interface-name-too-long").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
