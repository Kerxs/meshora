use std::error::Error;
use std::fmt;
use std::str::FromStr;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// 节点身份：一把 X25519 公钥。
///
/// 公钥即身份，不依赖 IP、主机名或账号。它同时也是这个节点在 WireGuard 数据面里的
/// 静态公钥 —— 认证和加密用的是同一对密钥，认证靠的是 Noise IK 握手而不是签名，见
/// [威胁模型 · 身份认证](https://kerxs.github.io/meshora/guide/threat-model#身份认证)。
///
/// 这里只是 32 字节的载体，**不校验它是不是曲线上的合法点**。点的合法性由握手时的
/// 密码库判定（比如 DH 结果全零就拒绝）。"自己不写密码学"也包括不在这里半吊子地做一遍。
///
/// 文本形式是标准 base64（44 个字符，带填充），和 `wg` 工具输出的格式一致，
/// 方便拿现成的 WireGuard 工具对照。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeKey([u8; 32]);

impl NodeKey {
    /// 从 32 字节构造。
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// 公钥的原始字节。
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for NodeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&STANDARD.encode(self.0))
    }
}

impl fmt::Debug for NodeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NodeKey({self})")
    }
}

/// 32 字节的标准 base64 恒为 44 个字符
const TEXT_LEN: usize = 44;

impl FromStr for NodeKey {
    type Err = ParseNodeKeyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // 先查长度：超长输入不值得解码
        if s.len() != TEXT_LEN {
            return Err(ParseNodeKeyError);
        }
        // STANDARD 要求规范编码：填充必须正确，末尾多出来的比特必须为零。
        // 所以每把公钥只有一种合法的文本形式，拿字符串比较公钥不会出错。
        let bytes = STANDARD.decode(s).map_err(|_| ParseNodeKeyError)?;
        let bytes: [u8; 32] = bytes.try_into().map_err(|_| ParseNodeKeyError)?;
        Ok(Self(bytes))
    }
}

/// 字符串不是一把合法编码的节点公钥。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseNodeKeyError;

impl fmt::Display for ParseNodeKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("不是合法的节点公钥：应为 32 字节的标准 base64 编码（44 个字符）")
    }
}

impl Error for ParseNodeKeyError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trip() {
        let bytes: [u8; 32] = std::array::from_fn(|i| i as u8);
        let key = NodeKey::from_bytes(bytes);
        let text = key.to_string();
        assert_eq!(text.len(), TEXT_LEN);
        assert_eq!(text.parse::<NodeKey>(), Ok(key));
        assert_eq!(key.as_bytes(), &bytes);
    }

    #[test]
    fn same_text_form_as_wg() {
        // wg-quick(8) 手册里的示例公钥：解析再格式化，必须原样回来
        let text = "xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=";
        let key: NodeKey = text.parse().unwrap();
        assert_eq!(key.to_string(), text);
    }

    #[test]
    fn all_zero_key_has_known_text() {
        let text = format!("{}=", "A".repeat(43));
        assert_eq!(NodeKey::from_bytes([0; 32]).to_string(), text);
    }

    #[test]
    fn rejects_wrong_length() {
        for text in [
            "",
            "AAAA",
            &"A".repeat(43),
            &format!("{}==", "A".repeat(43)),
        ] {
            assert_eq!(text.parse::<NodeKey>(), Err(ParseNodeKeyError), "{text:?}");
        }
    }

    #[test]
    fn rejects_non_base64() {
        let text = format!("{}!=", "A".repeat(42));
        assert_eq!(text.parse::<NodeKey>(), Err(ParseNodeKeyError));
        let text = format!(" {}=", "A".repeat(42));
        assert_eq!(text.parse::<NodeKey>(), Err(ParseNodeKeyError));
    }

    #[test]
    fn rejects_44_chars_that_decode_to_33_bytes() {
        // 长度对上了，但没有填充 —— 解出来是 33 字节
        assert_eq!("A".repeat(44).parse::<NodeKey>(), Err(ParseNodeKeyError));
    }

    #[test]
    fn rejects_non_canonical_trailing_bits() {
        // 43 个字符携带 258 比特，最后 2 比特必须为零。
        // 'E' = 0b000100 满足；'B' = 0b000001 不满足，是同一把公钥的非规范写法。
        let canonical = format!("{}E=", "A".repeat(42));
        let key: NodeKey = canonical.parse().unwrap();
        assert_eq!(key.to_string(), canonical);

        let sloppy = format!("{}B=", "A".repeat(42));
        assert_eq!(sloppy.parse::<NodeKey>(), Err(ParseNodeKeyError));
    }

    #[test]
    fn debug_shows_text_form() {
        let key = NodeKey::from_bytes([0; 32]);
        assert_eq!(format!("{key:?}"), format!("NodeKey({key})"));
    }
}
