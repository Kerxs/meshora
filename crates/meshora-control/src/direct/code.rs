//! 直连模式的两种连接码：房主发给朋友的**房主码**（[`Offer`]），朋友发回来的**回执码**（[`Reply`]）。
//!
//! 码里是对方要的全部东西：身份（WireGuard 公钥）、overlay 地址、候选端点（局域网地址、
//! STUN 问到的公网地址）、名字、过期时间。文字形式是 `meshora-offer:` / `meshora-reply:` 加 base64url。
//!
//! **不签名，只有校验和。** 签名防不了要防的事：谁能改码，谁就能连公钥一起换掉，签名跟着换。
//! 码的可信全靠传递它的渠道（聊天软件里本人发的），和网络码、邀请码一样 —— 威胁模型里写着。
//! 校验和只用来发现抄错、截断。WireGuard 握手认的是码里的公钥：码里的公钥不是对方的，就握不上手。

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use meshora_proto::control::clean_name;
use meshora_types::NodeKey;

/// 房主码的前缀。
pub const OFFER_PREFIX: &str = "meshora-offer:";
/// 回执码的前缀。
pub const REPLY_PREFIX: &str = "meshora-reply:";
/// 码多久之后作废：里面的公网端点过一阵就不准了（路由器的映射会过期、会换）。
pub const CODE_TTL: Duration = Duration::from_secs(60 * 60);
/// 码里最多几个候选端点。
const MAX_ENDPOINTS: usize = 8;
/// 第 2 版多了端口提示（[`PortHint`]）。第 1 版（1.0.2、1.0.3 生成的）照样读
const VERSION: u8 = 2;
const KIND_OFFER: u8 = 1;
const KIND_REPLY: u8 = 2;

/// 对称型 NAT 的端口提示：问两个 STUN 服务器看到的外部端口不一样（每个目的地换一个端口），
/// 很多路由器是按顺序分配的 —— 下一个目的地大概会拿到 `port + step`。
/// 对方据此往预测的那一段端口集中探测（见 [`PortHint::predict`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortHint {
    /// 公网 IPv4 地址。
    pub ip: Ipv4Addr,
    /// 最后一次看到的外部端口。
    pub port: u16,
    /// 相邻两次分配的端口差（可以是负的）。
    pub step: i16,
}

/// 预测往后推几个端口
const PREDICT: i32 = 64;

impl PortHint {
    /// 从两个 STUN 服务器先后看到的端点推出提示：同一个公网 IP、端口不同、差得不多（按顺序分配）才算。
    /// 端口差得很大（随机分配）时猜不中，不给
    pub fn from_observed(first: SocketAddr, second: SocketAddr) -> Option<Self> {
        let (SocketAddr::V4(a), SocketAddr::V4(b)) = (first, second) else {
            return None;
        };
        let step = i32::from(b.port()) - i32::from(a.port());
        if a.ip() != b.ip() || step == 0 || step.abs() > 64 {
            return None;
        }
        Some(Self {
            ip: *b.ip(),
            port: b.port(),
            step: step as i16,
        })
    }

    /// 预测的端点：往后推 [`PREDICT`] 个分配，每个分配前后各放宽一个端口（中间别的程序也在占端口）
    pub fn predict(&self) -> Vec<SocketAddr> {
        let mut out = Vec::new();
        for k in 1..=PREDICT {
            let center = i32::from(self.port) + i32::from(self.step) * k;
            for port in [center, center + 1] {
                if let Ok(port) = u16::try_from(port)
                    && port >= 1024
                {
                    let addr = SocketAddr::from((self.ip, port));
                    if !out.contains(&addr) {
                        out.push(addr);
                    }
                }
            }
        }
        out
    }
}

/// 房主码：房主发给一位朋友。每位朋友一个，里面带着房主分给他的地址。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    /// 房主的身份。
    pub host: NodeKey,
    /// 房主的 overlay 地址。
    pub host_ip: Ipv4Addr,
    /// 分给这位朋友的 overlay 地址。
    pub guest_ip: Ipv4Addr,
    /// overlay 网段的前缀长度。
    pub prefix_len: u8,
    /// 房主的名字。
    pub name: String,
    /// 房主的候选端点。
    pub endpoints: Vec<SocketAddr>,
    /// 房主是对称型 NAT 时的端口提示。
    pub hint: Option<PortHint>,
    /// 过期时间（Unix 秒）。
    pub expires: u64,
}

