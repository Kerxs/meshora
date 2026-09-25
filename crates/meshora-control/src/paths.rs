//! 选路：一个 peer 的候选端点、探测记录，以及该走哪条路。
//!
//! 纯逻辑，不碰 I/O，时间由调用方传进来 —— 规则都能直接测。
//!
//! - 候选端点有两个来源：协调服务公布的（NetMap 里），和从对方发来的 Ping 里学到的来源地址
//! - 每个候选每 [`PING_INTERVAL`] 探测一次；[`FRESH`] 之内收到过 Pong 才算通
//! - 选路：当前的直连还通就不换（免得在两条差不多的路之间来回跳）；否则挑 RTT 最低的；
//!   一条都不通就走中继

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use meshora_types::Path;

/// 每个候选多久探测一次。
pub const PING_INTERVAL: Duration = Duration::from_secs(3);
/// 多久之内收到过 Pong 才算通。比两轮探测稍长：丢一个 Pong 不至于切走，连丢两个就算断 ——
/// 直连断了，最多这么久就回落中继，这段时间里的报文会丢。
///
/// 取值参考 Tailscale（心跳 3 秒，信任 6.5 秒）。再短就容易在丢包的链路上来回切
pub const FRESH: Duration = Duration::from_millis(6500);
/// 学来的候选多久没通就忘掉 —— 前提是它回应过。
pub const LEARNED_TTL: Duration = Duration::from_secs(60);
/// 学来、但从没回应过的候选，多久就忘掉：只够探测两次（学到时一次，[`PING_INTERVAL`] 后一次，
/// 控制面按秒检查，第二次最多晚一秒）。
///
/// 学来的地址可能是伪造的：Ping 可以被录下来，换一个伪造的来源地址重放（它没有防重放）。
/// 要是每学到一个就在 [`LEARNED_TTL`] 里一直探测，一个重放的报文就能换来我们向任意地址
/// 发二十来个探测报文。真的对端会每一轮都再发 Ping 过来，学到的时刻随之刷新，不受影响
pub const UNVERIFIED_TTL: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Default)]
struct Candidate {
    /// 协调服务公布了它
    advertised: bool,
    /// 最近一次从这里学到它的时刻
    learned: Option<Instant>,
    last_ping: Option<Instant>,
    last_pong: Option<Instant>,
    rtt: Option<Duration>,
}

impl Candidate {
    fn fresh(&self, now: Instant) -> bool {
        self.last_pong
            .is_some_and(|pong| now.duration_since(pong) < FRESH)
    }

    /// 学来的候选还记不记得
    fn remembered(&self, now: Instant) -> bool {
        let ttl = if self.last_pong.is_some() {
            LEARNED_TTL
        } else {
            UNVERIFIED_TTL
        };
        self.learned
            .is_some_and(|learned| now.duration_since(learned) < ttl)
    }
}

/// 一个 peer 的候选端点和探测记录。
#[derive(Clone, Debug, Default)]
pub struct PeerPaths {
    candidates: BTreeMap<SocketAddr, Candidate>,
}

impl PeerPaths {
    /// 协调服务公布的端点（完整列表）。不在列表里的公布端点被移除，学来的不受影响。
    pub fn set_advertised(&mut self, endpoints: &[SocketAddr]) {
        for candidate in self.candidates.values_mut() {
            candidate.advertised = false;
        }
        for addr in endpoints {
            self.candidates.entry(*addr).or_default().advertised = true;
        }
        self.candidates
            .retain(|_, candidate| candidate.advertised || candidate.learned.is_some());
    }

    /// 学到一个候选：对方从这里发来过 Ping，或者协调服务转来的打洞请求里有它。
    ///
    /// 返回它是不是第一次出现 —— 第一次出现的应当马上探测，不必等下一轮。
    pub fn learn(&mut self, addr: SocketAddr, now: Instant) -> bool {
        let candidate = self.candidates.entry(addr).or_default();
        let new = candidate.last_ping.is_none();
        candidate.learned = Some(now);
        new
    }

    /// 让这个候选在下一次 [`due_pings`](Self::due_pings) 时立刻被探测（打洞对时要的就是"现在"）。
    pub fn ping_now(&mut self, addr: SocketAddr) {
        if let Some(candidate) = self.candidates.get_mut(&addr) {
            candidate.last_ping = None;
        }
    }

    /// 到时候该探测的候选，同时把它们记为"刚探测过"。顺带忘掉过期的学来候选。
    pub fn due_pings(&mut self, now: Instant) -> Vec<SocketAddr> {
        self.candidates.retain(|_, candidate| {
            candidate.advertised || candidate.fresh(now) || candidate.remembered(now)
        });
        let mut due = Vec::new();
        for (addr, candidate) in &mut self.candidates {
            let is_due = candidate
                .last_ping
                .is_none_or(|ping| now.duration_since(ping) >= PING_INTERVAL);
            if is_due {
                candidate.last_ping = Some(now);
                due.push(*addr);
            }
        }
        due
    }

    /// 收到了发往 `addr` 的 Ping 的回应。
    ///
    /// 注意是**发往**的地址，不是 Pong 的来源地址：来源地址可以伪造，
    /// 而能回应我们这次 Ping 的只有持有对方私钥的人。
    pub fn on_pong(&mut self, addr: SocketAddr, rtt: Duration, now: Instant) {
        if let Some(candidate) = self.candidates.get_mut(&addr) {
            candidate.last_pong = Some(now);
            candidate.rtt = Some(rtt);
        }
    }

