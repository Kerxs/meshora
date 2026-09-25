//! TCP 上的 Noise IK 连接：控制通道和中继通道都用它。
//!
//! 线上格式：
//!
//! ```text
//! 发起方 → 前导：  "MSHR" | 版本 (1) | 通道 (1)
//! 发起方 → 帧：    Noise IK 第一条握手消息
//! 响应方 → 帧：    Noise IK 第二条握手消息
//! 此后双向：       帧 = 长度 (2 字节，大端) | Noise 密文
//! ```
//!
//! 前导整个作为 Noise 的 prologue：中间人改了通道类型，握手就会失败。
//! 这也是域分离 —— 同一把节点密钥用在控制通道和中继通道上，两边的握手互不相通。
//!
//! 发起方必须事先知道响应方的公钥（IK 的 K）。响应方从握手里得知发起方是谁，
//! 由上层决定认不认。**上层在收到第一条加密消息之前不能做任何有副作用的事**：
//! IK 的第一个握手包可以被原样重放（威胁模型 R1）。

use std::fmt;
use std::io;
use std::sync::Arc;

use meshora_types::{CONTROL_MAGIC, NodeKey, NodeSecret};
use snow::StatelessTransportState;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};

const PATTERN: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";
/// 连接格式的版本。
pub const VERSION: u8 = 1;
/// Noise 消息的上限。
const MAX_FRAME: usize = u16::MAX as usize;
const TAG_LEN: usize = 16;
/// 一条消息的明文上限。
pub const MAX_MESSAGE: usize = MAX_FRAME - TAG_LEN;

/// 连接的用途，写在前导里。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    /// 节点 ↔ 协调服务。
    Control,
    /// 节点 ↔ 中继。
    Relay,
}

impl Channel {
    fn byte(self) -> u8 {
        match self {
            Self::Control => b'C',
            Self::Relay => b'R',
        }
    }

    fn prelude(self) -> [u8; 6] {
        let mut prelude = [0u8; 6];
        prelude[..4].copy_from_slice(&CONTROL_MAGIC);
        prelude[4] = VERSION;
        prelude[5] = self.byte();
        prelude
    }
}

fn builder<'a>(prelude: &'a [u8; 6], local: &'a NodeSecret) -> snow::Builder<'a> {
    snow::Builder::new(PATTERN.parse().expect("Noise 模式名是常量"))
        .local_private_key(local.as_bytes())
        .and_then(|b| b.prologue(prelude))
        .expect("参数都是定长的密钥和常量，不会失败")
}

/// 一条握手完成的加密连接。
pub struct NoiseStream<S> {
    stream: S,
    transport: Arc<StatelessTransportState>,
    remote: NodeKey,
    send_nonce: u64,
    recv_nonce: u64,
}

impl<S: AsyncRead + AsyncWrite + Unpin> NoiseStream<S> {
    /// 发起连接：发前导，用自己的私钥和事先知道的对方公钥做 IK 握手。
    ///
    /// 对方公钥不对，对方就解不开第一条握手消息，这里会因为连接被关闭而失败。
    pub async fn connect(
        mut stream: S,
        channel: Channel,
        local: &NodeSecret,
        remote: &NodeKey,
    ) -> Result<Self, NoiseError> {
        let prelude = channel.prelude();
        let mut handshake = builder(&prelude, local)
            .remote_public_key(remote.as_bytes())
            .and_then(|b| b.build_initiator())?;

        stream.write_all(&prelude).await?;
        let mut buf = vec![0u8; MAX_FRAME];
        let len = handshake.write_message(&[], &mut buf)?;
        write_frame(&mut stream, &buf[..len]).await?;

        let frame = read_frame(&mut stream).await?;
        handshake.read_message(&frame, &mut buf)?;

        Ok(Self {
            stream,
            transport: Arc::new(handshake.into_stateless_transport_mode()?),
            remote: *remote,
            send_nonce: 0,
            recv_nonce: 0,
        })
    }

