//! 选路：一个 peer 的候选端点、探测记录，以及该走哪条路。
//!
//! 纯逻辑，不碰 I/O，时间由调用方传进来 —— 规则都能直接测。
//!
//! - 候选端点有两个来源：协调服务公布的（NetMap 里），和从对方发来的 Ping 里学到的来源地址
//! - 每个候选每 [`PING_INTERVAL`] 探测一次；**正在用的那条**每 [`ACTIVE_PING_INTERVAL`] 一次
//! - 通不通：[`FRESH`] 之内收到过 Pong，而且没有连着 [`MAX_MISSES`] 次在 [`PONG_WAIT`] 内没回应。
//!   正在用的直连断了，大约两三秒就能发现、回落中继
//! - 延迟看两样：平滑后的往返时间和抖动（算法同 TCP 估算重传超时，RFC 6298）。
//!   打分是"往返时间 + 4 × 抖动 + 丢包罚分"：对联机游戏，忽快忽慢、时不时丢包，
//!   都比稳定地慢一点更难受。丢包率是探测的丢失比例，平滑过；每 1% 记 [`LOSS_PENALTY_PER_PERCENT`]。
//!   只算一条通着的路偶尔丢的：连丢到判定断开的是断线，不算丢包
//! - 中继那条路也探测（经中继发 Ping），同样记往返时间、抖动、丢包。网里有几个中继就探测几条，
//!   各记各的：一个中继挂了（或者只是本机、对方连不上它），探测不通，自然换到别的中继
//! - 选路：通着的直连和通着的中继放在一起按分数比。中继要**明显**比直连好才选它 ——
//!   它占着第三方的带宽，而且多绕一段；当前的路还通，别的路也要明显更好才换（免得来回跳）。
//!   一条直连都不通，就走最好的那个通着的中继；中继也都没测通，就留在当前的中继上，
//!   没有当前的就用排第一的。直连要测够 [`SETTLED_SAMPLES`] 次才和中继比分数 —— 头几个样本不准
//! - 本机换了网络，所有直连一律作废：先走中继，重新探测，通了再切回来

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use meshora_types::Path;

/// 每个候选多久探测一次。
pub const PING_INTERVAL: Duration = Duration::from_secs(3);
/// 正在用的直连多久探测一次。它断了要尽快发现：游戏的报文正往那边发。
pub const ACTIVE_PING_INTERVAL: Duration = Duration::from_secs(1);
/// 一个 Ping 发出去多久没回应，就算丢了一次。
pub const PONG_WAIT: Duration = Duration::from_secs(1);
/// 连着丢几次就算不通。一次不算：UDP 丢一个包很平常，丢了就立刻补探一次。
pub const MAX_MISSES: u32 = 2;
/// 多久之内收到过 Pong 才算通。这是上限：没有新的探测结果（比如没人发 Ping）时，最多信这么久。
///
/// 取值参考 Tailscale（心跳 3 秒，信任 6.5 秒）
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
/// 丢包率每 1%，分数加这么多。丢 10% 的路相当于慢了 50ms：游戏里丢一个包就是一次瞬移或者卡顿。
pub const LOSS_PENALTY_PER_PERCENT: Duration = Duration::from_millis(5);
/// 丢包率的平滑：每次探测的结果（丢或没丢）占这么多分之一
const LOSS_SMOOTHING: u32 = 8;
/// 当前的直连还通时，别的直连的分数要比它好这么多才换过去……
const SWITCH_MARGIN: Duration = Duration::from_millis(10);
/// ……而且至少好这么多分之一（当前分数的 1/4）。两个条件都要满足
const SWITCH_FRACTION: u32 = 4;
/// 一条直连测过这么多次，才拿它的分数和中继比。
///
/// 第一个样本把抖动设成往返时间的一半（RFC 6298 的初值），分数就是往返时间的三倍；而经过 NAT 的
/// 第一个报文常常慢几毫秒（ARP、conntrack）。只测了一次的直连要是就这样输给测过很多次的中继，
/// 没在用的直连 3 秒才探测一次，分数要半分钟以上才降得下来 —— e2e 里撞出来过：
/// 一边早就直连了，另一边在中继上待满了 30 秒
pub const SETTLED_SAMPLES: u32 = 5;

#[derive(Clone, Debug, Default)]
struct Candidate {
    /// 协调服务公布了它
    advertised: bool,
    /// 最近一次从这里学到它的时刻
    learned: Option<Instant>,
    last_ping: Option<Instant>,
    last_pong: Option<Instant>,
    /// 发出去、还没判定丢没丢的那个 Ping 的时刻
    awaiting: Option<Instant>,
    /// 连着丢了几次，收到 Pong 就清零
    misses: u32,
    /// 平滑后的往返时间
    srtt: Option<Duration>,
    /// 往返时间的平均偏差，也就是抖动
    rttvar: Duration,
    /// 记过几个往返时间样本
    samples: u32,
    /// 平滑后的丢包率，万分之几（0 到 10000）
    loss: u32,
}

