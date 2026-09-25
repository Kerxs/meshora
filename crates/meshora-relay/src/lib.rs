//! 中继服务：直连打不通时，在节点之间转发 WireGuard 报文。
//!
//! 它不需要被信任：转发的是已经加密的报文，读不到也改不了内容。它能看到元数据 ——
//! 谁在和谁通信、什么时候、多大流量，这一点[威胁模型](https://kerxs.github.io/meshora/guide/threat-model#恶意中继)
//! 里写明了。在意的话就自建中继。
//!
//! - 节点用自己的节点密钥握手，中继确切知道每条连接属于谁（R6）
//! - 只服务名单里的节点：转发的带宽是实打实要付钱的
//! - 收到第一帧（Hello）之前不登记连接（R1）
//! - 转发像 UDP 一样不保证送达：目标不在线、或者它的队列满了，报文就丢掉

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use meshora_proto::noise::{Channel, NoiseStream};
use meshora_proto::relay::{ClientFrame, ServerFrame};
use meshora_types::{NodeKey, NodeSecret};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// 握手和第一帧的时限。
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// 第一帧（Hello，一个字节）的长度上限：还没说上一句话的连接，不按它自称的长度分配内存。
const FIRST_MESSAGE_MAX: usize = 64;
/// 接受连接出错（比如文件描述符用完）之后，等多久再接。
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);
/// 多久没收到节点的任何帧就断开。节点每 30 秒发一次保活。
const IDLE_TIMEOUT: Duration = Duration::from_secs(90);
/// 发往一个节点的帧队列。满了就丢，和 UDP 一样。
const QUEUE: usize = 256;

/// 中继服务的配置。
pub struct Config {
    /// 中继自己的身份。节点事先知道它的公钥（协调服务在 Welcome 里告诉节点）。
    pub secret: NodeSecret,
    /// 允许使用这个中继的节点。
    pub nodes: Vec<NodeKey>,
}

struct Client {
    id: u64,
    tx: mpsc::Sender<ServerFrame>,
}

struct Shared {
    secret: NodeSecret,
    nodes: Vec<NodeKey>,
    clients: Mutex<HashMap<NodeKey, Client>>,
    next_id: AtomicU64,
}

impl Shared {
    fn clients(&self) -> MutexGuard<'_, HashMap<NodeKey, Client>> {
        self.clients.lock().expect("中继的状态在持锁时 panic 过")
    }
}

/// 运行中继，直到监听的 socket 出错。
pub async fn serve(config: Config, listener: TcpListener) -> io::Result<()> {
    let shared = Arc::new(Shared {
        secret: config.secret,
        nodes: config.nodes,
        clients: Mutex::new(HashMap::new()),
        next_id: AtomicU64::new(0),
    });
    info!(key = %shared.secret.public_key(), "中继启动");
    loop {
        match listener.accept().await {
            Ok((tcp, from)) => {
                tokio::spawn(handle(Arc::clone(&shared), tcp, from));
            }
            // 这类错误是暂时的（文件描述符用完之类），不能因此退出 ——
            // 否则谁都能靠开一大堆连接把中继打挂。稍等再接，免得空转
            Err(err) => {
                warn!(%err, "接受连接失败");
                tokio::time::sleep(ACCEPT_BACKOFF).await;
            }
        }
    }
}

async fn handle(shared: Arc<Shared>, tcp: TcpStream, from: SocketAddr) {
    let _ = tcp.set_nodelay(true);
    let stream = match tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        NoiseStream::accept(tcp, Channel::Relay, &shared.secret),
    )
    .await
    {
        Ok(Ok(stream)) => stream,
        _ => {
            debug!(%from, "握手失败或超时");
            return;
        }
    };
    let key = stream.remote();
    if !shared.nodes.contains(&key) {
        debug!(%from, node = %key, "不在名单里，断开");
        return;
    }
    let (mut reader, mut writer) = stream.into_split();

    // R1：第一帧到了才登记，被重放的握手包走不到这一步
    let hello =
        match tokio::time::timeout(HANDSHAKE_TIMEOUT, reader.recv_at_most(FIRST_MESSAGE_MAX)).await
        {
            Ok(Ok(bytes)) => ClientFrame::decode(&bytes),
            _ => return,
        };
    if hello != Ok(ClientFrame::Hello) {
        return;
    }

    let (tx, mut rx) = mpsc::channel(QUEUE);
    let id = shared.next_id.fetch_add(1, Ordering::Relaxed);
    shared.clients().insert(key, Client { id, tx: tx.clone() });
    debug!(%from, node = %key, "节点连上中继");

    let writer_task = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            if writer.send(&frame.encode()).await.is_err() {
                break;
            }
        }
    });

    loop {
        let bytes = match tokio::time::timeout(IDLE_TIMEOUT, reader.recv()).await {
            Ok(Ok(bytes)) => bytes,
            _ => break,
        };
        match ClientFrame::decode(&bytes) {
            Ok(ClientFrame::Send { dst, datagram }) => {
                let clients = shared.clients();
                if let Some(target) = clients.get(&dst) {
                    // 满了就丢：一个读得慢的节点不能拖住别人
                    let _ = target.tx.try_send(ServerFrame::Recv { src: key, datagram });
                }
            }
            Ok(ClientFrame::Ping) => {
                let _ = tx.try_send(ServerFrame::Pong);
            }
            Ok(ClientFrame::Hello) => {}
            Err(err) => {
                debug!(node = %key, %err, "帧解不开，断开");
                break;
            }
        }
    }

    {
        let mut clients = shared.clients();
        if clients.get(&key).is_some_and(|client| client.id == id) {
            clients.remove(&key);
        }
    }
    writer_task.abort();
    debug!(node = %key, "节点离开中继");
}

