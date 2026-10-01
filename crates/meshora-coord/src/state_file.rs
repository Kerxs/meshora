//! 状态文件：一个网络的邀请码和凭邀请码加入的成员，纯文本，一行一项。
//!
//! 单网络模式下是 `--state` 指的那个文件，人可以直接改；hub 模式下每个网络一个 `<网络 ID>.state`，
//! 多三种行：`owner`（网主）、`name`（网络名）、`seen`（最近有人在线的时刻，闲置清理用）。

use std::fmt::Write as _;
use std::io;
use std::net::Ipv4Addr;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ipnet::Ipv4Net;
use meshora_types::{Invite, NodeKey};

/// 一个成员
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Member {
    pub key: NodeKey,
    pub ip: Ipv4Addr,
    /// 凭邀请码加入的（存在状态文件里），不是名单里的
    pub joined: bool,
}

/// 一个带限制的邀请码：只能用几次，或者到时候就过期
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Limited {
    pub code: Invite,
    /// 还能用几次；`None` 是不限次数
    pub uses_left: Option<u32>,
    /// 过期时间；`None` 是不过期
    pub expires: Option<SystemTime>,
}

impl Limited {
    pub fn expired(&self, now: SystemTime) -> bool {
        self.expires.is_some_and(|at| now >= at) || self.uses_left == Some(0)
    }

    /// 状态文件里的一行
    pub fn line(&self) -> String {
        let mut line = format!("invite {}", self.code);
        if let Some(uses) = self.uses_left {
            let _ = write!(line, " uses={uses}");
        }
        if let Some(at) = self.expires {
            let _ = write!(line, " expires={}", format_utc(at));
        }
        line
    }
}

/// 状态文件里的东西
#[derive(Default)]
pub(crate) struct Stored {
    pub invite: Option<Invite>,
    pub limited: Vec<Limited>,
    pub joined: Vec<(NodeKey, Ipv4Addr)>,
    pub owner: Option<NodeKey>,
    pub name: Option<String>,
    pub seen: Option<SystemTime>,
}

/// 要写进状态文件的东西
pub(crate) struct Saved<'a> {
    pub invite: &'a Invite,
    pub limited: &'a [Limited],
    pub members: &'a [Member],
    pub owner: Option<&'a NodeKey>,
    pub name: &'a str,
    pub seen: Option<SystemTime>,
}

fn invalid_data(path: &Path, line: usize, what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("状态文件 {} 第 {line} 行：{what}", path.display()),
    )
}

/// 读状态文件。文件不在就是还没有
pub(crate) fn load_state(path: &Path) -> io::Result<Stored> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Stored::default()),
        Err(err) => return Err(err),
    };
    let mut stored = Stored::default();
    for (index, line) in text.lines().enumerate() {
        let line_no = index + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // 网络名里可以有空格：整行剩下的都是
        if let Some(name) = line.strip_prefix("name ") {
            stored.name = Some(name.trim().to_owned());
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        match fields.as_slice() {
            ["invite", code] => {
                if stored.invite.is_some() {
                    return Err(invalid_data(path, line_no, "不带限制的 invite 只能有一行"));
                }
                stored.invite = Some(
                    code.parse()
                        .map_err(|_| invalid_data(path, line_no, "邀请码格式不对"))?,
                );
            }
            ["invite", code, options @ ..] => {
                let mut entry = Limited {
                    code: code
                        .parse()
                        .map_err(|_| invalid_data(path, line_no, "邀请码格式不对"))?,
                    uses_left: None,
                    expires: None,
                };
                for option in options {
                    match option.split_once('=') {
                        Some(("uses", n)) => {
                            entry.uses_left = Some(n.parse().map_err(|_| {
                                invalid_data(path, line_no, "uses= 后面应该是次数")
                            })?);
                        }
                        Some(("expires", at)) => {
                            entry.expires = Some(parse_utc(at).ok_or_else(|| {
                                invalid_data(
                                    path,
                                    line_no,
                                    "expires= 后面应该是 2026-10-01T12:00:00Z 这样的 UTC 时间",
                                )
                            })?);
                        }
                        _ => return Err(invalid_data(path, line_no, "认不出 invite 后面的选项")),
                    }
                }
                stored.limited.push(entry);
            }
            ["member", key, ip] => stored.joined.push((
                key.parse()
                    .map_err(|_| invalid_data(path, line_no, "公钥格式不对"))?,
                ip.parse()
                    .map_err(|_| invalid_data(path, line_no, "地址格式不对"))?,
            )),
            ["owner", key] => {
                stored.owner = Some(
                    key.parse()
                        .map_err(|_| invalid_data(path, line_no, "网主的公钥格式不对"))?,
                );
            }
            ["seen", at] => {
                stored.seen = Some(
                    parse_utc(at)
                        .ok_or_else(|| invalid_data(path, line_no, "seen 的时间格式不对"))?,
                );
            }
            _ => return Err(invalid_data(path, line_no, "认不出这一行")),
        }
    }
    Ok(stored)
}

/// 整个重写状态文件：先写临时文件再改名，写到一半断电也不会留下半个文件
pub(crate) fn save_state(path: &Path, saved: &Saved<'_>) -> io::Result<()> {
    let mut text =
        String::from("# Meshora 协调服务的状态文件。可以手改，改完不用重启，几秒内生效。\n");
    if let Some(owner) = saved.owner {
        let _ = writeln!(text, "owner {owner}");
    }
    if !saved.name.is_empty() {
        let _ = writeln!(text, "name {}", saved.name);
    }
    if let Some(seen) = saved.seen {
        let _ = writeln!(text, "seen {}", format_utc(seen));
    }
    text.push_str(
        "# invite：邀请码，拿到它的人都能加入。删掉不带限制的那一行会换一个新的，旧的网络码随之失效\n",
    );
    let _ = writeln!(text, "invite {}", saved.invite);
    let now = SystemTime::now();
    for entry in saved.limited.iter().filter(|l| !l.expired(now)) {
        let _ = writeln!(text, "{}", entry.line());
    }
    text.push_str("# member：凭邀请码加入的成员（公钥 地址）。删掉一行，这个成员就不在网里了\n");
    for member in saved.members.iter().filter(|member| member.joined) {
        let _ = writeln!(text, "member {} {}", member.key, member.ip);
    }
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, path)
}

