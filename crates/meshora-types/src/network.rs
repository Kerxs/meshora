use std::fmt;
use std::str::FromStr;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand_core::{OsRng, RngCore};

/// 网络 ID 的字节数。
const LEN: usize = 16;
/// 文本形式的长度：16 字节的 base64url，不带填充
const TEXT_LEN: usize = 22;

/// 一个协调服务托管很多网络时（hub 模式），用它区分是哪一个。
///
/// 它**不是秘密**：知道网络 ID 只能找到这个网络，进不进得去还要看邀请码或者名单。
/// 随机生成，不按顺序编号 —— 不让人一个个试出别人的网络。文本形式和邀请码一样是 base64url。
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NetworkId([u8; LEN]);

impl NetworkId {
    /// 生成一个新的。
    pub fn generate() -> Self {
        let mut bytes = [0u8; LEN];
        OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }

    /// 由原始字节构造。
    pub const fn from_bytes(bytes: [u8; LEN]) -> Self {
        Self(bytes)
    }

    /// 原始字节。
    pub const fn as_bytes(&self) -> &[u8; LEN] {
        &self.0
    }
}

impl fmt::Display for NetworkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&URL_SAFE_NO_PAD.encode(self.0))
    }
}

impl fmt::Debug for NetworkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NetworkId({self})")
    }
}

/// 文本不是合法的网络 ID。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseNetworkIdError;

impl fmt::Display for ParseNetworkIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "网络 ID 应为 {TEXT_LEN} 个字符的 base64url")
    }
}

impl std::error::Error for ParseNetworkIdError {}

impl FromStr for NetworkId {
    type Err = ParseNetworkIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != TEXT_LEN {
            return Err(ParseNetworkIdError);
        }
        let bytes = URL_SAFE_NO_PAD.decode(s).map_err(|_| ParseNetworkIdError)?;
        let bytes: [u8; LEN] = bytes.try_into().map_err(|_| ParseNetworkIdError)?;
        Ok(Self(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_text() {
        let id = NetworkId::generate();
        let text = id.to_string();
        assert_eq!(text.len(), TEXT_LEN);
        assert!(!text.contains(['+', '/', '=', '#', '@']));
        assert_eq!(text.parse::<NetworkId>(), Ok(id));
        assert_ne!(NetworkId::generate(), id);
    }

    #[test]
    fn rejects_malformed_text() {
        for bad in ["", "short", "!!!!!!!!!!!!!!!!!!!!!!"] {
            assert!(bad.parse::<NetworkId>().is_err(), "{bad:?}");
        }
    }
}