    /// 有没有一条通的直连。
    pub fn has_fresh(&self, now: Instant) -> bool {
        self.candidates.values().any(|c| c.fresh(now))
    }

    /// 该走哪条路：当前的直连还通就不换；否则挑 RTT 最低的通路；都不通就走中继。
    pub fn choose(&self, current: Option<Path>, relay: Option<Path>, now: Instant) -> Option<Path> {
        if let Some(Path::Direct(addr)) = current
            && self.candidates.get(&addr).is_some_and(|c| c.fresh(now))
        {
            return current;
        }
        self.candidates
            .iter()
            .filter(|(_, c)| c.fresh(now))
            .min_by_key(|(_, c)| c.rtt.unwrap_or(Duration::MAX))
            .map(|(addr, _)| Path::Direct(*addr))
            .or(relay)
    }
}

#[cfg(test)]
mod tests {
    use meshora_types::NodeKey;

    use super::*;

    fn addr(last: u8) -> SocketAddr {
        SocketAddr::from(([198, 51, 100, last], 41641))
    }

    fn relay() -> Option<Path> {
        Some(Path::Relay {
            relay: NodeKey::from_bytes([9; 32]),
            addr: SocketAddr::from(([203, 0, 113, 1], 443)),
        })
    }

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn relay_first_until_a_direct_path_answers() {
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        paths.set_advertised(&[addr(1), addr(2)]);

        assert_eq!(paths.choose(None, relay(), now), relay());
        assert_eq!(paths.due_pings(now), [addr(1), addr(2)]);
        // 刚探测过，不重复
        assert!(paths.due_pings(now + MS).is_empty());

        paths.on_pong(addr(2), 30 * MS, now + 30 * MS);
        assert_eq!(
            paths.choose(relay(), relay(), now + 30 * MS),
            Some(Path::Direct(addr(2)))
        );
    }

    #[test]
    fn pings_repeat_every_interval() {
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        paths.set_advertised(&[addr(1)]);
        assert_eq!(paths.due_pings(now), [addr(1)]);
        assert!(paths.due_pings(now + PING_INTERVAL - MS).is_empty());
        assert_eq!(paths.due_pings(now + PING_INTERVAL), [addr(1)]);
    }

    #[test]
    fn lowest_rtt_wins_but_a_working_current_path_is_kept() {
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        paths.set_advertised(&[addr(1), addr(2)]);
        paths.on_pong(addr(1), 80 * MS, now);
        paths.on_pong(addr(2), 20 * MS, now);
        assert_eq!(paths.choose(None, None, now), Some(Path::Direct(addr(2))));

        // 已经在走 addr(1) 而且它还通：不为了更低的 RTT 换来换去
        let current = Some(Path::Direct(addr(1)));
        assert_eq!(paths.choose(current, None, now), current);
    }

    #[test]
    fn falls_back_to_relay_when_direct_goes_quiet() {
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        paths.set_advertised(&[addr(1)]);
        paths.on_pong(addr(1), 20 * MS, now);
        let current = Some(Path::Direct(addr(1)));
        assert_eq!(paths.choose(current, relay(), now + FRESH - MS), current);
        assert_eq!(paths.choose(current, relay(), now + FRESH), relay());
        assert!(!paths.has_fresh(now + FRESH));
    }

    #[test]
    fn pong_for_an_unknown_address_changes_nothing() {
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        paths.on_pong(addr(7), 10 * MS, now);
        assert_eq!(paths.choose(None, relay(), now), relay());
    }

    #[test]
    fn an_unanswered_learned_candidate_gets_just_two_pings() {
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        assert!(paths.learn(addr(5), now), "第一次学到");
        assert_eq!(paths.due_pings(now), [addr(5)]);
        assert!(!paths.learn(addr(5), now + MS), "探测过了就不算新");

        // 从最后一次学到它（now + 1ms）起算：再探测一次，然后忘掉
        assert_eq!(paths.due_pings(now + PING_INTERVAL), [addr(5)]);
        let forgotten = now + MS + UNVERIFIED_TTL;
        assert!(paths.due_pings(forgotten - MS).is_empty(), "两次探测之间");
        assert!(paths.due_pings(forgotten).is_empty());
        assert!(paths.learn(addr(5), forgotten), "忘掉之后再学到又是新的");
    }

    #[test]
    fn a_learned_candidate_that_answered_is_kept_longer() {
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        paths.learn(addr(5), now);
        paths.due_pings(now);
        paths.on_pong(addr(5), 10 * MS, now);

        // 早就不"通"了，但回应过，所以按 LEARNED_TTL 记着，照常探测
        let later = now + LEARNED_TTL - MS;
        assert!(!paths.has_fresh(later));
        assert_eq!(paths.due_pings(later), [addr(5)]);
        assert!(
            paths
                .due_pings(now + LEARNED_TTL + PING_INTERVAL)
                .is_empty()
        );
    }

    #[test]
    fn readvertising_drops_stale_advertised_but_keeps_learned() {
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        paths.set_advertised(&[addr(1), addr(2)]);
        paths.learn(addr(3), now);
        paths.set_advertised(&[addr(2)]);
        assert_eq!(paths.due_pings(now), [addr(2), addr(3)]);
    }

    #[test]
    fn ping_now_makes_a_candidate_due_immediately() {
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        paths.set_advertised(&[addr(1)]);
        paths.due_pings(now);
        paths.ping_now(addr(1));
        assert_eq!(paths.due_pings(now + MS), [addr(1)]);
    }
}
