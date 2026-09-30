//! 网络码：加入一个联机网络要的全部信息 —— 协调服务的公钥和地址。
//!
//! 写成 `公钥@主机:端口`，比如 `mTe0...Xk=@play.example.com:7443`。建网络的人把它发给朋友，
//! 朋友贴进客户端就能连。前面可以带 `meshora:`，前后的空白不算。
//!
//! 网络码不是秘密：拿到它只能连到协调服务，协调服务不认你的公钥照样拒之门外。
//! 公钥是用来认协调服务的 —— 有了它，冒充协调服务的人过不了握手。

use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::str::FromStr;

use meshora_types::NodeKey;

const PREFIX: &str = "meshora:";

/// 一个网络码：协调服务的公钥和地址，写成 `公钥@主机:端口`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkCode {
    /// 协调服务的公钥。
    pub coord_key: NodeKey,
    /// 协调服务的主机名或 IP。IPv6 地址不带方括号。
    pub host: String,
    /// 协调服务的端口。
    pub port: u16,
}

impl NetworkCode {
    /// 用已知的地址拼一个网络码。
    pub fn new(coord_key: NodeKey, addr: SocketAddr) -> Self {
        Self {
            coord_key,
            host: addr.ip().to_string(),
            port: addr.port(),
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
            write!(f, "{}@[{}]:{}", self.coord_key, self.host, self.port)
        } else {
            write!(f, "{}@{}:{}", self.coord_key, self.host, self.port)
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
        let (key, addr) = s
            .split_once('@')
            .ok_or(ParseNetworkCodeError("网络码应为 公钥@地址:端口，没找到 @"))?;
        let coord_key = key
            .parse()
            .map_err(|_| ParseNetworkCodeError("网络码里 @ 前面不是合法的公钥"))?;
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
    }

    #[test]
    fn round_trips_through_display() {
        for text in [
            format!("{}@play.example.com:7443", key()),
            format!("{}@[2001:db8::1]:7443", key()),
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