    /// 接受连接：检查前导，完成 IK 握手。对方的身份由握手证明，认不认由调用方决定。
    ///
    /// 握手不限时，调用方要自己套一层超时，别让慢吞吞的连接一直占着。
    pub async fn accept(
        mut stream: S,
        channel: Channel,
        local: &NodeSecret,
    ) -> Result<Self, NoiseError> {
        let expected = channel.prelude();
        let mut prelude = [0u8; 6];
        stream.read_exact(&mut prelude).await?;
        if prelude != expected {
            return Err(NoiseError::BadPrelude);
        }

        let mut handshake = builder(&expected, local).build_responder()?;
        let frame = read_frame(&mut stream).await?;
        let mut buf = vec![0u8; MAX_FRAME];
        handshake.read_message(&frame, &mut buf)?;
        let remote = handshake
            .get_remote_static()
            .and_then(|key| <[u8; 32]>::try_from(key).ok())
            .map(NodeKey::from_bytes)
            .ok_or(NoiseError::Handshake)?;

        let len = handshake.write_message(&[], &mut buf)?;
        write_frame(&mut stream, &buf[..len]).await?;

        Ok(Self {
            stream,
            transport: Arc::new(handshake.into_stateless_transport_mode()?),
            remote,
            send_nonce: 0,
            recv_nonce: 0,
        })
    }

    /// 对方的身份（握手已经证明过）。
    pub fn remote(&self) -> NodeKey {
        self.remote
    }

    /// 发一条消息。
    pub async fn send(&mut self, message: &[u8]) -> Result<(), NoiseError> {
        send(
            &self.transport,
            &mut self.send_nonce,
            &mut self.stream,
            message,
        )
        .await
    }

    /// 收一条消息。对方正常关闭连接时返回 [`NoiseError::Closed`]。
    pub async fn recv(&mut self) -> Result<Vec<u8>, NoiseError> {
        recv(&self.transport, &mut self.recv_nonce, &mut self.stream).await
    }

    /// 拆成读、写两半，可以在不同的任务里同时收发。
    pub fn into_split(self) -> (NoiseReader<S>, NoiseWriter<S>) {
        let (read, write) = tokio::io::split(self.stream);
        (
            NoiseReader {
                stream: read,
                transport: Arc::clone(&self.transport),
                nonce: self.recv_nonce,
            },
            NoiseWriter {
                stream: write,
                transport: self.transport,
                nonce: self.send_nonce,
            },
        )
    }
}

/// 连接的读半边。
pub struct NoiseReader<S> {
    stream: ReadHalf<S>,
    transport: Arc<StatelessTransportState>,
    nonce: u64,
}

impl<S: AsyncRead + AsyncWrite> NoiseReader<S> {
    /// 收一条消息。
    pub async fn recv(&mut self) -> Result<Vec<u8>, NoiseError> {
        recv(&self.transport, &mut self.nonce, &mut self.stream).await
    }
}

/// 连接的写半边。
pub struct NoiseWriter<S> {
    stream: WriteHalf<S>,
    transport: Arc<StatelessTransportState>,
    nonce: u64,
}

impl<S: AsyncRead + AsyncWrite> NoiseWriter<S> {
    /// 发一条消息。
    pub async fn send(&mut self, message: &[u8]) -> Result<(), NoiseError> {
        send(&self.transport, &mut self.nonce, &mut self.stream, message).await
    }
}

async fn send(
    transport: &StatelessTransportState,
    nonce: &mut u64,
    stream: &mut (impl AsyncWrite + Unpin),
    message: &[u8],
) -> Result<(), NoiseError> {
    if message.len() > MAX_MESSAGE {
        return Err(NoiseError::TooLarge(message.len()));
    }
    let mut buf = vec![0u8; message.len() + TAG_LEN];
    let len = transport.write_message(*nonce, message, &mut buf)?;
    *nonce += 1;
    write_frame(stream, &buf[..len]).await
}