impl Candidate {
    fn fresh(&self, now: Instant) -> bool {
        self.misses < MAX_MISSES
            && self
                .last_pong
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

    /// 选路用的分数，越小越好：往返时间加上四倍抖动，再加丢包罚分
    fn score(&self) -> Duration {
        let loss = LOSS_PENALTY_PER_PERCENT * self.loss / 100;
        self.srtt.unwrap_or(Duration::MAX / 8) + 4 * self.rttvar + loss
    }

    /// 记一次探测的结果：丢了还是回来了
    fn record_loss(&mut self, lost: bool) {
        let sample = if lost { 10_000 } else { 0 };
        self.loss = (self.loss * (LOSS_SMOOTHING - 1) + sample) / LOSS_SMOOTHING;
    }

    /// 记一个往返时间样本（RFC 6298 的平滑办法）。
    ///
    /// 头几个样本改用算术平均（第 n 个占 1/n），到了 RFC 的权重（1/8、1/4）再按它来。
    /// 不然第一个样本分量太重：经 NAT 的第一个报文慢几毫秒，要十几二十个样本才冲得淡
    fn sample(&mut self, rtt: Duration) {
        self.samples = self.samples.saturating_add(1);
        match self.srtt {
            None => {
                self.srtt = Some(rtt);
                self.rttvar = rtt / 2;
            }
            Some(srtt) => {
                let deviation = srtt.abs_diff(rtt);
                let var_weight = self.samples.min(4);
                let rtt_weight = self.samples.min(8);
                self.rttvar = (self.rttvar * (var_weight - 1) + deviation) / var_weight;
                self.srtt = Some((srtt * (rtt_weight - 1) + rtt) / rtt_weight);
            }
        }
    }
}

impl Candidate {
    /// 到没到探测的时候。顺带结算上一个 Ping：等够了还没回就记一次丢失。
    /// 到了的话记为"刚探测过"
    fn poll_due(&mut self, now: Instant, interval: Duration) -> bool {
        if let Some(sent) = self.awaiting
            && now.duration_since(sent) >= PONG_WAIT
        {
            // 先不记进丢包率：这次丢失是"偶尔丢一个"还是"整条路断了"，要等下文。
            // 见 pong()
            self.awaiting = None;
            self.misses = self.misses.saturating_add(1);
        }
        let working = self
            .last_pong
            .is_some_and(|pong| now.duration_since(pong) < FRESH);
        // 一条通着的路刚丢了一次：马上补探，别等下一轮
        let retry = working && (1..MAX_MISSES).contains(&self.misses);
        let is_due = retry
            || self
                .last_ping
                .is_none_or(|ping| now.duration_since(ping) >= interval);
        if is_due && self.awaiting.is_none() {
            self.last_ping = Some(now);
            self.awaiting = Some(now);
            return true;
        }
        false
    }

    /// 收到了回应
    fn pong(&mut self, rtt: Duration, now: Instant) {
        // 丢过几次、路又没断（没到 MAX_MISSES）：那几次是偶尔丢包，记进丢包率。
        // 到了 MAX_MISSES 的是断线，由"通不通"管，不算丢包 —— 不然断了几十秒的直连恢复之后，
        // 丢包率高得吓人、要一两分钟才降下来，这期间一直输给中继
        if self.misses < MAX_MISSES {
            for _ in 0..self.misses {
                self.record_loss(true);
            }
        }
        // 还在等的那个 Ping 按时回来了，记一次"没丢"。迟到的 Pong 不再记：一次探测只记一个结果
        if self.awaiting.take().is_some() {
            self.record_loss(false);
        }
        self.last_pong = Some(now);
        self.misses = 0;
        self.sample(rtt);
    }

    /// 此前的探测结果作废，马上重新探测
    fn invalidate(&mut self) {
        self.misses = MAX_MISSES;
        self.awaiting = None;
        self.last_ping = None;
    }

