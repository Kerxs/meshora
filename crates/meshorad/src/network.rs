//! 网络码：加入一个联机网络要的全部信息 —— 协调服务的公钥和地址，可能还有网络 ID 和邀请码。
//!
//! 写成 `公钥@主机:端口/网络ID#邀请码`，比如 `mTe0...Xk=@play.example.com:7443/Qm9yZ...#3q2-7wEYkQ6n0Cf8Hs5VYA`。
//! 建网络的人把它发给朋友，朋友贴进客户端就能加入。前面可以带 `meshora:`，前后的空白不算。
//!
//! - `/网络ID` 只有托管很多网络的协调服务（hub 模式）才有；不带的是单网络的协调服务
//! - 网络 ID、邀请码都不带的，就是一台协调服务的地址：在它上面建网络用
//!
//! - **不带邀请码的**不是秘密：拿到它只能连到协调服务，协调服务不认你的公钥照样拒之门外
//! - **带邀请码的是秘密**：谁拿到都能加入这个网络。只发给要一起玩的人
//!
//! 公钥是用来认协调服务的 —— 有了它，冒充协调服务的人过不了握手。

use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::str::FromStr;

use meshora_control::Entry;
use meshora_types::{Invite, NetworkId, NodeKey};

const PREFIX: &str = "meshora:";

/// 一个网络码：协调服务的公钥和地址，可能还有网络 ID 和邀请码。写成 `公钥@主机:端口/网络ID#邀请码`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkCode {
    /// 协调服务的公钥。
    pub coord_key: NodeKey,
    /// 协调服务的主机名或 IP。IPv6 地址不带方括号。
    pub host: String,
    /// 协调服务的端口。
    pub port: u16,
    /// 网络 ID：协调服务托管很多网络时，进的是哪一个。
    pub network: Option<NetworkId>,
    /// 邀请码。有了它，不在名单里也能加入。
    pub invite: Option<Invite>,
}

impl NetworkCode {
    /// 用已知的地址拼一个网络码。
    pub fn new(coord_key: NodeKey, addr: SocketAddr) -> Self {
        Self {
            coord_key,
            host: addr.ip().to_string(),
            port: addr.port(),
            network: None,
            invite: None,
        }
    }

    /// 连协调服务时第一条消息怎么说：有网络 ID 就是进 hub 里的那个网络，没有就是单网络的老说法。
    pub fn entry(&self) -> Entry {
        match self.network {
            Some(network) => Entry::Join {
                network,
                invite: self.invite,
            },
            None => Entry::Hello {
                invite: self.invite,
            },
        }
    }

    /// 只留协调服务的公钥和地址（建网络时用的"服务器地址"）。
    pub fn server(&self) -> Self {
        Self {
            network: None,
            invite: None,
            ..self.clone()
        }
    }

    /// 把主机名解析成地址。有 IPv4 地址就用 IPv4 —— 本机数据面的 socket 只绑了 IPv4。
    pub async fn resolve(&self) -> io::Result<SocketAddr> {
        let addrs: Vec<SocketAddr> = tokio::net::lookup_host((self.host.as_str(), self.port))
            .await?
            .collect();
        addrs
            .iter()
            .find(|addr| addr.is_ipv4())
            .or(addrs.first())
            .copied()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("{} 解析不出地址", self.host),
                )
            })
    }
}

impl fmt::Display for NetworkCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.host.contains(':') {
            write!(f, "{}@[{}]:{}", self.coord_key, self.host, self.port)?;
        } else {
            write!(f, "{}@{}:{}", self.coord_key, self.host, self.port)?;
        }
        if let Some(network) = &self.network {
            write!(f, "/{network}")?;
        }
        match &self.invite {
            Some(invite) => write!(f, "#{invite}"),
            None => Ok(()),
        }
    }
}

/// 网络码格式不对，说明哪里不对。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseNetworkCodeError(&'static str);

impl fmt::Display for ParseNetworkCodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for ParseNetworkCodeError {}

