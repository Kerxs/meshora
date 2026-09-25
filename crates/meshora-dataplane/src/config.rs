use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::net::IpAddr;
use std::num::NonZeroU16;

use ipnet::IpNet;
use meshora_types::NodeKey;

/// 交给数据面的一个 peer。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerConfig {
    /// peer 的身份，也就是它的 WireGuard 静态公钥。
    pub key: NodeKey,
    /// WireGuard 的 cryptokey routing：目的地址落在这些网段里的报文发给这个 peer；
    /// 从它那里收到的报文，源地址也必须落在这些网段里，否则丢弃。
    ///
    /// 网段必须是规范形式（主机位全为零）。可以为空：能握手，但不收发任何 IP 报文。
    pub allowed_ips: Vec<IpNet>,
    /// WireGuard 的 persistent keepalive 间隔（秒），`None` 表示不发。
    ///
    /// NAT 后面的 peer 靠它维持映射，具体取值由控制面按网络状况决定。
    pub keepalive: Option<NonZeroU16>,
}

/// 一份校验过的 peer 集合，[`DataPlane::apply`](crate::DataPlane::apply) 的输入。
///
/// 只能通过 [`PeerSet::new`] 构造，构造时检查：
///
/// - 同一个 peer 不能出现两次
/// - 网段必须是规范形式。`10.0.0.1/24` 这种主机位非零的写法多半是笔误，
///   直接拒绝，不替调用方悄悄截断
/// - **同一个网段只能分给一个 peer。** `wg set` 遇到这种情况会把网段从原来的 peer
///   身上静默挪走；这里选择拒绝 —— 静默挪走意味着一条错误配置就能把别人的流量引过来
///
/// 前缀长度不同的重叠是允许的（比如 `10.0.0.0/8` 给 A、`10.1.0.0/16` 给 B），
/// 按最长前缀匹配，见 [`route`](Self::route)。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PeerSet {
    peers: BTreeMap<NodeKey, PeerConfig>,
}

impl PeerSet {
    /// 校验并构造。遇到第一处问题就返回。
    pub fn new(peers: impl IntoIterator<Item = PeerConfig>) -> Result<Self, ConfigError> {
        let mut by_key = BTreeMap::new();
        let mut owners: BTreeMap<IpNet, NodeKey> = BTreeMap::new();
        for peer in peers {
            // 先查 peer 重复：否则重复 peer 的网段会先撞上自己，报出一个误导人的网段冲突
            if by_key.contains_key(&peer.key) {
                return Err(ConfigError::DuplicatePeer(peer.key));
            }
            for &prefix in &peer.allowed_ips {
                if prefix != prefix.trunc() {
                    return Err(ConfigError::NonCanonicalPrefix {
                        peer: peer.key,
                        prefix,
                    });
                }
                if let Some(&first) = owners.get(&prefix) {
                    return Err(ConfigError::DuplicatePrefix {
                        prefix,
                        first,
                        second: peer.key,
                    });
                }
                owners.insert(prefix, peer.key);
            }
            by_key.insert(peer.key, peer);
        }
        Ok(Self { peers: by_key })
    }

    /// 按身份取一个 peer。
    pub fn get(&self, key: &NodeKey) -> Option<&PeerConfig> {
        self.peers.get(key)
    }

    /// 所有 peer，按公钥排序。
    pub fn iter(&self) -> impl Iterator<Item = &PeerConfig> {
        self.peers.values()
    }

    /// peer 的个数。
    pub fn len(&self) -> usize {
        self.peers.len()
    }

    /// 是否一个 peer 都没有。
    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    /// 目的地址为 `dst` 的报文该发给哪个 peer：最长前缀匹配，没有网段覆盖它时返回 `None`。
    ///
    /// 同一个网段不会分给两个 peer，所以最长的匹配前缀是唯一的。
    ///
    /// 这里是线性扫描，够用来把语义钉死；M1 的数据面在热路径上要换成前缀树。
    pub fn route(&self, dst: IpAddr) -> Option<&NodeKey> {
        self.peers
            .values()
            .flat_map(|peer| peer.allowed_ips.iter().map(move |net| (net, &peer.key)))
            .filter(|(net, _)| net.contains(&dst))
            .max_by_key(|(net, _)| net.prefix_len())
            .map(|(_, key)| key)
    }
}

/// [`PeerSet::new`] 拒绝一份配置的原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// 同一个 peer 出现了不止一次。
    DuplicatePeer(NodeKey),
    /// 网段的主机位不为零，比如 `10.0.0.1/24`。
    NonCanonicalPrefix {
        /// 出问题的 peer。
        peer: NodeKey,
        /// 写错的网段。
        prefix: IpNet,
    },
    /// 同一个网段分给了两个 peer，或者在同一个 peer 里出现了两次。
    DuplicatePrefix {
        /// 重复的网段。
        prefix: IpNet,
        /// 先拿到这个网段的 peer。
        first: NodeKey,
        /// 后来又要这个网段的 peer。
        second: NodeKey,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicatePeer(key) => write!(f, "peer {key} 出现了不止一次"),
            Self::NonCanonicalPrefix { peer, prefix } => write!(
                f,
                "peer {peer} 的网段 {prefix} 主机位不为零，是想写 {} 吗",
                prefix.trunc()
            ),
            Self::DuplicatePrefix {
                prefix,
                first,
                second,
            } if first == second => {
                write!(f, "网段 {prefix} 在 peer {first} 里出现了两次")
            }
            Self::DuplicatePrefix {
                prefix,
                first,
                second,
            } => write!(f, "网段 {prefix} 同时分给了 peer {first} 和 {second}"),
        }
    }
}

