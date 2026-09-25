//! Windows：wintun。
//!
//! **还没在 Windows 上跑过。** 这一版只保证编译通过，照着 wintun crate 的接口写成；
//! 地址和 MTU 由 wintun crate 调 netsh 设置。

use std::io;
use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::{Pipes, TunConfig, netmask};

/// wintun 环形缓冲区的容量，必须是 2 的幂。取 wireguard-go 用的 8 MiB
const RING_CAPACITY: u32 = 0x80_0000;

pub(crate) struct Device {
    session: Arc<wintun::Session>,
}

pub(crate) fn open(config: &TunConfig) -> io::Result<(Device, String)> {
    let dll = std::env::current_exe()?.with_file_name("wintun.dll");
    // SAFETY: 加载 DLL 会执行它的初始化代码。只从 meshorad.exe 所在的目录加载，
    // 不走系统搜索路径 —— 那样当前目录或 PATH 里任何一个叫 wintun.dll 的文件都会被加载
    // （DLL 劫持）。这个文件可不可信，取决于安装目录的写权限。
    let wintun = unsafe { wintun::load_from_path(&dll) }
        .map_err(|err| io::Error::other(format!("加载 {} 失败：{err}", dll.display())))?;
    let adapter = wintun::Adapter::create(&wintun, &config.name, "Meshora", None).map_err(other)?;
    adapter
        .set_network_addresses_tuple(
            config.address.into(),
            netmask(config.prefix_len).into(),
            None,
        )
        .map_err(other)?;
    adapter.set_mtu(config.mtu.into()).map_err(other)?;
    let name = adapter.get_name().map_err(other)?;
    let session = Arc::new(adapter.start_session(RING_CAPACITY).map_err(other)?);
    Ok((Device { session }, name))
}

pub(crate) fn spawn(device: Device, capacity: usize) -> io::Result<Pipes> {
    let (read_tx, read_rx) = mpsc::channel(capacity);
    let (write_tx, mut write_rx) = mpsc::channel::<Vec<u8>>(capacity);
    let session = device.session;

    // wintun 的读是阻塞的，放进单独的线程
    let reader = Arc::clone(&session);
    std::thread::Builder::new()
        .name("meshora-tun-read".into())
        .spawn(move || {
            loop {
                match reader.receive_blocking() {
                    Ok(packet) => {
                        if read_tx.blocking_send(packet.bytes().to_vec()).is_err() {
                            return;
                        }
                    }
                    Err(err) => {
                        debug!(%err, "wintun 会话结束");
                        return;
                    }
                }
            }
        })?;

    tokio::spawn(async move {
        while let Some(packet) = write_rx.recv().await {
            let Ok(len) = u16::try_from(packet.len()) else {
                continue;
            };
            match session.allocate_send_packet(len) {
                Ok(mut out) => {
                    out.bytes_mut().copy_from_slice(&packet);
                    session.send_packet(out);
                }
                Err(err) => debug!(%err, "wintun 发送缓冲区满，丢弃一个报文"),
            }
        }
        // 写的一方关了：结束会话，让阻塞在读上的线程退出
        if let Err(err) = session.shutdown() {
            warn!(%err, "关闭 wintun 会话失败");
        }
    });
    Ok(Pipes {
        from_tun: read_rx,
        to_tun: write_tx,
    })
}

fn other(err: wintun::Error) -> io::Error {
    io::Error::other(err.to_string())
}
