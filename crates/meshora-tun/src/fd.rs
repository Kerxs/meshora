//! 读写一个文件描述符形式的虚拟网卡：Linux 的 `/dev/net/tun`、安卓 VpnService 交出来的都是这样。
//! 一次 `read` 就是一个 IP 报文，一次 `write` 写一个。

use std::fs::File;
use std::io::{self, Read, Write};
use std::sync::Arc;

use tokio::io::unix::AsyncFd;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::Pipes;

/// 启动两个方向的搬运。`file` 要已经是非阻塞的
pub(crate) fn spawn(file: File, capacity: usize) -> io::Result<Pipes> {
    let fd = Arc::new(AsyncFd::new(file)?);
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
