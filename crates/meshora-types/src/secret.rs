use std::fmt;
use std::str::FromStr;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

use crate::{NodeKey, ParseNodeKeyError};

/// 节点私钥：[`NodeKey`] 的另一半。
///
/// WireGuard 握手和 Noise IK 认证用的都是它 —— 公钥即身份，认证和加密用同一对密钥。
///
/// 为了不让它意外流出去：
///
/// - 不实现 `Display`；`Debug` 只打印对应的公钥，日志里不会出现私钥
/// - 离开作用域时内存清零（x25519-dalek 的 `StaticSecret` 自带 zeroize）
/// - 文本形式只能通过 [`to_base64`](Self::to_base64) 显式取出，用来写密钥文件；
///   格式和 `wg genkey` 一样是标准 base64
#[derive(Clone)]
pub struct NodeSecret(StaticSecret);

impl NodeSecret {
    /// 用操作系统的随机数生成一把新私钥。
    pub fn generate() -> Self {
        Self(StaticSecret::random())
    }

    /// 从 32 字节构造。X25519 的 clamping 在每次 DH 时才做，这里原样保存。
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(StaticSecret::from(bytes))
    }

    /// 对应的公钥，也就是这个节点的身份。
    pub fn public_key(&self) -> NodeKey {
        NodeKey::from_bytes(PublicKey::from(&self.0).to_bytes())
    }

    /// 私钥的原始字节。只给确实需要它的协议实现（WireGuard、Noise）用。
    pub fn as_bytes(&self) -> &[u8; 32] {
        self.0.as_bytes()
    }

    /// x25519-dalek 的形式，交给 boringtun 用。
    pub fn to_static_secret(&self) -> StaticSecret {
        self.0.clone()
    }

    /// 写密钥文件用的文本形式。返回的字符串离开作用域时同样会被清零。
    pub fn to_base64(&self) -> Zeroizing<String> {
        Zeroizing::new(STANDARD.encode(self.0.as_bytes()))
    }

    /// 从密钥文件的内容读私钥：去掉首尾空白，也认文件开头的 BOM。
    ///
    /// Windows PowerShell 5.1 的 `>` 重定向会把输出写成带 BOM 的 UTF-16LE，记事本存盘时
    /// 可能加上 UTF-8 的 BOM。文件里装的还是那 44 个字符，照样认；私钥本身照旧严格解析。
    pub fn from_key_file(contents: &[u8]) -> Result<Self, ParseNodeKeyError> {
        match contents {
            [0xFF, 0xFE, rest @ ..] => {
                let (pairs, odd) = rest.as_chunks::<2>();
                if !odd.is_empty() {
                    return Err(ParseNodeKeyError);
                }
                let units: Zeroizing<Vec<u16>> =
                    Zeroizing::new(pairs.iter().map(|pair| u16::from_le_bytes(*pair)).collect());
                let text =
                    Zeroizing::new(String::from_utf16(&units).map_err(|_| ParseNodeKeyError)?);
                text.trim().parse()
            }
            [0xEF, 0xBB, 0xBF, rest @ ..] | rest => std::str::from_utf8(rest)
                .map_err(|_| ParseNodeKeyError)?
                .trim()
                .parse(),
        }
    }
}

impl fmt::Debug for NodeSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NodeSecret(公钥 {})", self.public_key())
    }
}

impl FromStr for NodeSecret {
    type Err = ParseNodeKeyError;

    /// 和 [`NodeKey`] 的解析一样严格：必须是规范的 44 字符标准 base64，
    /// 不去掉首尾空白 —— 读密钥文件时由调用方自己 trim。
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // 复用公钥的解析规则，拿到的只是 32 字节
        let bytes = Zeroizing::new(*s.parse::<NodeKey>()?.as_bytes());
        Ok(Self::from_bytes(*bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex32(s: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
        }
        out
    }

    #[test]
    fn public_key_matches_rfc7748_vector() {
        // RFC 7748 第 6.1 节，Alice 的密钥对
        let secret = NodeSecret::from_bytes(hex32(
            "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a",
        ));
        let public = hex32("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a");
        assert_eq!(secret.public_key(), NodeKey::from_bytes(public));
    }

    #[test]
    fn generated_keys_differ() {
        let a = NodeSecret::generate();
        let b = NodeSecret::generate();
        assert_ne!(a.as_bytes(), b.as_bytes());
        assert_ne!(a.public_key(), b.public_key());
    }

    #[test]
    fn text_round_trip() {
        let secret = NodeSecret::generate();
        let parsed: NodeSecret = secret.to_base64().parse().unwrap();
        assert_eq!(parsed.as_bytes(), secret.as_bytes());
        assert_eq!(parsed.public_key(), secret.public_key());
    }

    #[test]
    fn parsing_is_strict() {
        let text = NodeSecret::generate().to_base64();
        assert!(format!("{}\n", *text).parse::<NodeSecret>().is_err());
        assert!(text[..43].parse::<NodeSecret>().is_err());
    }

    #[test]
    fn key_files_in_the_encodings_windows_produces() {
        let secret = NodeSecret::generate();
        let text = format!("{}\r\n", *secret.to_base64());
        let read = |contents: &[u8]| NodeSecret::from_key_file(contents).map(|s| s.public_key());

        assert_eq!(
            read(text.as_bytes()),
            Ok(secret.public_key()),
            "UTF-8，带换行"
        );
        let with_bom = [&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat();
        assert_eq!(
            read(&with_bom),
            Ok(secret.public_key()),
            "UTF-8 BOM（记事本）"
        );
        // Windows PowerShell 5.1 的 `meshorad genkey > node.key` 写出来的就是这样
        let utf16: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain(text.encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        assert_eq!(read(&utf16), Ok(secret.public_key()), "UTF-16LE BOM");

        assert!(
            read(&utf16[..utf16.len() - 1]).is_err(),
            "UTF-16 截断了半个字符"
        );
        assert!(read(b"\xFF\x00garbage").is_err());
        assert!(read(b"").is_err());
    }

    #[test]
    fn debug_never_prints_the_secret() {
        let secret = NodeSecret::generate();
        let debug = format!("{secret:?}");
        assert!(!debug.contains(secret.to_base64().as_str()));
        assert!(debug.contains(&secret.public_key().to_string()));
    }
}