    fn loss_fraction(&self) -> Option<f32> {
        self.srtt.map(|_| self.loss as f32 / 10_000.0)
    }
}

/// 一个 peer 的候选端点和探测记录。
#[derive(Clone, Debug, Default)]
pub struct PeerPaths {
    candidates: BTreeMap<SocketAddr, Candidate>,
    /// 经各个中继到它的路，键是 [`Path::Relay`]
    relays: HashMap<Path, Candidate>,
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
            // 正在等的那个 Ping 不算丢：这一次是另起炉灶
            candidate.awaiting = None;
        }
    }

    /// 本机换了网络：此前的探测结果一律作废，所有候选马上重新探测。
    ///
    /// 直连是从旧网络上打通的，多半已经不通了。与其等它们一个个超时，不如立刻当作不通，
    /// 先走中继，重新探测通了再切回来。
    pub fn network_changed(&mut self) {
        for candidate in self.candidates.values_mut() {
            candidate.invalidate();
        }
        // 到中继的连接也是在旧网络上建的，测到的数也不作数了
        for relay in self.relays.values_mut() {
            relay.invalidate();
        }
    }

    /// 协调服务给的中继变了：不在 `relays` 里的，探测记录一并忘掉。
    pub fn retain_relays(&mut self, relays: &[Path]) {
        self.relays.retain(|path, _| relays.contains(path));
    }

    /// 到时候该探测的候选，同时把它们记为"刚探测过"。顺带忘掉过期的学来候选。
    ///
    /// `active` 是正在用的直连：它探测得更勤，丢了一次马上补探。
    pub fn due_pings(&mut self, now: Instant, active: Option<SocketAddr>) -> Vec<SocketAddr> {
        self.candidates.retain(|_, candidate| {
            candidate.advertised || candidate.fresh(now) || candidate.remembered(now)
        });
        let mut due = Vec::new();
        for (addr, candidate) in &mut self.candidates {
            let interval = if Some(*addr) == active {
                ACTIVE_PING_INTERVAL
            } else {
                PING_INTERVAL
            };
            if candidate.poll_due(now, interval) {
                due.push(*addr);
            }
        }
        due
    }

    /// 通着的直连里分数最好的那个。
    ///
    /// 正在走中继、却有通着的直连时（比如直连刚测出来、分数还没稳），控制面把它当"正在用的"那样
    /// 每秒探测：要回到直连，得先把它测准，别按 3 秒一轮慢慢等
    pub fn best_direct(&self, now: Instant) -> Option<SocketAddr> {
        self.candidates
            .iter()
            .filter(|(_, c)| c.fresh(now))
            .min_by_key(|(_, c)| c.score())
            .map(|(addr, _)| *addr)
    }

    /// 该不该经中继 `relay` 探测一次。`active` 表示正在走这个中继：和正在用的直连一样探测得更勤。
    pub fn relay_due(&mut self, relay: Path, now: Instant, active: bool) -> bool {
        let interval = if active {
            ACTIVE_PING_INTERVAL
        } else {
            PING_INTERVAL
        };
        self.relays
            .entry(relay)
            .or_default()
            .poll_due(now, interval)
    }

    /// 收到了发往 `addr` 的 Ping 的回应。
    ///
    /// 注意是**发往**的地址，不是 Pong 的来源地址：来源地址可以伪造，
    /// 而能回应我们这次 Ping 的只有持有对方私钥的人。
    pub fn on_pong(&mut self, addr: SocketAddr, rtt: Duration, now: Instant) {
        if let Some(candidate) = self.candidates.get_mut(&addr) {
            candidate.pong(rtt, now);
        }
    }

    /// 收到了经中继 `relay` 发出的 Ping 的回应。
    pub fn on_relay_pong(&mut self, relay: Path, rtt: Duration, now: Instant) {
        if let Some(candidate) = self.relays.get_mut(&relay) {
            candidate.pong(rtt, now);
        }
    }

    /// 这个候选平滑后的往返时间。没探测通过为 `None`。
    pub fn rtt(&self, addr: SocketAddr) -> Option<Duration> {
        self.candidates.get(&addr).and_then(|c| c.srtt)
    }

    /// 这个候选平滑后的丢包率（0.0 到 1.0）。没探测通过为 `None`。
    pub fn loss(&self, addr: SocketAddr) -> Option<f32> {
        self.candidates
            .get(&addr)
            .and_then(Candidate::loss_fraction)
    }

    /// 这个候选往返时间的抖动（平均偏差）。没探测通过为 `None`。
    pub fn jitter(&self, addr: SocketAddr) -> Option<Duration> {
        self.candidates
            .get(&addr)
            .and_then(|c| c.srtt.map(|_| c.rttvar))
    }

    /// 经中继 `relay` 那条路平滑后的往返时间、抖动、丢包率。没探测通过为 `None`。
    pub fn relay_quality(&self, relay: Path) -> Option<(Duration, Duration, f32)> {
        let relay = self.relays.get(&relay)?;
        relay
            .srtt
            .map(|srtt| (srtt, relay.rttvar, relay.loss as f32 / 10_000.0))
    }

    /// 有没有一条通的直连。
    pub fn has_fresh(&self, now: Instant) -> bool {
        self.candidates.values().any(|c| c.fresh(now))
    }

    /// 该走哪条路。
    ///
    /// - 通着的直连里挑分数最好的；通着的中继也参加比较，但要**明显**更好才选它
    /// - 当前的路还通，别的路也要明显更好才换
    /// - 一条直连都不通，走通着的中继里最好的；一个都没测通，留在当前的中继上，没有就用排第一的
    ///
    /// `relays` 是协调服务给的中继，按它排的顺序（分数一样时靠前的优先）。
    pub fn choose(&self, current: Option<Path>, relays: &[Path], now: Instant) -> Option<Path> {
        let better = |challenger: Duration, incumbent: Duration| {
            challenger + SWITCH_MARGIN.max(incumbent / SWITCH_FRACTION) < incumbent
        };
        let best_direct = self
            .candidates
            .iter()
            .filter(|(_, c)| c.fresh(now))
            .min_by_key(|(_, c)| c.score())
            .map(|(addr, c)| (Path::Direct(*addr), c.score(), c.samples >= SETTLED_SAMPLES));
        let relay_score = |path: &Path| {
            self.relays
                .get(path)
                .filter(|c| c.fresh(now))
                .map(Candidate::score)
        };
        // min_by_key 遇到一样的取第一个：分数一样时按协调服务排的顺序
        let best_relay = relays
            .iter()
            .filter_map(|path| relay_score(path).map(|score| (*path, score)))
            .min_by_key(|(_, score)| *score);

        // 不考虑"当前走哪条"时的最佳：直连优先，中继要明显更好 —— 而且直连得先测够了才比
        let best = match (best_direct, best_relay) {
            (Some((_, direct, true)), Some(relay)) if better(relay.1, direct) => Some(relay),
            (Some((path, score, _)), _) => Some((path, score)),
            (None, relay) => relay,
        };

        let current_score = match current {
            Some(Path::Direct(addr)) => self
                .candidates
                .get(&addr)
                .filter(|c| c.fresh(now))
                .map(Candidate::score),
            Some(path @ Path::Relay { .. }) if relays.contains(&path) => relay_score(&path),
            _ => None,
        };
        match (current, current_score, best) {
            // 正在走中继、有了通着的直连，而中继又没有明显更好（best 已经这样比过了）：
            // 回到直连。这里不能再要求直连"明显更好" —— 两条路都很快时（比如直连 0.1ms、
            // 中继 0.5ms），直连永远好不出那么多，一旦落到中继就再也回不来
            (Some(Path::Relay { .. }), _, Some((path @ Path::Direct(_), _))) => Some(path),
            // 当前的路还通：别的路明显更好才换
            (Some(current), Some(ours), Some((path, score))) => {
                if path != current && better(score, ours) {
                    Some(path)
                } else {
                    Some(current)
                }
            }
            (_, _, Some((path, _))) => Some(path),
            // 什么都没测通（刚开始、或者全断了）：别在没测过的中继之间来回换
            (_, _, None) => current
                .filter(|path| matches!(path, Path::Relay { .. }) && relays.contains(path))
                .or_else(|| relays.first().copied()),
        }
    }
}