#[cfg(test)]
mod tests {
    use meshora_proto::noise::{NoiseReader, NoiseWriter};

    use super::*;

    const WAIT: Duration = Duration::from_secs(5);

    type Conn = (NoiseReader<TcpStream>, NoiseWriter<TcpStream>);

    async fn start(nodes: &[&NodeSecret]) -> (SocketAddr, NodeKey) {
        let secret = NodeSecret::generate();
        let key = secret.public_key();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let config = Config {
            secret,
            nodes: nodes.iter().map(|n| n.public_key()).collect(),
        };
        tokio::spawn(serve(config, listener));
        (addr, key)
    }

    async fn connect(addr: SocketAddr, relay: &NodeKey, secret: &NodeSecret, hello: bool) -> Conn {
        let tcp = TcpStream::connect(addr).await.unwrap();
        let mut conn = NoiseStream::connect(tcp, Channel::Relay, secret, relay)
            .await
            .unwrap()
            .into_split();
        if hello {
            conn.1.send(&ClientFrame::Hello.encode()).await.unwrap();
        }
        conn
    }

    async fn send(conn: &mut Conn, frame: ClientFrame) {
        conn.1.send(&frame.encode()).await.unwrap();
    }

    async fn recv(conn: &mut Conn) -> ServerFrame {
        let bytes = tokio::time::timeout(WAIT, conn.0.recv())
            .await
            .expect("等帧超时")
            .expect("连接断了");
        ServerFrame::decode(&bytes).unwrap()
    }

    /// 发一个 Ping 等 Pong：确认中继已经处理完这条连接之前的所有帧
    async fn sync(conn: &mut Conn) {
        send(conn, ClientFrame::Ping).await;
        assert_eq!(recv(conn).await, ServerFrame::Pong);
    }

    #[tokio::test]
    async fn forwards_between_members_with_the_authenticated_sender() {
        let a = NodeSecret::generate();
        let b = NodeSecret::generate();
        let (addr, relay) = start(&[&a, &b]).await;
        let mut conn_a = connect(addr, &relay, &a, true).await;
        let mut conn_b = connect(addr, &relay, &b, true).await;
        sync(&mut conn_b).await;

        let datagram = vec![4, 0, 0, 0, 1, 2, 3];
        send(
            &mut conn_a,
            ClientFrame::Send {
                dst: b.public_key(),
                datagram: datagram.clone(),
            },
        )
        .await;
        assert_eq!(
            recv(&mut conn_b).await,
            ServerFrame::Recv {
                src: a.public_key(),
                datagram
            }
        );
    }

    #[tokio::test]
    async fn strangers_are_cut_off() {
        let a = NodeSecret::generate();
        let stranger = NodeSecret::generate();
        let (addr, relay) = start(&[&a]).await;
        let mut conn = connect(addr, &relay, &stranger, true).await;
        let result = tokio::time::timeout(WAIT, conn.0.recv()).await.unwrap();
        assert!(result.is_err(), "陌生节点的连接应当被断开");
    }

    #[tokio::test]
    async fn nothing_is_delivered_before_hello() {
        let a = NodeSecret::generate();
        let b = NodeSecret::generate();
        let (addr, relay) = start(&[&a, &b]).await;
        let mut silent_b = connect(addr, &relay, &b, false).await;
        let mut conn_a = connect(addr, &relay, &a, true).await;
        send(
            &mut conn_a,
            ClientFrame::Send {
                dst: b.public_key(),
                datagram: vec![1],
            },
        )
        .await;
        sync(&mut conn_a).await;
        let nothing = tokio::time::timeout(Duration::from_millis(200), silent_b.0.recv()).await;
        assert!(nothing.is_err(), "没发 Hello 的连接不该收到任何东西");
    }

    #[tokio::test]
    async fn datagrams_for_offline_nodes_are_dropped() {
        let a = NodeSecret::generate();
        let b = NodeSecret::generate();
        let (addr, relay) = start(&[&a, &b]).await;
        let mut conn_a = connect(addr, &relay, &a, true).await;
        send(
            &mut conn_a,
            ClientFrame::Send {
                dst: b.public_key(),
                datagram: vec![1],
            },
        )
        .await;
        // 确认中继已经处理完这一帧（B 此时不在线），B 才上线：之前的报文不会补发
        sync(&mut conn_a).await;
        let mut conn_b = connect(addr, &relay, &b, true).await;
        sync(&mut conn_b).await;
        let nothing = tokio::time::timeout(Duration::from_millis(200), conn_b.0.recv()).await;
        assert!(nothing.is_err());
    }

    #[tokio::test]
    async fn reconnecting_replaces_the_old_connection() {
        let a = NodeSecret::generate();
        let b = NodeSecret::generate();
        let (addr, relay) = start(&[&a, &b]).await;
        let mut conn_a = connect(addr, &relay, &a, true).await;
        // 先确认旧连接登记好了，新连接才上来：两者的 Hello 谁先被处理不能靠运气
        let mut old_b = connect(addr, &relay, &b, true).await;
        sync(&mut old_b).await;
        let mut new_b = connect(addr, &relay, &b, true).await;
        sync(&mut new_b).await;
        // 旧连接断开时不能把新连接的登记也删掉
        drop(old_b);
        tokio::time::sleep(Duration::from_millis(100)).await;

        send(
            &mut conn_a,
            ClientFrame::Send {
                dst: b.public_key(),
                datagram: vec![7],
            },
        )
        .await;
        assert_eq!(
            recv(&mut new_b).await,
            ServerFrame::Recv {
                src: a.public_key(),
                datagram: vec![7]
            }
        );
    }
}