/// 把状态文件里凭邀请码加入的成员并进名单里的成员。名单里已经有的跳过（以名单为准）；
/// 地址不在网段里、和别人撞上都是错
pub(crate) fn merge_joined(
    mut members: Vec<Member>,
    joined: Vec<(NodeKey, Ipv4Addr)>,
    overlay: Ipv4Net,
) -> Result<Vec<Member>, String> {
    for (key, ip) in joined {
        if members.iter().any(|member| member.key == key) {
            // 后来又写进了名单：以名单为准，这一条下次存的时候自然消失
            continue;
        }
        if !overlay.contains(&ip) {
            return Err(format!(
                "状态文件里 {key} 的地址 {ip} 不在 overlay 网段 {overlay} 里"
            ));
        }
        if let Some(other) = members.iter().find(|member| member.ip == ip) {
            return Err(format!(
                "状态文件里 {key} 的地址 {ip} 已经分给了 {}（名单加长之后撞上了？）。\
                 把名单里的这个节点挪到最后，或者从状态文件里删掉那一行",
                other.key
            ));
        }
        members.push(Member {
            key,
            ip,
            joined: true,
        });
    }
    Ok(members)
}

/// 网段里最小的、还没分出去的地址
pub(crate) fn free_address(members: &[Member], overlay: Ipv4Net) -> Option<Ipv4Addr> {
    overlay
        .hosts()
        .find(|ip| !members.iter().any(|member| member.ip == *ip))
}

/// 按名单分配地址：第 n 个节点拿网段里第 n 个可用地址（从 .1 开始）
pub(crate) fn assign(nodes: &[NodeKey], overlay: Ipv4Net) -> io::Result<Vec<Member>> {
    let invalid = |message: String| io::Error::new(io::ErrorKind::InvalidInput, message);
    let mut hosts = overlay.hosts();
    let mut members: Vec<Member> = Vec::with_capacity(nodes.len());
    for key in nodes {
        if members.iter().any(|member| member.key == *key) {
            return Err(invalid(format!("节点 {key} 在名单里出现了两次")));
        }
        let ip = hosts.next().ok_or_else(|| {
            invalid(format!(
                "overlay 网段 {overlay} 装不下 {} 个节点",
                nodes.len()
            ))
        })?;
        members.push(Member {
            key: *key,
            ip,
            joined: false,
        });
    }
    Ok(members)
}

/// 生成一个带限制的邀请码，交出邀请码和写进状态文件的那一行。
///
/// `uses` 是能用几次，`valid_for` 是从现在起多久过期；两个都是 `None` 就和长期有效的没区别，
/// 所以至少要给一个。
pub fn limited_invite(uses: Option<u32>, valid_for: Option<Duration>) -> Option<(Invite, String)> {
    let entry = new_limited(uses, valid_for)?;
    Some((entry.code, entry.line()))
}

pub(crate) fn new_limited(uses: Option<u32>, valid_for: Option<Duration>) -> Option<Limited> {
    if uses.is_none() && valid_for.is_none() {
        return None;
    }
    Some(Limited {
        code: Invite::generate(),
        uses_left: uses,
        expires: valid_for.map(|d| SystemTime::now() + d),
    })
}

/// 状态文件的样子：内容的哈希。文件不在是 `None`
///
/// 不用修改时间：Windows 上文件时间的精度可能只有十几毫秒，紧挨着的两次写、长度又一样
/// （比如换了个邀请码），修改时间可能根本不变。文件很小，每轮读一遍不算什么
pub(crate) type Stamp = u64;

pub(crate) fn file_stamp(path: &Path) -> Option<Stamp> {
    use std::hash::{Hash, Hasher};
    let bytes = std::fs::read(path).ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    Some(hasher.finish())
}

/// 公历日期到 1970-01-01 起的天数（Howard Hinnant 的算法）
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let month = i64::from(month);
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// 1970-01-01 起的天数到公历日期
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// 写成 `2026-10-01T12:00:00Z`
pub(crate) fn format_utc(at: SystemTime) -> String {
    let secs = at
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
    let rest = secs.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// 读 `2026-10-01T12:00:00Z`。只认 UTC、只认这一种写法
pub(crate) fn parse_utc(text: &str) -> Option<SystemTime> {
    let text = text.strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let mut date = date.split('-');
    let year: i64 = date.next()?.parse().ok()?;
    let month: u32 = date.next()?.parse().ok()?;
    let day: u32 = date.next()?.parse().ok()?;
    let mut time = time.split(':');
    let hour: u64 = time.next()?.parse().ok()?;
    let minute: u64 = time.next()?.parse().ok()?;
    let second: u64 = time.next()?.parse().ok()?;
    if date.next().is_some()
        || time.next().is_some()
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
        || year < 1970
    {
        return None;
    }
    let days = u64::try_from(days_from_civil(year, month, day)).ok()?;
    let secs = days * 86_400 + hour * 3600 + minute * 60 + second;
    // 2 月 30 日之类的：换算回去对不上
    if format_utc(UNIX_EPOCH + Duration::from_secs(secs)) != format!("{text}Z") {
        return None;
    }
    Some(UNIX_EPOCH + Duration::from_secs(secs))
}
