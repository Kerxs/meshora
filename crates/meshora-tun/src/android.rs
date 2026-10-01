//! 安卓：网卡由系统的 VpnService 建（地址、路由、MTU 都在 Java 那边的 `VpnService.Builder` 上设），
//! 这里只接过它交出来的文件描述符。App 没有 root，自己建不了网卡，所以 [`crate::Tun::open`] 在安卓上总是失败。

use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};

use crate::{Pipes, TunConfig};

pub(crate) struct Device {
    file: File,
}

pub(crate) fn open(_config: &TunConfig) -> io::Result<(Device, String)> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "安卓上网卡由 VpnService 建：用 Tun::from_fd",
    ))
}

pub(crate) fn from_fd(fd: OwnedFd) -> io::Result<Device> {
    let raw = fd.as_raw_fd();
    // SAFETY: raw 是我们持有的、打开着的描述符；F_GETFL / F_SETFL 只读写它的状态标志
    let flags = unsafe { libc::fcntl(raw, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: 同上
    if unsafe { libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Device {
        file: File::from(fd),
    })
}

pub(crate) fn spawn(device: Device, capacity: usize) -> io::Result<Pipes> {
    crate::fd::spawn(device.file, capacity)
}