async fn recv(
    transport: &StatelessTransportState,
    nonce: &mut u64,
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<Vec<u8>, NoiseError> {
    let frame = read_frame(stream).await?;
    let mut buf = vec![0u8; frame.len()];
    let len = transport.read_message(*nonce, &frame, &mut buf)?;
    *nonce += 1;
    buf.truncate(len);
    Ok(buf)
}

async fn write_frame(
    stream: &mut (impl AsyncWrite + Unpin),
    frame: &[u8],
) -> Result<(), NoiseError> {
    let len = u16::try_from(frame.len()).map_err(|_| NoiseError::TooLarge(frame.len()))?;
    let mut out = Vec::with_capacity(2 + frame.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(frame);
    stream.write_all(&out).await?;
    Ok(())
}

async fn read_frame(stream: &mut (impl AsyncRead + Unpin)) -> Result<Vec<u8>, NoiseError> {
    let mut len = [0u8; 2];
    match stream.read_exact(&mut len).await {
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => return Err(NoiseError::Closed),
        Err(err) => return Err(err.into()),
    }
    let mut frame = vec![0u8; usize::from(u16::from_be_bytes(len))];
    stream.read_exact(&mut frame).await?;
    Ok(frame)
}

/// 连接出错的原因。
#[derive(Debug)]
pub enum NoiseError {
    /// 底层 I/O 出错。
    Io(io::Error),
    /// Noise 握手或解密失败：对方公钥不对、数据被篡改，或者对方不是它声称的那个人。
    Noise(snow::Error),
    /// 握手没有得到对方的静态公钥。
    Handshake,
    /// 前导不对：不是 Meshora、版本不对，或者通道类型不对。
    BadPrelude,
    /// 消息太长，一个帧装不下。
    TooLarge(usize),
    /// 对方关闭了连接。
    Closed,
}

impl From<io::Error> for NoiseError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<snow::Error> for NoiseError {
    fn from(err: snow::Error) -> Self {
        Self::Noise(err)
    }
}

impl fmt::Display for NoiseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "连接 I/O 出错：{err}"),
            Self::Noise(err) => write!(f, "Noise 握手或解密失败：{err}"),
            Self::Handshake => f.write_str("握手没有得到对方的公钥"),
            Self::BadPrelude => f.write_str("连接前导不对"),
            Self::TooLarge(len) => write!(f, "消息 {len} 字节，超过单帧上限"),
            Self::Closed => f.write_str("对方关闭了连接"),
        }
    }
}

