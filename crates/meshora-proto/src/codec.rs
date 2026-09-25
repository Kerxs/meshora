//! 二进制编解码：定长字段按网络字节序，变长字段带长度前缀。
//!
//! 手写而不用 serde：进来的都是不可信输入，每一次读都要查边界，
//! 格式要一眼能看清、能照着写出别的实现。

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use meshora_types::NodeKey;

/// 往缓冲区里写。
#[derive(Default)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    /// 空的写入器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 一个字节。
    pub fn u8(&mut self, value: u8) {
        self.buf.push(value);
    }

    /// 两个字节，大端。
    pub fn u16(&mut self, value: u16) {
        self.buf.extend_from_slice(&value.to_be_bytes());
    }

    /// 原样写入定长字节。
    pub fn bytes(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// 节点公钥：32 字节。
    pub fn key(&mut self, key: &NodeKey) {
        self.bytes(key.as_bytes());
    }

    /// IPv4 地址：4 字节。
    pub fn ipv4(&mut self, addr: Ipv4Addr) {
        self.bytes(&addr.octets());
    }

    /// socket 地址：地址族（4 或 6）+ 地址 + 端口。
    pub fn socket_addr(&mut self, addr: &SocketAddr) {
        match addr.ip() {
            IpAddr::V4(ip) => {
                self.u8(4);
                self.bytes(&ip.octets());
            }
            IpAddr::V6(ip) => {
                self.u8(6);
                self.bytes(&ip.octets());
            }
        }
        self.u16(addr.port());
    }

    /// 列表：两字节的个数，后面逐个写。
    ///
    /// # Panics
    ///
    /// 超过 65535 项时 panic —— 那是调用方的 bug，协议里没有这么长的列表。
    pub fn list<T>(&mut self, items: &[T], mut write: impl FnMut(&mut Self, &T)) {
        let count = u16::try_from(items.len()).expect("列表超过 65535 项");
        self.u16(count);
        for item in items {
            write(self, item);
        }
    }

    /// 字符串：两字节的长度，后面是 UTF-8。
    ///
    /// # Panics
    ///
    /// 超过 65535 字节时 panic。
    pub fn string(&mut self, value: &str) {
        let len = u16::try_from(value.len()).expect("字符串超过 65535 字节");
        self.u16(len);
        self.bytes(value.as_bytes());
    }

    /// 写完，交出字节。
    pub fn finish(self) -> Vec<u8> {
        self.buf
    }
}

/// 从缓冲区里读，每一步都查边界。
pub struct Reader<'a> {
    buf: &'a [u8],
}

impl<'a> Reader<'a> {
    /// 从头读 `buf`。
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf }
    }

    /// 定长的 N 个字节。
    pub fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let (head, rest) = self
            .buf
            .split_first_chunk::<N>()
            .ok_or(DecodeError::Truncated)?;
        self.buf = rest;
        Ok(*head)
    }

    /// 一个字节。
    pub fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.array::<1>()?[0])
    }

    /// 两个字节，大端。
    pub fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    /// 节点公钥。
    pub fn key(&mut self) -> Result<NodeKey, DecodeError> {
        Ok(NodeKey::from_bytes(self.array()?))
    }

    /// IPv4 地址。
    pub fn ipv4(&mut self) -> Result<Ipv4Addr, DecodeError> {
        Ok(Ipv4Addr::from(self.array::<4>()?))
    }

    /// socket 地址。
    pub fn socket_addr(&mut self) -> Result<SocketAddr, DecodeError> {
        let ip = match self.u8()? {
            4 => IpAddr::V4(Ipv4Addr::from(self.array::<4>()?)),
            6 => IpAddr::V6(Ipv6Addr::from(self.array::<16>()?)),
            _ => return Err(DecodeError::Invalid("未知的地址族")),
        };
        Ok(SocketAddr::new(ip, self.u16()?))
    }

    /// 列表。个数来自不可信输入，所以不按它预分配内存。
    pub fn list<T>(
        &mut self,
        mut read: impl FnMut(&mut Self) -> Result<T, DecodeError>,
    ) -> Result<Vec<T>, DecodeError> {
        let count = self.u16()?;
        let mut items = Vec::new();
        for _ in 0..count {
            items.push(read(self)?);
        }
        Ok(items)
    }

    /// 字符串。
    pub fn string(&mut self) -> Result<String, DecodeError> {
        let len = usize::from(self.u16()?);
        if self.buf.len() < len {
            return Err(DecodeError::Truncated);
        }
        let (head, rest) = self.buf.split_at(len);
        self.buf = rest;
        String::from_utf8(head.to_vec()).map_err(|_| DecodeError::Invalid("字符串不是 UTF-8"))
    }

    /// 剩下的全部字节。
    pub fn rest(&mut self) -> &'a [u8] {
        std::mem::take(&mut self.buf)
    }

    /// 读完了：后面不许有多余的字节。
    pub fn finish(self) -> Result<(), DecodeError> {
        if self.buf.is_empty() {
            Ok(())
        } else {
            Err(DecodeError::TrailingBytes)
        }
    }
}