#[cfg(test)]
mod tests {
    use meshora_types::NodeKey;

    use super::*;

    fn addr(last: u8) -> SocketAddr {
        SocketAddr::from(([198, 51, 100, last], 41641))
    }

    /// 第 `n` 个中继
    fn relay_n(n: u8) -> Path {
        Path::Relay {
            relay: NodeKey::from_bytes([9 + n; 32]),
            addr: SocketAddr::from(([203, 0, 113, 1 + n], 443)),
        }
    }

    /// 只有一个中继时的那个
    fn relay() -> Option<Path> {
        Some(relay_n(0))
    }

    fn relays() -> Vec<Path> {
        vec![relay_n(0)]
    }

    const MS: Duration = Duration::from_millis(1);
    const SEC: Duration = Duration::from_secs(1);

    /// 一条已经通了、正在用的直连
    fn working(now: Instant) -> PeerPaths {
        let mut paths = PeerPaths::default();
        paths.set_advertised(&[addr(1)]);
        assert_eq!(paths.due_pings(now, None), [addr(1)]);
        paths.on_pong(addr(1), 20 * MS, now + 20 * MS);
        paths
    }

    #[test]
    fn relay_first_until_a_direct_path_answers() {
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        paths.set_advertised(&[addr(1), addr(2)]);

        assert_eq!(paths.choose(None, &relays(), now), relay());
        assert_eq!(paths.due_pings(now, None), [addr(1), addr(2)]);
        // 刚探测过，不重复
        assert!(paths.due_pings(now + MS, None).is_empty());

        paths.on_pong(addr(2), 30 * MS, now + 30 * MS);
        assert_eq!(
            paths.choose(relay(), &relays(), now + 30 * MS),
            Some(Path::Direct(addr(2)))
        );
    }