impl Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u8) -> NodeKey {
        NodeKey::from_bytes([n; 32])
    }

    fn peer(n: u8, allowed_ips: &[&str]) -> PeerConfig {
        PeerConfig {
            key: key(n),
            allowed_ips: allowed_ips.iter().map(|s| s.parse().unwrap()).collect(),
            keepalive: None,
        }
    }

    fn net(s: &str) -> IpNet {
        s.parse().unwrap()
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn empty_set_is_valid() {
        let set = PeerSet::new([]).unwrap();
        assert!(set.is_empty());
        assert_eq!(set.route(ip("10.0.0.1")), None);
    }

    #[test]
    fn valid_set_keeps_every_peer() {
        let a = peer(1, &["100.64.0.1/32", "fd7a::1/128"]);
        let b = peer(2, &["100.64.0.2/32"]);
        let empty = peer(3, &[]);
        let set = PeerSet::new([b.clone(), a.clone(), empty.clone()]).unwrap();
        assert_eq!(set.len(), 3);
        assert_eq!(set.get(&key(1)), Some(&a));
        assert_eq!(set.get(&key(4)), None);
        // 按公钥排序，和传入顺序无关
        let order: Vec<_> = set.iter().collect();
        assert_eq!(order, [&a, &b, &empty]);
    }

    #[test]
    fn rejects_duplicate_peer() {
        let result = PeerSet::new([peer(1, &["10.0.0.0/8"]), peer(1, &["10.0.0.0/8"])]);
        assert_eq!(result, Err(ConfigError::DuplicatePeer(key(1))));
    }

    #[test]
    fn rejects_non_canonical_prefix() {
        let result = PeerSet::new([peer(1, &["10.0.0.1/24"])]);
        assert_eq!(
            result,
            Err(ConfigError::NonCanonicalPrefix {
                peer: key(1),
                prefix: net("10.0.0.1/24"),
            })
        );
        let message = result.unwrap_err().to_string();
        assert!(message.contains("是想写 10.0.0.0/24 吗"), "{message}");

        // 主机路由和默认路由本身就是规范形式
        assert!(PeerSet::new([peer(1, &["10.0.0.1/32", "0.0.0.0/0", "::/0"])]).is_ok());
    }

    #[test]
    fn rejects_same_prefix_on_two_peers() {
        let result = PeerSet::new([peer(1, &["10.1.0.0/16"]), peer(2, &["10.1.0.0/16"])]);
        assert_eq!(
            result,
            Err(ConfigError::DuplicatePrefix {
                prefix: net("10.1.0.0/16"),
                first: key(1),
                second: key(2),
            })
        );
    }

    #[test]
    fn rejects_same_prefix_twice_in_one_peer() {
        let result = PeerSet::new([peer(1, &["10.1.0.0/16", "10.1.0.0/16"])]);
        let err = result.unwrap_err();
        assert_eq!(
            err,
            ConfigError::DuplicatePrefix {
                prefix: net("10.1.0.0/16"),
                first: key(1),
                second: key(1),
            }
        );
        assert!(err.to_string().contains("出现了两次"), "{err}");
    }

    #[test]
    fn overlapping_prefixes_route_by_longest_match() {
        let set = PeerSet::new([
            peer(1, &["10.0.0.0/8", "fd00::/8"]),
            peer(2, &["10.1.0.0/16"]),
            peer(3, &["10.1.2.3/32"]),
        ])
        .unwrap();
        assert_eq!(set.route(ip("10.2.0.1")), Some(&key(1)));
        assert_eq!(set.route(ip("10.1.9.9")), Some(&key(2)));
        assert_eq!(set.route(ip("10.1.2.3")), Some(&key(3)));
        assert_eq!(set.route(ip("fd00::1")), Some(&key(1)));
        assert_eq!(set.route(ip("192.168.1.1")), None);
    }

    #[test]
    fn default_route_loses_to_anything_more_specific() {
        // 出口节点（Gateway）拿默认路由，其余流量照旧走各自的 peer
        let set =
            PeerSet::new([peer(1, &["100.64.0.1/32"]), peer(9, &["0.0.0.0/0", "::/0"])]).unwrap();
        assert_eq!(set.route(ip("100.64.0.1")), Some(&key(1)));
        assert_eq!(set.route(ip("8.8.8.8")), Some(&key(9)));
        assert_eq!(set.route(ip("2001:db8::1")), Some(&key(9)));
    }

    #[test]
    fn ipv4_prefix_never_matches_ipv6_address() {
        let set = PeerSet::new([peer(1, &["0.0.0.0/0"])]).unwrap();
        assert_eq!(set.route(ip("::ffff:10.0.0.1")), None);
        assert_eq!(set.route(ip("fd00::1")), None);
    }
}