impl FromStr for NetworkCode {
    type Err = ParseNetworkCodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let s = s.strip_prefix(PREFIX).unwrap_or(s);
        let (s, invite) = match s.rsplit_once('#') {
            Some((rest, invite)) => (
                rest,
                Some(
                    invite
                        .parse()
                        .map_err(|_| ParseNetworkCodeError("网络码里 # 后面不是合法的邀请码"))?,
                ),
            ),
            None => (s, None),
        };
        let (key, addr) = s
            .split_once('@')
            .ok_or(ParseNetworkCodeError("网络码应为 公钥@地址:端口，没找到 @"))?;
        let coord_key = key
            .parse()
            .map_err(|_| ParseNetworkCodeError("网络码里 @ 前面不是合法的公钥"))?;
        // 地址里不会有 /（公钥里倒可能有，但它在 @ 前面）
        let (addr, network) = match addr.split_once('/') {
            Some((addr, network)) => (
                addr,
                Some(
                    network
                        .parse()
                        .map_err(|_| ParseNetworkCodeError("网络码里 / 后面不是合法的网络 ID"))?,
                ),
            ),
            None => (addr, None),
        };
        let (host, port) = addr
            .rsplit_once(':')
            .ok_or(ParseNetworkCodeError("网络码里的地址缺少端口"))?;
        let port = port
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or(ParseNetworkCodeError("网络码里的端口不对"))?;
        let host = match host.strip_prefix('[') {
            Some(v6) => v6
                .strip_suffix(']')
                .ok_or(ParseNetworkCodeError("网络码里的 IPv6 地址少了 ]"))?,
            None if host.contains(':') => {
                return Err(ParseNetworkCodeError(
                    "网络码里的 IPv6 地址要用方括号括起来",
                ));
            }
            None => host,
        };
        if host.is_empty() || host.chars().any(char::is_whitespace) {
            return Err(ParseNetworkCodeError("网络码里的地址不对"));
        }
        Ok(Self {
            coord_key,
            host: host.to_owned(),
            port,
            network,
            invite,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> NodeKey {
        NodeKey::from_bytes([7; 32])
    }

    #[test]
    fn parses_host_names_and_addresses() {
        let code: NetworkCode = format!("{}@play.example.com:7443", key()).parse().unwrap();
        assert_eq!(code.coord_key, key());
        assert_eq!(code.host, "play.example.com");
        assert_eq!(code.port, 7443);

        let code: NetworkCode = format!("  meshora:{}@203.0.113.5:7443\n", key())
            .parse()
            .unwrap();
        assert_eq!(code.host, "203.0.113.5");

        let code: NetworkCode = format!("{}@[2001:db8::1]:7443", key()).parse().unwrap();
        assert_eq!(code.host, "2001:db8::1");
        assert_eq!(code.invite, None);
    }

    #[test]
    fn carries_an_invite() {
        let invite = Invite::from_bytes([3; 16]);
        let code: NetworkCode = format!("{}@play.example.com:7443#{invite}", key())
            .parse()
            .unwrap();
        assert_eq!(code.invite, Some(invite));
        assert_eq!(code.host, "play.example.com");
        assert_eq!(code.port, 7443);

        let code: NetworkCode = format!("{}@[2001:db8::1]:7443#{invite}", key())
            .parse()
            .unwrap();
        assert_eq!(code.host, "2001:db8::1");
        assert_eq!(code.invite, Some(invite));

        assert!(
            format!("{}@play.example.com:7443#nope", key())
                .parse::<NetworkCode>()
                .is_err()
        );
    }

    #[test]
    fn carries_a_network_id() {
        let network = NetworkId::from_bytes([5; 16]);
        let invite = Invite::from_bytes([3; 16]);
        // 公钥里可能有 /：网络 ID 只认 @ 后面的那个
        let slashy = NodeKey::from_bytes([0xff; 32]);
        assert!(slashy.to_string().contains('/'));
        let code: NetworkCode = format!("{slashy}@play.example.com:7443/{network}#{invite}")
            .parse()
            .unwrap();
        assert_eq!(code.coord_key, slashy);
        assert_eq!(code.port, 7443);
        assert_eq!(code.network, Some(network));
        assert_eq!(code.invite, Some(invite));
        assert_eq!(
            code.entry(),
            Entry::Join {
                network,
                invite: Some(invite)
            }
        );
        assert_eq!(
            code.server().to_string(),
            format!("{slashy}@play.example.com:7443")
        );

        let plain: NetworkCode = format!("{}@play.example.com:7443#{invite}", key())
            .parse()
            .unwrap();
        assert_eq!(
            plain.entry(),
            Entry::Hello {
                invite: Some(invite)
            }
        );
        assert!(
            format!("{}@play.example.com:7443/nope", key())
                .parse::<NetworkCode>()
                .is_err()
        );
    }

    #[test]
    fn round_trips_through_display() {
        let invite = Invite::from_bytes([3; 16]);
        let network = NetworkId::from_bytes([5; 16]);
        for text in [
            format!("{}@play.example.com:7443/{network}#{invite}", key()),
            format!("{}@[2001:db8::1]:7443/{network}", key()),
            format!("{}@play.example.com:7443", key()),
            format!("{}@[2001:db8::1]:7443", key()),
            format!("{}@play.example.com:7443#{invite}", key()),
            format!("{}@[2001:db8::1]:7443#{invite}", key()),
        ] {
            let code: NetworkCode = text.parse().unwrap();
            assert_eq!(code.to_string(), text);
        }
        let code = NetworkCode::new(key(), "203.0.113.5:7443".parse().unwrap());
        assert_eq!(code.to_string().parse::<NetworkCode>().unwrap(), code);
    }

    #[test]
    fn rejects_malformed_codes() {
        let k = key();
        for bad in [
            "".to_string(),
            "play.example.com:7443".to_string(),
            "not-a-key@play.example.com:7443".to_string(),
            format!("{k}@play.example.com"),
            format!("{k}@play.example.com:0"),
            format!("{k}@play.example.com:99999"),
            format!("{k}@:7443"),
            format!("{k}@2001:db8::1:7443"),
            format!("{k}@[2001:db8::1:7443"),
            format!("{k}@play example.com:7443"),
        ] {
            assert!(bad.parse::<NetworkCode>().is_err(), "{bad:?} 不该通过");
        }
    }

    #[tokio::test]
    async fn resolves_literal_addresses() {
        let code: NetworkCode = format!("{}@127.0.0.1:7443", key()).parse().unwrap();
        assert_eq!(
            code.resolve().await.unwrap(),
            "127.0.0.1:7443".parse().unwrap()
        );
    }
}