    #[test]
    fn pings_repeat_every_interval() {
        let now = Instant::now();
        let mut paths = working(now);
        assert!(paths.due_pings(now + PING_INTERVAL - MS, None).is_empty());
        assert_eq!(paths.due_pings(now + PING_INTERVAL, None), [addr(1)]);
    }

    #[test]
    fn the_path_in_use_is_pinged_every_second() {
        let now = Instant::now();
        let mut paths = working(now);
        let active = Some(addr(1));
        assert!(paths.due_pings(now + SEC - MS, active).is_empty());
        assert_eq!(paths.due_pings(now + SEC, active), [addr(1)]);
    }

    #[test]
    fn a_dead_path_in_use_is_noticed_within_seconds() {
        let now = Instant::now();
        let mut paths = working(now);
        let current = Some(Path::Direct(addr(1)));
        let active = Some(addr(1));
        // 1 秒：例行探测，从此再也没有回应
        assert_eq!(paths.due_pings(now + SEC, active), [addr(1)]);
        // 2 秒：丢了一次 —— 还算通，马上补探
        assert_eq!(paths.due_pings(now + 2 * SEC, active), [addr(1)]);
        assert_eq!(paths.choose(current, &relays(), now + 2 * SEC), current);
        // 3 秒：连丢两次，回落中继。以前要等到 6.5 秒
        paths.due_pings(now + 3 * SEC, active);
        assert_eq!(paths.choose(current, &relays(), now + 3 * SEC), relay());
        assert!(!paths.has_fresh(now + 3 * SEC));
    }

    #[test]
    fn a_single_lost_pong_does_not_switch() {
        let now = Instant::now();
        let mut paths = working(now);
        let current = Some(Path::Direct(addr(1)));
        let active = Some(addr(1));
        paths.due_pings(now + SEC, active);
        // 丢了一次，补探的那个回来了
        assert_eq!(paths.due_pings(now + 2 * SEC, active), [addr(1)]);
        paths.on_pong(addr(1), 20 * MS, now + 2 * SEC + 20 * MS);
        paths.due_pings(now + 3 * SEC, active);
        assert_eq!(paths.choose(current, &relays(), now + 3 * SEC), current);
    }

    #[test]
    fn still_falls_back_when_nobody_pings() {
        // 没有新的探测结果时，FRESH 仍是上限
        let now = Instant::now();
        let paths = working(now);
        let current = Some(Path::Direct(addr(1)));
        let pong = now + 20 * MS;
        assert_eq!(paths.choose(current, &relays(), pong + FRESH - MS), current);
        assert_eq!(paths.choose(current, &relays(), pong + FRESH), relay());
    }

    #[test]
    fn a_network_change_drops_to_relay_at_once_and_reprobes() {
        let now = Instant::now();
        let mut paths = working(now);
        let current = Some(Path::Direct(addr(1)));
        paths.network_changed();
        assert_eq!(paths.choose(current, &relays(), now + 100 * MS), relay());
        // 马上重新探测，通了就回来
        assert_eq!(paths.due_pings(now + 100 * MS, None), [addr(1)]);
        paths.on_pong(addr(1), 25 * MS, now + 125 * MS);
        assert_eq!(
            paths.choose(relay(), &relays(), now + 125 * MS),
            Some(Path::Direct(addr(1)))
        );
    }

    #[test]
    fn rtt_is_smoothed_and_jitter_is_tracked() {
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        paths.set_advertised(&[addr(1)]);
        paths.on_pong(addr(1), 20 * MS, now);
        assert_eq!(paths.rtt(addr(1)), Some(20 * MS));
        assert_eq!(paths.jitter(addr(1)), Some(10 * MS));
        for _ in 0..50 {
            paths.on_pong(addr(1), 20 * MS, now);
        }
        assert_eq!(paths.rtt(addr(1)), Some(20 * MS));
        assert!(paths.jitter(addr(1)).unwrap() < MS, "稳定的路抖动趋于零");
        assert_eq!(paths.rtt(addr(9)), None);
    }

    #[test]
    fn a_steady_path_beats_a_faster_but_jittery_one() {
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        paths.set_advertised(&[addr(1), addr(2)]);
        for i in 0..20 {
            // addr(1) 平均 20 ms，但在 5 和 35 之间跳；addr(2) 稳稳的 30 ms
            let jittery = if i % 2 == 0 { 5 * MS } else { 35 * MS };
            paths.on_pong(addr(1), jittery, now);
            paths.on_pong(addr(2), 30 * MS, now);
        }
        assert!(paths.rtt(addr(1)).unwrap() < paths.rtt(addr(2)).unwrap());
        assert_eq!(
            paths.choose(None, &relays(), now),
            Some(Path::Direct(addr(2)))
        );
    }

