use std::fmt;
use std::str::FromStr;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand_core::{OsRng, RngCore};

/// 邀请码的字节数。128 位，猜不到。
const LEN: usize = 16;
/// 文本形式的长度：16 字节的 base64url，不带填充
const TEXT_LEN: usize = 22;

/// 邀请码：拿着它的节点可以自己加入一个网络，不用网络主人先把公钥加进名单。
///
/// 它是一个共享的秘密，谁拿到都能加入 —— 和"网络名 + 密码"是一回事。它只在 Noise 加密的
/// 控制通道里传，协调服务比较时用常数时间。文本形式是 base64url（不带填充），
/// 能直接放进网络码里，也不会被当成 URL 里的特殊字符。
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Invite([u8; LEN]);

impl Invite {
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

    /// 常数时间比较：比较的耗时不随"前面对了几个字节"变化，不能靠计时一个字节一个字节地猜。
    pub fn matches(&self, other: &Invite) -> bool {
        self.0
            .iter()
            .zip(other.0.iter())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
    }
}

impl fmt::Display for Invite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&URL_SAFE_NO_PAD.encode(self.0))
    }
}

/// 日志里不打出邀请码本身：它是秘密
impl fmt::Debug for Invite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Invite(..)")
    }
}

/// 文本不是合法的邀请码。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseInviteError;

impl fmt::Display for ParseInviteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "邀请码应为 {TEXT_LEN} 个字符的 base64url")
    }
}

impl std::error::Error for ParseInviteError {}

impl FromStr for Invite {
    type Err = ParseInviteError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != TEXT_LEN {
            return Err(ParseInviteError);
        }
        let bytes = URL_SAFE_NO_PAD.decode(s).map_err(|_| ParseInviteError)?;
        let bytes: [u8; LEN] = bytes.try_into().map_err(|_| ParseInviteError)?;
        Ok(Self(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_text() {
        let invite = Invite::generate();
        let text = invite.to_string();
        assert_eq!(text.len(), TEXT_LEN);
        assert!(!text.contains(['+', '/', '=', '#', '@']));
        assert_eq!(text.parse::<Invite>(), Ok(invite));
    }

    #[test]
    fn two_invites_differ() {
        assert_ne!(Invite::generate(), Invite::generate());
    }

    #[test]
    fn matching_is_exact() {
        let invite = Invite::from_bytes([7; LEN]);
        assert!(invite.matches(&Invite::from_bytes([7; LEN])));
        let mut other = [7; LEN];
        other[LEN - 1] = 8;
        assert!(!invite.matches(&Invite::from_bytes(other)));
    }

    #[test]
    fn rejects_malformed_text() {
        for bad in [
            "",
            "short",
            "!!!!!!!!!!!!!!!!!!!!!!",
            "BwcHBwcHBwcHBwcHBwcHBw==",
        ] {
            assert!(bad.parse::<Invite>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn debug_does_not_leak_the_secret() {
        let invite = Invite::generate();
        assert!(!format!("{invite:?}").contains(&invite.to_string()));
    }
}