/// 解码失败的原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// 数据不够长。
    Truncated,
    /// 读完了还有多余的字节。
    TrailingBytes,
    /// 不认识的消息类型。
    UnknownType(u8),
    /// 字段的值不合法。
    Invalid(&'static str),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => f.write_str("消息不完整"),
            Self::TrailingBytes => f.write_str("消息末尾有多余的字节"),
            Self::UnknownType(t) => write!(f, "不认识的消息类型 {t}"),
            Self::Invalid(what) => f.write_str(what),
        }
    }
}

impl std::error::Error for DecodeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_of_every_field_type() {
        let key = NodeKey::from_bytes([7; 32]);
        let v4: SocketAddr = "192.0.2.1:41641".parse().unwrap();
        let v6: SocketAddr = "[2001:db8::1]:443".parse().unwrap();

        let mut w = Writer::new();
        w.u8(0xAB);
        w.u16(0x1234);
        w.key(&key);
        w.ipv4(Ipv4Addr::new(100, 64, 0, 1));
        w.socket_addr(&v4);
        w.socket_addr(&v6);
        w.list(&[1u16, 2, 3], |w, v| w.u16(*v));
        w.string("中继");
        let bytes = w.finish();

        let mut r = Reader::new(&bytes);
        assert_eq!(r.u8(), Ok(0xAB));
        assert_eq!(r.u16(), Ok(0x1234));
        assert_eq!(r.key(), Ok(key));
        assert_eq!(r.ipv4(), Ok(Ipv4Addr::new(100, 64, 0, 1)));
        assert_eq!(r.socket_addr(), Ok(v4));
        assert_eq!(r.socket_addr(), Ok(v6));
        assert_eq!(r.list(|r| r.u16()), Ok(vec![1, 2, 3]));
        assert_eq!(r.string(), Ok("中继".to_string()));
        assert_eq!(r.finish(), Ok(()));
    }

    #[test]
    fn every_prefix_of_a_message_is_truncated() {
        let mut w = Writer::new();
        w.key(&NodeKey::from_bytes([1; 32]));
        w.socket_addr(&"[2001:db8::1]:443".parse().unwrap());
        w.string("abc");
        let bytes = w.finish();

        for len in 0..bytes.len() {
            let mut r = Reader::new(&bytes[..len]);
            let result = (|| {
                r.key()?;
                r.socket_addr()?;
                r.string()
            })();
            assert_eq!(result, Err(DecodeError::Truncated), "截到 {len} 字节");
        }
    }

    #[test]
    fn huge_list_count_does_not_allocate_up_front() {
        // 声称有 65535 项，实际一项都没有
        let bytes = [0xFF, 0xFF];
        let mut r = Reader::new(&bytes);
        assert_eq!(r.list(|r| r.key()), Err(DecodeError::Truncated));
    }

    #[test]
    fn rejects_bad_values_and_leftovers() {
        let mut r = Reader::new(&[5, 0, 0, 0, 0, 0, 0]);
        assert_eq!(r.socket_addr(), Err(DecodeError::Invalid("未知的地址族")));

        let mut r = Reader::new(&[0, 2, 0xFF, 0xFE]);
        assert_eq!(r.string(), Err(DecodeError::Invalid("字符串不是 UTF-8")));

        let mut r = Reader::new(&[1, 2]);
        r.u8().unwrap();
        assert_eq!(r.finish(), Err(DecodeError::TrailingBytes));
    }
}