    #[test]
    fn loss_is_smoothed() {
        let mut candidate = Candidate::default();
        for _ in 0..100 {
            candidate.record_loss(true);
            candidate.record_loss(false);
        }
        let loss = candidate.loss;
        assert!(
            (4_000..6_000).contains(&loss),
            "一半一半应接近 50%，实际万分之 {loss}"
        );
        for _ in 0..100 {
            candidate.record_loss(false);
        }
        assert!(candidate.loss < 100, "不再丢包，丢包率降回去");
    }

    #[test]
    fn a_lossy_path_loses_to_a_clean_slower_one() {
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        paths.set_advertised(&[addr(1), addr(2)]);
        for _ in 0..30 {
            paths.on_pong(addr(1), 20 * MS, now);
            paths.on_pong(addr(2), 35 * MS, now);
        }
        assert_eq!(
            paths.choose(None, &relays(), now),
            Some(Path::Direct(addr(1)))
        );
        // addr(1) 丢 30% 的包：罚 150ms
        paths.candidates.get_mut(&addr(1)).unwrap().loss = 3_000;
        assert_eq!(
            paths.choose(None, &relays(), now),
            Some(Path::Direct(addr(2)))
        );
    }

    #[test]
    fn an_occasional_miss_counts_as_loss() {
        let now = Instant::now();
        let mut paths = working(now);
        assert_eq!(paths.loss(addr(1)), Some(0.0));
        let active = Some(addr(1));
        paths.due_pings(now + SEC, active);
        // 1 秒没回：丢了一次，补探
        assert_eq!(paths.due_pings(now + 2 * SEC, active), [addr(1)]);
        // 补探的回来了：那一次算偶尔丢包
        paths.on_pong(addr(1), 20 * MS, now + 2 * SEC + 20 * MS);
        let loss = paths.loss(addr(1)).unwrap();
        assert!(loss > 0.05, "{loss}");
    }

    #[test]
    fn an_outage_is_not_counted_as_loss() {
        // e2e 里撞出来的：直连断了几十秒，恢复后丢包率要是记着那几十秒，就一直输给中继
        let now = Instant::now();
        let mut paths = working(now);
        let active = Some(addr(1));
        for second in 1..30 {
            paths.due_pings(now + second * SEC, active);
        }
        assert!(!paths.has_fresh(now + 30 * SEC), "断了");
        paths.due_pings(now + 30 * SEC, active);
        paths.on_pong(addr(1), 20 * MS, now + 30 * SEC + 20 * MS);
        assert_eq!(paths.loss(addr(1)), Some(0.0), "恢复了，丢包率还是干净的");
        assert!(paths.has_fresh(now + 30 * SEC + 20 * MS));
    }

    #[test]
    fn a_working_current_path_is_kept_unless_clearly_worse() {
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        paths.set_advertised(&[addr(1), addr(2), addr(3)]);
        for _ in 0..30 {
            paths.on_pong(addr(1), 40 * MS, now);
            paths.on_pong(addr(2), 35 * MS, now);
            paths.on_pong(addr(3), 10 * MS, now);
        }
        // 差 5 ms：不值得换
        let on_1 = Some(Path::Direct(addr(1)));
        paths.candidates.remove(&addr(3));
        assert_eq!(paths.choose(on_1, &[], now), on_1);
        // 差 30 ms：换
        paths.set_advertised(&[addr(1), addr(2), addr(3)]);
        for _ in 0..30 {
            paths.on_pong(addr(3), 10 * MS, now);
        }
        assert_eq!(paths.choose(on_1, &[], now), Some(Path::Direct(addr(3))));
    }

    /// 中继那条路测过若干次，每次都是 `rtt`
    fn measure_relay(paths: &mut PeerPaths, rtt: Duration, now: Instant) {
        measure_relay_n(paths, relay_n(0), rtt, now);
    }

    fn measure_relay_n(paths: &mut PeerPaths, relay: Path, rtt: Duration, now: Instant) {
        paths.relay_due(relay, now, false);
        for _ in 0..30 {
            paths.on_relay_pong(relay, rtt, now);
        }
    }

    #[test]
    fn the_relay_path_is_probed_and_measured() {
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        assert!(paths.relay_due(relay().unwrap(), now, false));
        assert!(
            !paths.relay_due(relay().unwrap(), now + MS, false),
            "刚探测过"
        );
        assert_eq!(paths.relay_quality(relay().unwrap()), None);
        paths.on_relay_pong(relay().unwrap(), 40 * MS, now + 40 * MS);
        let (rtt, _, loss) = paths.relay_quality(relay().unwrap()).unwrap();
        assert_eq!(rtt, 40 * MS);
        assert_eq!(loss, 0.0);
        // 正在走中继时探测得更勤
        assert!(paths.relay_due(relay().unwrap(), now + SEC, true));
        assert!(!paths.relay_due(relay().unwrap(), now + SEC + MS, false));
    }

