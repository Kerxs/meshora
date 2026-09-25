//! 虚拟网卡。
//!
//! 数据面只要两件事：读出系统要发进 overlay 的 IP 报文，把解密出来的 IP 报文写回系统。
//! 这个 crate 把各平台的虚拟网卡包装成同一个样子：[`Tun::open`] 创建并配置好网卡，
//! [`Tun::spawn`] 启动搬运任务，交出一对 channel（[`Pipes`]）—— 正好接到数据面的 `TunChannels` 上。
//!
//! | 平台 | 实现 | 状态 |
//! | --- | --- | --- |
//! | Linux | `/dev/net/tun` + ioctl | 实测过 |
//! | Windows | wintun | 只做过编译检查，还没在 Windows 上跑过 |
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
}