/// 回执码：朋友收到房主码之后发回给房主。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    /// 朋友的身份。
    pub guest: NodeKey,
    /// 这是回给哪个房主的：房主核对它，免得贴进了给别人的回执。
    pub host: NodeKey,
    /// 房主码里分给朋友的地址，原样带回来。
    pub guest_ip: Ipv4Addr,
    /// 朋友的名字。
    pub name: String,
    /// 朋友的候选端点。
    pub endpoints: Vec<SocketAddr>,
    /// 朋友是对称型 NAT 时的端口提示。
    pub hint: Option<PortHint>,
    /// 过期时间（Unix 秒）。
    pub expires: u64,
}

/// 连接码读不出来的原因。给人看的话在 [`Display`](fmt::Display) 里。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeError {
    /// 不是 Meshora 的连接码。
    NotACode,
    /// 要的是房主码，贴进来的是回执码（或者反过来）。
    WrongKind {
        /// 要的是不是房主码。
        wanted_offer: bool,
    },
    /// 校验和对不上、长度不对：多半是没复制全，或者被改了几个字。
    Damaged,
    /// 新版本的客户端生成的，这个版本读不懂。
    Version,
    /// 已经过期。
    Expired,
}

impl fmt::Display for CodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotACode => "这不是 Meshora 的连接码",
            Self::WrongKind { wanted_offer: true } => {
                "这是回执码，要贴的是房主发给你的房主码（meshora-offer: 开头）"
            }
            Self::WrongKind {
                wanted_offer: false,
            } => "这是房主码，要贴的是朋友发回来的回执码（meshora-reply: 开头）",
            Self::Damaged => "连接码不完整或者被改动过，请对方重新复制一遍",
            Self::Version => "连接码是更新版本的 Meshora 生成的，先把客户端更新到最新",
            Self::Expired => "连接码已经过期（一小时内有效），请对方重新生成一个",
        })
    }
}

impl std::error::Error for CodeError {}