    #[test]
    fn a_clearly_better_relay_beats_a_bad_direct_path() {
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        paths.set_advertised(&[addr(1)]);
        for _ in 0..30 {
            paths.on_pong(addr(1), 20 * MS, now);
        }
        measure_relay(&mut paths, 60 * MS, now);
        let direct = Some(Path::Direct(addr(1)));
        // 干净的 20ms 直连：不走 60ms 的中继
        assert_eq!(paths.choose(None, &relays(), now), direct);
        assert_eq!(paths.choose(direct, &relays(), now), direct);
        // 直连丢 40% 的包（罚 200ms）：中继明显更好，换过去
        paths.candidates.get_mut(&addr(1)).unwrap().loss = 4_000;
        assert_eq!(paths.choose(direct, &relays(), now), relay());
        assert_eq!(paths.choose(None, &relays(), now), relay());
    }

    #[test]
    fn a_barely_measured_direct_path_is_not_judged_against_the_relay() {
        // e2e 里撞出来的：直连第一个样本 4ms（经 NAT 的第一个报文慢），分数 12ms；
        // 中继测过很多次，0.5ms。不能就此认定中继明显更好
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        measure_relay(&mut paths, Duration::from_micros(500), now);
        paths.set_advertised(&[addr(1)]);
        paths.on_pong(addr(1), 4 * MS, now);
        let direct = Some(Path::Direct(addr(1)));
        assert_eq!(paths.choose(relay(), &relays(), now), direct);
        // 测够了，直连其实很快：照样直连
        for _ in 0..SETTLED_SAMPLES {
            paths.on_pong(addr(1), Duration::from_micros(300), now);
        }
        assert_eq!(paths.choose(direct, &relays(), now), direct);
    }

    #[test]
    fn direct_is_preferred_when_the_relay_is_only_a_little_faster() {
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        paths.set_advertised(&[addr(1)]);
        for _ in 0..30 {
            paths.on_pong(addr(1), 20 * MS, now);
        }
        measure_relay(&mut paths, 15 * MS, now);
        assert_eq!(
            paths.choose(None, &relays(), now),
            Some(Path::Direct(addr(1)))
        );
        // 正在走中继（比如直连刚恢复）：中继没有明显更好，就回到直连
        assert_eq!(
            paths.choose(relay(), &relays(), now),
            Some(Path::Direct(addr(1)))
        );
    }

    #[test]
    fn comes_back_to_direct_when_both_paths_are_fast() {
        // e2e 里撞出来的：直连 0.1ms、中继 0.5ms，落到中继后要能回到直连
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        paths.set_advertised(&[addr(1)]);
        for _ in 0..30 {
            paths.on_pong(addr(1), Duration::from_micros(100), now);
        }
        measure_relay(&mut paths, Duration::from_micros(500), now);
        assert_eq!(
            paths.choose(relay(), &relays(), now),
            Some(Path::Direct(addr(1)))
        );
    }

    #[test]
    fn an_unmeasured_relay_does_not_displace_a_working_direct_path() {
        let now = Instant::now();
        let paths = working(now);
        let direct = Some(Path::Direct(addr(1)));
        assert_eq!(paths.choose(direct, &relays(), now + 30 * MS), direct);
        assert_eq!(paths.choose(relay(), &relays(), now + 30 * MS), direct);
    }

    #[test]
    fn a_network_change_also_invalidates_the_relay_measurement() {
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        measure_relay(&mut paths, 30 * MS, now);
        assert!(paths.relays[&relay().unwrap()].fresh(now));
        paths.network_changed();
        assert!(!paths.relays[&relay().unwrap()].fresh(now));
        assert!(
            paths.relay_due(relay().unwrap(), now + MS, false),
            "马上重新探测"
        );
    }

    #[test]
    fn pong_for_an_unknown_address_changes_nothing() {
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        paths.on_pong(addr(7), 10 * MS, now);
        assert_eq!(paths.choose(None, &relays(), now), relay());
    }

    #[test]
    fn an_unanswered_learned_candidate_gets_just_two_pings() {
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        assert!(paths.learn(addr(5), now), "第一次学到");
        assert_eq!(paths.due_pings(now, None), [addr(5)]);
        assert!(!paths.learn(addr(5), now + MS), "探测过了就不算新");

        // 从没通过的候选丢了也不补探：还是 PING_INTERVAL 一次
        assert!(paths.due_pings(now + 2 * SEC, None).is_empty());
        // 从最后一次学到它（now + 1ms）起算：再探测一次，然后忘掉
        assert_eq!(paths.due_pings(now + PING_INTERVAL, None), [addr(5)]);
        let forgotten = now + MS + UNVERIFIED_TTL;
        assert!(
            paths.due_pings(forgotten - MS, None).is_empty(),
            "两次探测之间"
        );
        assert!(paths.due_pings(forgotten, None).is_empty());
        assert!(paths.learn(addr(5), forgotten), "忘掉之后再学到又是新的");
    }