impl std::error::Error for NoiseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Noise(err) => Some(err),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{DuplexStream, duplex};

    use super::*;

    async fn pair(
        channel_client: Channel,
        channel_server: Channel,
        client: &NodeSecret,
        server: &NodeSecret,
    ) -> (
        Result<NoiseStream<DuplexStream>, NoiseError>,
        Result<NoiseStream<DuplexStream>, NoiseError>,
    ) {
        let (a, b) = duplex(1 << 16);
        let server_key = server.public_key();
        // 一端失败时它持有的那半条管道随之释放，另一端读到 EOF，不会一直等下去
        tokio::join!(
            NoiseStream::connect(a, channel_client, client, &server_key),
            NoiseStream::accept(b, channel_server, server),
        )
    }

    #[tokio::test]
    async fn handshake_authenticates_both_sides_and_messages_flow() {
        let client = NodeSecret::generate();
        let server = NodeSecret::generate();
        let (c, s) = pair(Channel::Control, Channel::Control, &client, &server).await;
        let mut c = c.unwrap();
        let mut s = s.unwrap();
        assert_eq!(c.remote(), server.public_key());
        assert_eq!(s.remote(), client.public_key());

        c.send(b"hello").await.unwrap();
        assert_eq!(s.recv().await.unwrap(), b"hello");
        s.send(b"welcome").await.unwrap();
        s.send(b"").await.unwrap();
        assert_eq!(c.recv().await.unwrap(), b"welcome");
        assert_eq!(c.recv().await.unwrap(), b"");
    }

    #[tokio::test]
    async fn split_halves_work_concurrently() {
        let client = NodeSecret::generate();
        let server = NodeSecret::generate();
        let (c, s) = pair(Channel::Relay, Channel::Relay, &client, &server).await;
        let (mut c_read, mut c_write) = c.unwrap().into_split();
        let (mut s_read, mut s_write) = s.unwrap().into_split();

        let echo = tokio::spawn(async move {
            for _ in 0..100 {
                let message = s_read.recv().await.unwrap();
                s_write.send(&message).await.unwrap();
            }
        });
        let sender = tokio::spawn(async move {
            for i in 0..100u32 {
                c_write.send(&i.to_be_bytes()).await.unwrap();
            }
        });
        for i in 0..100u32 {
            assert_eq!(c_read.recv().await.unwrap(), i.to_be_bytes());
        }
        sender.await.unwrap();
        echo.await.unwrap();
    }

    #[tokio::test]
    async fn wrong_server_key_fails_the_handshake() {
        let client = NodeSecret::generate();
        let server = NodeSecret::generate();
        let impostor_key = NodeSecret::generate().public_key();
        let (a, b) = duplex(1 << 16);
        let server_side = tokio::spawn(async move {
            // 解不开第一条握手消息，失败并丢掉连接
            NoiseStream::accept(b, Channel::Control, &server)
                .await
                .err()
        });
        let client_result = NoiseStream::connect(a, Channel::Control, &client, &impostor_key).await;
        assert!(matches!(
            server_side.await.unwrap(),
            Some(NoiseError::Noise(_))
        ));
        assert!(client_result.is_err());
    }

    #[tokio::test]
    async fn channel_mismatch_is_rejected() {
        let client = NodeSecret::generate();
        let server = NodeSecret::generate();
        let (c, s) = pair(Channel::Relay, Channel::Control, &client, &server).await;
        assert!(matches!(s, Err(NoiseError::BadPrelude)));
        assert!(c.is_err());
    }

    #[tokio::test]
    async fn tampered_frame_is_rejected() {
        // 客户端这一侧照着模块文档里的线上格式手工实现，顺带验证文档写的格式没错
        let client = NodeSecret::generate();
        let server = NodeSecret::generate();
        let server_key = server.public_key();
        let (mut a, b) = duplex(1 << 16);
        let server_task = tokio::spawn(async move {
            let mut s = NoiseStream::accept(b, Channel::Control, &server)
                .await
                .unwrap();
            let first = s.recv().await.unwrap();
            (first, s.recv().await)
        });

        let prelude = Channel::Control.prelude();
        let mut handshake = builder(&prelude, &client)
            .remote_public_key(server_key.as_bytes())
            .and_then(|b| b.build_initiator())
            .unwrap();
        let mut buf = vec![0u8; MAX_FRAME];
        a.write_all(&prelude).await.unwrap();
        let len = handshake.write_message(&[], &mut buf).unwrap();
        write_frame(&mut a, &buf[..len]).await.unwrap();
        let frame = read_frame(&mut a).await.unwrap();
        handshake.read_message(&frame, &mut buf).unwrap();
        let transport = handshake.into_stateless_transport_mode().unwrap();

        let len = transport.write_message(0, b"ok", &mut buf).unwrap();
        write_frame(&mut a, &buf[..len]).await.unwrap();
        // 第二条改掉一个字节
        let len = transport.write_message(1, b"evil", &mut buf).unwrap();
        buf[0] ^= 0x01;
        write_frame(&mut a, &buf[..len]).await.unwrap();

        let (first, second) = server_task.await.unwrap();
        assert_eq!(first, b"ok");
        assert!(matches!(second, Err(NoiseError::Noise(_))));
    }

    #[tokio::test]
    async fn oversized_message_is_refused_before_sending() {
        let client = NodeSecret::generate();
        let server = NodeSecret::generate();
        let (c, s) = pair(Channel::Control, Channel::Control, &client, &server).await;
        let (mut c, mut s) = (c.unwrap(), s.unwrap());
        let big = vec![0u8; MAX_MESSAGE + 1];
        assert!(matches!(c.send(&big).await, Err(NoiseError::TooLarge(_))));

        // 正好到上限的能完整送到。两端同时收发：一整帧比管道的缓冲还大
        let max = vec![0xAB; MAX_MESSAGE];
        let (sent, received) = tokio::join!(c.send(&max), s.recv());
        sent.unwrap();
        assert_eq!(received.unwrap(), max);
    }

    #[tokio::test]
    async fn clean_close_is_reported_as_closed() {
        let client = NodeSecret::generate();
        let server = NodeSecret::generate();
        let (c, s) = pair(Channel::Control, Channel::Control, &client, &server).await;
        drop(c.unwrap());
        assert!(matches!(s.unwrap().recv().await, Err(NoiseError::Closed)));
    }
}
