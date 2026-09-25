//! 中继客户端：一个中继一条长连接（TCP + Noise IK），经它收发 WireGuard 报文。
//!
//! 连接由第一次用到这个中继的报文触发，之后一直保持；断了按退避重连。
//! 断开期间积压的报文在重连前清掉 —— 过时的 WireGuard 报文发过去也没用。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use meshora_proto::noise::{Channel, NoiseError, NoiseStream};
use meshora_proto::relay::{ClientFrame, ServerFrame};
use meshora_types::{NodeKey, NodeSecret};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tracing::{debug, info};

use super::{Link, Shared};

/// 连接和握手的时限。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 保活间隔。中继 90 秒收不到任何帧就断开。
const PING_INTERVAL: Duration = Duration::from_secs(10);
/// 多久收不到中继的任何帧，就认为连接已经死了，重连。中继对每个保活都回 Pong。
///
/// 不能指望 TCP 自己发现：比如本机换了网络，连接绑着的旧地址已经不在了，写进去的东西
/// 哪儿也到不了，却不报错 —— TCP 要十几分钟才放弃，这期间经中继的路全断
const SILENCE: Duration = Duration::from_secs(25);
/// 重连等待的上限。
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// 一个中继的连接任务：连上、收发、断了重连，直到数据面停下（发件箱关闭）。
pub(super) async fn run(
    shared: Arc<Shared>,
    relay: NodeKey,
    addr: SocketAddr,
    mut outbox: mpsc::Receiver<(NodeKey, Vec<u8>)>,
) {
    let mut backoff = Duration::from_secs(1);
    loop {
        match connect(&shared.secret, relay, addr).await {
            Ok(stream) => {
                info!(%relay, %addr, "连上中继");
                backoff = Duration::from_secs(1);
                if !session(&shared, relay, addr, stream, &mut outbox).await {
                    return;
                }
                debug!(%relay, "和中继的连接断了");
            }
            Err(err) => debug!(%relay, %addr, %err, "连中继失败"),
        }
        while outbox.try_recv().is_ok() {}
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

async fn connect(
    secret: &NodeSecret,
    relay: NodeKey,
    addr: SocketAddr,
) -> Result<NoiseStream<TcpStream>, NoiseError> {
    let timed_out = |_| NoiseError::Io(std::io::ErrorKind::TimedOut.into());
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(timed_out)??;
    let _ = tcp.set_nodelay(true);
    let mut stream = tokio::time::timeout(
        CONNECT_TIMEOUT,
        NoiseStream::connect(tcp, Channel::Relay, secret, &relay),
    )
    .await
    .map_err(timed_out)??;
    stream.send(&ClientFrame::Hello.encode()).await?;
    Ok(stream)
}

/// 跑一条连接。返回 `false` 表示数据面已经停下，不用再重连
async fn session(
    shared: &Arc<Shared>,
    relay: NodeKey,
    addr: SocketAddr,
    stream: NoiseStream<TcpStream>,
    outbox: &mut mpsc::Receiver<(NodeKey, Vec<u8>)>,
) -> bool {
    let (mut reader, mut writer) = stream.into_split();
    // Noise 帧的读不能在 select 里被中途取消，放进单独的任务
    let (inbox_tx, mut inbox) = mpsc::channel(super::RELAY_QUEUE);
    let reader_task = tokio::spawn(async move {
        while let Ok(bytes) = reader.recv().await {
            if inbox_tx.send(bytes).await.is_err() {
                return;
            }
        }
    });
    let mut ping = tokio::time::interval(PING_INTERVAL);
    ping.tick().await;
    let mut last_heard = Instant::now();

    let keep_going = loop {
        tokio::select! {
            out = outbox.recv() => match out {
                Some((peer, datagram)) => {
                    let frame = ClientFrame::Send { dst: peer, datagram };
                    if writer.send(&frame.encode()).await.is_err() {
                        break true;
                    }
                }
                None => break false,
            },
            incoming = inbox.recv() => {
                let Some(bytes) = incoming else { break true };
                last_heard = Instant::now();
                match ServerFrame::decode(&bytes) {
                    Ok(ServerFrame::Recv { src, datagram }) => {
                        let link = Link::Relay { relay, addr, peer: src };
                        let actions = shared.engine().inbound(&datagram, link, Instant::now());
                        shared.act(actions).await;
                    }
                    Ok(ServerFrame::Pong) => {}
                    Err(err) => {
                        debug!(%relay, %err, "中继发来的帧解不开，断开");
                        break true;
                    }
                }
            }
            _ = ping.tick() => {
                if last_heard.elapsed() > SILENCE {
                    info!(%relay, silence = ?SILENCE, "中继太久没有回音，重连");
                    break true;
                }
                if writer.send(&ClientFrame::Ping.encode()).await.is_err() {
                    break true;
                }
            }
        }
    };
    reader_task.abort();
    keep_going
}