    #[test]
    fn a_learned_candidate_that_answered_is_kept_longer() {
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        paths.learn(addr(5), now);
        paths.due_pings(now, None);
        paths.on_pong(addr(5), 10 * MS, now);

        // 早就不"通"了，但回应过，所以按 LEARNED_TTL 记着，照常探测
        let later = now + LEARNED_TTL - MS;
        assert!(!paths.has_fresh(later));
        assert_eq!(paths.due_pings(later, None), [addr(5)]);
        assert!(
            paths
                .due_pings(now + LEARNED_TTL + PING_INTERVAL, None)
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
        assert_eq!(paths.due_pings(now, None), [addr(2), addr(3)]);
    }

    #[test]
    fn ping_now_makes_a_candidate_due_immediately() {
        let now = Instant::now();
        let mut paths = working(now);
        paths.ping_now(addr(1));
        assert_eq!(paths.due_pings(now + 30 * MS, None), [addr(1)]);
    }

    #[test]
    fn a_dead_relay_is_replaced_by_another_within_seconds() {
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        let both = [relay_n(0), relay_n(1)];
        measure_relay_n(&mut paths, relay_n(0), 30 * MS, now);
        measure_relay_n(&mut paths, relay_n(1), 40 * MS, now);
        let on_0 = Some(relay_n(0));
        assert_eq!(paths.choose(None, &both, now), on_0);
        // 中继 0 挂了：正在用它，每秒探测，连丢两次就换到中继 1
        assert!(paths.relay_due(relay_n(0), now + SEC, true));
        assert!(
            paths.relay_due(relay_n(0), now + 2 * SEC, true),
            "丢了一次，补探"
        );
        assert_eq!(paths.choose(on_0, &both, now + 2 * SEC), on_0);
        paths.relay_due(relay_n(0), now + 3 * SEC, true);
        assert_eq!(paths.choose(on_0, &both, now + 3 * SEC), Some(relay_n(1)));
    }

    #[test]
    fn relays_follow_the_same_switching_rules() {
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        let both = [relay_n(0), relay_n(1)];
        measure_relay_n(&mut paths, relay_n(0), 50 * MS, now);
        measure_relay_n(&mut paths, relay_n(1), 45 * MS, now);
        // 只快一点：留在当前的中继上
        let on_0 = Some(relay_n(0));
        assert_eq!(paths.choose(on_0, &both, now), on_0);
        // 快得多：换
        measure_relay_n(&mut paths, relay_n(1), 10 * MS, now);
        assert_eq!(paths.choose(on_0, &both, now), Some(relay_n(1)));
        // 一条通着、稳定的直连，只比中继慢一点，照样优先
        paths.set_advertised(&[addr(1)]);
        for _ in 0..30 {
            paths.on_pong(addr(1), 12 * MS, now);
        }
        assert_eq!(
            paths.choose(Some(relay_n(1)), &both, now),
            Some(Path::Direct(addr(1)))
        );
    }

    #[test]
    fn with_nothing_measured_the_current_relay_is_kept() {
        let now = Instant::now();
        let paths = PeerPaths::default();
        let both = [relay_n(0), relay_n(1)];
        assert_eq!(paths.choose(None, &both, now), Some(relay_n(0)), "排第一的");
        let on_1 = Some(relay_n(1));
        assert_eq!(
            paths.choose(on_1, &both, now),
            on_1,
            "不在没测过的中继之间换"
        );
        // 当前的中继不在协调服务给的列表里了
        assert_eq!(paths.choose(on_1, &[relay_n(0)], now), Some(relay_n(0)));
        assert_eq!(paths.choose(None, &[], now), None, "没有中继");
    }

    #[test]
    fn relays_no_longer_offered_are_forgotten() {
        let now = Instant::now();
        let mut paths = PeerPaths::default();
        measure_relay_n(&mut paths, relay_n(0), 30 * MS, now);
        measure_relay_n(&mut paths, relay_n(1), 30 * MS, now);
        paths.retain_relays(&[relay_n(1)]);
        assert_eq!(paths.relay_quality(relay_n(0)), None);
        assert!(paths.relay_quality(relay_n(1)).is_some());
        // 已经测过的中继 0 即使还在记录里，不在列表里也不选
        measure_relay_n(&mut paths, relay_n(0), MS, now);
        assert_eq!(paths.choose(None, &[relay_n(1)], now), Some(relay_n(1)));
    }
}