/// 现在起 [`CODE_TTL`] 之后的 Unix 秒数。
pub fn expires_from_now() -> u64 {
    unix_now() + CODE_TTL.as_secs()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Offer {
    /// 写成文字。
    pub fn encode(&self) -> String {
        let mut w = Writer::new(KIND_OFFER, self.expires);
        w.key(&self.host);
        w.ip(self.host_ip);
        w.ip(self.guest_ip);
        w.bytes.push(self.prefix_len);
        w.name(&self.name);
        w.endpoints(&self.endpoints);
        w.hint(self.hint);
        w.finish(OFFER_PREFIX)
    }

    /// 从文字读出来，并检查有没有过期。
    pub fn decode(text: &str) -> Result<Self, CodeError> {
        Self::decode_at(text, unix_now())
    }

    fn decode_at(text: &str, now: u64) -> Result<Self, CodeError> {
        let mut r = Reader::open(text, true, now)?;
        let offer = Self {
            expires: r.expires,
            host: r.key()?,
            host_ip: r.ip()?,
            guest_ip: r.ip()?,
            prefix_len: r.u8()?,
            name: r.name()?,
            endpoints: r.endpoints()?,
            hint: r.hint()?,
        };
        r.done()?;
        if offer.prefix_len > 30 || offer.host_ip == offer.guest_ip {
            return Err(CodeError::Damaged);
        }
        Ok(offer)
    }
}

impl Reply {
    /// 写成文字。
    pub fn encode(&self) -> String {
        let mut w = Writer::new(KIND_REPLY, self.expires);
        w.key(&self.guest);
        w.key(&self.host);
        w.ip(self.guest_ip);
        w.name(&self.name);
        w.endpoints(&self.endpoints);
        w.hint(self.hint);
        w.finish(REPLY_PREFIX)
    }

    /// 从文字读出来，并检查有没有过期。
    pub fn decode(text: &str) -> Result<Self, CodeError> {
        Self::decode_at(text, unix_now())
    }

    fn decode_at(text: &str, now: u64) -> Result<Self, CodeError> {
        let mut r = Reader::open(text, false, now)?;
        let reply = Self {
            expires: r.expires,
            guest: r.key()?,
            host: r.key()?,
            guest_ip: r.ip()?,
            name: r.name()?,
            endpoints: r.endpoints()?,
            hint: r.hint()?,
        };
        r.done()?;
        Ok(reply)
    }
}

/// FNV-1a，32 位。只用来发现抄错，不是安全机制（见模块文档）
fn checksum(bytes: &[u8]) -> [u8; 4] {
    let mut hash: u32 = 0x811c_9dc5;
    for &b in bytes {
        hash ^= u32::from(b);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash.to_be_bytes()
}

struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    fn new(kind: u8, expires: u64) -> Self {
        let mut bytes = vec![VERSION, kind];
        bytes.extend_from_slice(&expires.to_be_bytes());
        Self { bytes }
    }

    fn key(&mut self, key: &NodeKey) {
        self.bytes.extend_from_slice(key.as_bytes());
    }

    fn ip(&mut self, ip: Ipv4Addr) {
        self.bytes.extend_from_slice(&ip.octets());
    }

    fn name(&mut self, name: &str) {
        let name = clean_name(name);
        // 32 个字符最多 128 字节，放得进一个字节的长度
        self.bytes.push(name.len() as u8);
        self.bytes.extend_from_slice(name.as_bytes());
    }

    fn endpoints(&mut self, endpoints: &[SocketAddr]) {
        let endpoints = &endpoints[..endpoints.len().min(MAX_ENDPOINTS)];
        self.bytes.push(endpoints.len() as u8);
        for addr in endpoints {
            match addr.ip() {
                IpAddr::V4(ip) => {
                    self.bytes.push(4);
                    self.bytes.extend_from_slice(&ip.octets());
                }
                IpAddr::V6(ip) => {
                    self.bytes.push(6);
                    self.bytes.extend_from_slice(&ip.octets());
                }
            }
            self.bytes.extend_from_slice(&addr.port().to_be_bytes());
        }
    }

    fn hint(&mut self, hint: Option<PortHint>) {
        match hint {
            None => self.bytes.push(0),
            Some(hint) => {
                self.bytes.push(1);
                self.bytes.extend_from_slice(&hint.ip.octets());
                self.bytes.extend_from_slice(&hint.port.to_be_bytes());
                self.bytes.extend_from_slice(&hint.step.to_be_bytes());
            }
        }
    }

    fn finish(mut self, prefix: &str) -> String {
        let sum = checksum(&self.bytes);
        self.bytes.extend_from_slice(&sum);
        format!("{prefix}{}", URL_SAFE_NO_PAD.encode(&self.bytes))
    }
}

struct Reader {
    bytes: Vec<u8>,
    at: usize,
    expires: u64,
    version: u8,
}

impl Reader {
    fn open(text: &str, wanted_offer: bool, now: u64) -> Result<Self, CodeError> {
        // 聊天软件会在长串里折行、加空格：去掉所有空白再读
        let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        let (body, is_offer) = if let Some(body) = text.strip_prefix(OFFER_PREFIX) {
            (body, true)
        } else if let Some(body) = text.strip_prefix(REPLY_PREFIX) {
            (body, false)
        } else {
            return Err(CodeError::NotACode);
        };
        if is_offer != wanted_offer {
            return Err(CodeError::WrongKind { wanted_offer });
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(body)
            .map_err(|_| CodeError::Damaged)?;
        let Some((payload, sum)) = bytes.split_last_chunk::<4>() else {
            return Err(CodeError::Damaged);
        };
        if checksum(payload) != *sum {
            return Err(CodeError::Damaged);
        }
        let mut reader = Self {
            bytes: payload.to_vec(),
            at: 0,
            expires: 0,
            version: 0,
        };
        reader.version = reader.u8()?;
        if !(1..=VERSION).contains(&reader.version) {
            return Err(CodeError::Version);
        }
        let kind = reader.u8()?;
        if kind != if wanted_offer { KIND_OFFER } else { KIND_REPLY } {
            return Err(CodeError::Damaged);
        }
        reader.expires = u64::from_be_bytes(reader.take::<8>()?);
        if now > reader.expires {
            return Err(CodeError::Expired);
        }
        Ok(reader)
    }

    fn take<const N: usize>(&mut self) -> Result<[u8; N], CodeError> {
        let chunk = self
            .bytes
            .get(self.at..self.at + N)
            .ok_or(CodeError::Damaged)?;
        self.at += N;
        Ok(chunk.try_into().expect("长度刚核对过"))
    }

    fn u8(&mut self) -> Result<u8, CodeError> {
        Ok(self.take::<1>()?[0])
    }

    fn key(&mut self) -> Result<NodeKey, CodeError> {
        Ok(NodeKey::from_bytes(self.take::<32>()?))
    }

    fn ip(&mut self) -> Result<Ipv4Addr, CodeError> {
        Ok(Ipv4Addr::from(self.take::<4>()?))
    }

    fn name(&mut self) -> Result<String, CodeError> {
        let len = usize::from(self.u8()?);
        let raw = self
            .bytes
            .get(self.at..self.at + len)
            .ok_or(CodeError::Damaged)?;
        self.at += len;
        let name = std::str::from_utf8(raw).map_err(|_| CodeError::Damaged)?;
        Ok(clean_name(name))
    }

    fn endpoints(&mut self) -> Result<Vec<SocketAddr>, CodeError> {
        let count = usize::from(self.u8()?);
        if count > MAX_ENDPOINTS {
            return Err(CodeError::Damaged);
        }
        (0..count)
            .map(|_| {
                let ip = match self.u8()? {
                    4 => IpAddr::from(self.take::<4>()?),
                    6 => IpAddr::from(self.take::<16>()?),
                    _ => return Err(CodeError::Damaged),
                };
                let port = u16::from_be_bytes(self.take::<2>()?);
                Ok(SocketAddr::new(ip, port))
            })
            .collect()
    }

    /// 第 2 版起才有端口提示
    fn hint(&mut self) -> Result<Option<PortHint>, CodeError> {
        if self.version < 2 || self.u8()? == 0 {
            return Ok(None);
        }
        Ok(Some(PortHint {
            ip: self.ip()?,
            port: u16::from_be_bytes(self.take::<2>()?),
            step: i16::from_be_bytes(self.take::<2>()?),
        }))
    }

    fn done(&self) -> Result<(), CodeError> {
        if self.at == self.bytes.len() {
            Ok(())
        } else {
            Err(CodeError::Damaged)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshora_types::NodeSecret;

    fn offer() -> Offer {
        Offer {
            host: NodeSecret::generate().public_key(),
            host_ip: Ipv4Addr::new(100, 96, 0, 1),
            guest_ip: Ipv4Addr::new(100, 96, 0, 2),
            prefix_len: 24,
            name: "阿杰的台式机".into(),
            endpoints: vec![
                "192.168.1.20:41641".parse().unwrap(),
                "203.0.113.9:53122".parse().unwrap(),
                "[2001:db8::7]:41641".parse().unwrap(),
            ],
            hint: Some(PortHint {
                ip: Ipv4Addr::new(203, 0, 113, 9),
                port: 53122,
                step: 2,
            }),
            expires: 2_000_000_000,
        }
    }

    const NOW: u64 = 1_900_000_000;

    #[test]
    fn an_offer_round_trips() {
        let offer = offer();
        let text = offer.encode();
        assert!(text.starts_with(OFFER_PREFIX));
        assert_eq!(Offer::decode_at(&text, NOW), Ok(offer));
    }

    #[test]
    fn a_reply_round_trips() {
        let reply = Reply {
            guest: NodeSecret::generate().public_key(),
            host: NodeSecret::generate().public_key(),
            guest_ip: Ipv4Addr::new(100, 96, 0, 2),
            name: "小明".into(),
            endpoints: vec!["198.51.100.4:6000".parse().unwrap()],
            hint: None,
            expires: 2_000_000_000,
        };
        let text = reply.encode();
        assert_eq!(Reply::decode_at(&text, NOW), Ok(reply));
    }

    #[test]
    fn chat_apps_breaking_the_line_do_not_matter() {
        let offer = offer();
        let text = offer.encode();
        let (a, b) = text.split_at(30);
        let wrapped = format!("  {a}\n {b} \n");
        assert_eq!(Offer::decode_at(&wrapped, NOW), Ok(offer));
    }

    #[test]
    fn the_wrong_kind_is_named() {
        let text = offer().encode();
        assert_eq!(
            Reply::decode_at(&text, NOW),
            Err(CodeError::WrongKind {
                wanted_offer: false
            })
        );
        assert_eq!(Offer::decode_at("你好", NOW), Err(CodeError::NotACode));
    }

    #[test]
    fn a_changed_or_cut_code_is_caught() {
        let text = offer().encode();
        // 改一个字符
        let mut changed: Vec<char> = text.chars().collect();
        let i = OFFER_PREFIX.len() + 20;
        changed[i] = if changed[i] == 'A' { 'B' } else { 'A' };
        let changed: String = changed.into_iter().collect();
        assert_eq!(Offer::decode_at(&changed, NOW), Err(CodeError::Damaged));
        // 少复制了最后几个字符
        assert_eq!(
            Offer::decode_at(&text[..text.len() - 3], NOW),
            Err(CodeError::Damaged)
        );
    }

    #[test]
    fn an_expired_code_is_refused() {
        let text = offer().encode();
        assert_eq!(
            Offer::decode_at(&text, 2_000_000_001),
            Err(CodeError::Expired)
        );
    }

    #[test]
    fn long_names_and_many_endpoints_are_trimmed() {
        let mut long = offer();
        long.name = "名".repeat(80);
        long.endpoints = (0..20)
            .map(|i| SocketAddr::from(([10, 0, 0, i], 1000)))
            .collect();
        let back = Offer::decode_at(&long.encode(), NOW).unwrap();
        assert_eq!(back.name.chars().count(), 32);
        assert_eq!(back.endpoints.len(), MAX_ENDPOINTS);
    }

    #[test]
    fn version_1_codes_still_read() {
        // 1.0.2、1.0.3 生成的码：没有端口提示那一段
        let mut offer = offer();
        offer.hint = None;
        let mut w = Writer::new(KIND_OFFER, offer.expires);
        w.bytes[0] = 1;
        w.key(&offer.host);
        w.ip(offer.host_ip);
        w.ip(offer.guest_ip);
        w.bytes.push(offer.prefix_len);
        w.name(&offer.name);
        w.endpoints(&offer.endpoints);
        let text = w.finish(OFFER_PREFIX);
        assert_eq!(Offer::decode_at(&text, NOW), Ok(offer));
    }

    #[test]
    fn a_hint_comes_from_sequential_ports_only() {
        let a: SocketAddr = "203.0.113.9:40000".parse().unwrap();
        let hint = PortHint::from_observed(a, "203.0.113.9:40002".parse().unwrap()).unwrap();
        assert_eq!((hint.port, hint.step), (40002, 2));
        assert_eq!(PortHint::from_observed(a, a), None, "端口一样：不是对称型");
        assert_eq!(
            PortHint::from_observed(a, "203.0.113.9:51234".parse().unwrap()),
            None,
            "随机分配，猜不中"
        );
        assert_eq!(
            PortHint::from_observed(a, "198.51.100.1:40001".parse().unwrap()),
            None,
            "公网 IP 不一样"
        );
    }

    #[test]
    fn predictions_follow_the_step() {
        let hint = PortHint {
            ip: Ipv4Addr::new(203, 0, 113, 9),
            port: 40002,
            step: 2,
        };
        let ports: Vec<u16> = hint.predict().iter().map(SocketAddr::port).collect();
        assert_eq!(&ports[..4], &[40004, 40005, 40006, 40007]);
        assert!(ports.len() <= 2 * PREDICT as usize);
        let low = PortHint {
            ip: Ipv4Addr::new(203, 0, 113, 9),
            port: 1030,
            step: -4,
        };
        assert!(
            low.predict().iter().all(|a| a.port() >= 1024),
            "不预测系统端口"
        );
    }
}
