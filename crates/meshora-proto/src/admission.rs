//! 服务端的准入：同一个来源同时在握手的连接数有上限。
//!
//! 握手完成之前不知道对方是谁，只知道它从哪来。一个来源开一大堆连接、每条都磨蹭到超时，
//! 就能占满服务端的文件描述符，让别人都连不上。正常的节点握手只要几毫秒，同一个来源同时在握手的
//! 连接很少超过一两条 —— 哪怕很多节点躲在同一个 NAT 后面 —— 所以这个上限碰不到正常节点。
//!
//! "来源"对 IPv4 是地址本身；对 IPv6 是它所在的 /64：一台机器往往就分到一整个 /64，
//! 按单个地址算等于没限。
//!
//! 来源很多（比如僵尸网络）的时候，这挡不住占满文件描述符。它只让单个来源做不到。

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::{Arc, Mutex, MutexGuard};

/// 准入表。服务端每接受一条连接就先 [`admit`](Admission::admit)，确认对方身份之后放掉凭证。
pub struct Admission {
    per_source: usize,
    pending: Mutex<HashMap<IpAddr, usize>>,
}

impl Admission {
    /// 每个来源同时最多 `per_source` 条连接在握手。
    pub fn new(per_source: usize) -> Arc<Self> {
        Arc::new(Self {
            per_source,
            pending: Mutex::new(HashMap::new()),
        })
    }

    /// 来自 `ip` 的一条连接要开始握手。这个来源名额满了就返回 `None`，调用方应当直接断开。
    ///
    /// 返回的凭证在丢掉时归还名额：握手失败、超时、确认了身份，都一样。
    pub fn admit(self: &Arc<Self>, ip: IpAddr) -> Option<Pending> {
        let source = source_of(ip);
        let mut pending = self.pending();
        let count = pending.entry(source).or_insert(0);
        if *count >= self.per_source {
            return None;
        }
        *count += 1;
        Some(Pending {
            admission: Arc::clone(self),
            source,
        })
    }

    fn pending(&self) -> MutexGuard<'_, HashMap<IpAddr, usize>> {
        self.pending.lock().expect("准入表在持锁时 panic 过")
    }
}

/// 一条正在握手的连接占着的名额，丢掉时归还。
pub struct Pending {
    admission: Arc<Admission>,
    source: IpAddr,
}

impl Drop for Pending {
    fn drop(&mut self) {
        let mut pending = self.admission.pending();
        if let Some(count) = pending.get_mut(&self.source) {
            *count -= 1;
            if *count == 0 {
                pending.remove(&self.source);
            }
        }
    }
}

/// IPv4 按地址算；IPv6 按 /64 算；IPv4 映射成的 IPv6 地址当 IPv4 算
fn source_of(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(_) => ip,
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => {
                let prefix = u128::from(v6) & !u128::from(u64::MAX);
                IpAddr::V6(Ipv6Addr::from(prefix))
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn a_source_gets_a_fixed_number_of_slots() {
        let admission = Admission::new(2);
        let first = admission.admit(ip("192.0.2.1"));
        let second = admission.admit(ip("192.0.2.1"));
        assert!(first.is_some() && second.is_some());
        assert!(admission.admit(ip("192.0.2.1")).is_none(), "名额满了");
        assert!(
            admission.admit(ip("192.0.2.2")).is_some(),
            "别的来源不受影响"
        );

        drop(first);
        assert!(admission.admit(ip("192.0.2.1")).is_some(), "归还之后又有了");
    }

    #[test]
    fn ipv6_counts_per_slash_64() {
        let admission = Admission::new(1);
        let _held = admission.admit(ip("2001:db8:1:2::1")).unwrap();
        assert!(
            admission.admit(ip("2001:db8:1:2::ffff")).is_none(),
            "同一个 /64"
        );
        assert!(
            admission.admit(ip("2001:db8:1:3::1")).is_some(),
            "另一个 /64"
        );
    }

    #[test]
    fn ipv4_mapped_addresses_count_as_ipv4() {
        let admission = Admission::new(1);
        let _held = admission.admit(ip("192.0.2.7")).unwrap();
        assert!(admission.admit(ip("::ffff:192.0.2.7")).is_none());
    }

    #[test]
    fn released_sources_are_forgotten() {
        let admission = Admission::new(1);
        drop(admission.admit(ip("192.0.2.1")));
        assert!(admission.pending().is_empty());
    }
}
