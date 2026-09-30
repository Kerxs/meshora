//! Windows：wintun。
//!
//! 在 CI 的 Windows 虚拟机上实测过：两个方向都能收发报文，meshorad 能经它和别的节点通信。
//! 地址、MTU、跃点数用 IP Helper API 设置，不调 netsh：桌面端没有控制台，起一个 netsh 就闪一个黑窗口。

use std::io;
use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{debug, warn};

use windows_sys::Win32::Foundation::{ERROR_OBJECT_ALREADY_EXISTS, NO_ERROR, WIN32_ERROR};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    CreateUnicastIpAddressEntry, GetIpInterfaceEntry, InitializeIpInterfaceEntry,
    InitializeUnicastIpAddressEntry, MIB_IPINTERFACE_ROW, MIB_UNICASTIPADDRESS_ROW,
    SetIpInterfaceEntry,
};
use windows_sys::Win32::NetworkManagement::Ndis::NET_LUID_LH;
use windows_sys::Win32::Networking::WinSock::{AF_INET, IN_ADDR, IpDadStatePreferred};

use crate::{Pipes, TunConfig};

/// wintun 环形缓冲区的容量，必须是 2 的幂。取 wireguard-go 用的 8 MiB
const RING_CAPACITY: u32 = 0x80_0000;

pub(crate) struct Device {
    session: Arc<wintun::Session>,
}

pub(crate) fn open(config: &TunConfig) -> io::Result<(Device, String)> {
    let dll = std::env::current_exe()?.with_file_name("wintun.dll");
    // SAFETY: 加载 DLL 会执行它的初始化代码。只从 meshorad.exe 所在的目录加载，
    // 不走系统搜索路径 —— 那样当前目录或 PATH 里任何一个叫 wintun.dll 的文件都会被加载
    // （DLL 劫持）。这个文件可不可信，取决于安装目录的写权限。
    let wintun = unsafe { wintun::load_from_path(&dll) }
        .map_err(|err| io::Error::other(format!("加载 {} 失败：{err}", dll.display())))?;
    let adapter = wintun::Adapter::create(&wintun, &config.name, "Meshora", None).map_err(other)?;
    // wintun 用的是另一个版本的 windows-sys，LUID 按 64 位整数交接
    // SAFETY: NET_LUID_LH 是 u64 和位域结构的 union，两种解读都是同样的 8 个字节
    let luid = NET_LUID_LH {
        Value: unsafe { adapter.get_luid().Value },
    };
    add_address(luid, config)?;
    configure_interface(luid, config)?;
    let name = adapter.get_name().map_err(other)?;
    let session = Arc::new(adapter.start_session(RING_CAPACITY).map_err(other)?);
    Ok((Device { session }, name))
}

pub(crate) fn spawn(device: Device, capacity: usize) -> io::Result<Pipes> {
    let (read_tx, read_rx) = mpsc::channel(capacity);
    let (write_tx, mut write_rx) = mpsc::channel::<Vec<u8>>(capacity);
    let session = device.session;

    // wintun 的读是阻塞的，放进单独的线程
    let reader = Arc::clone(&session);
    std::thread::Builder::new()
        .name("meshora-tun-read".into())
        .spawn(move || {
            loop {
                match reader.receive_blocking() {
                    Ok(packet) => {
                        if read_tx.blocking_send(packet.bytes().to_vec()).is_err() {
                            return;
                        }
                    }
                    Err(err) => {
                        debug!(%err, "wintun 会话结束");
                        return;
                    }
                }
            }
        })?;

    tokio::spawn(async move {
        while let Some(packet) = write_rx.recv().await {
            let Ok(len) = u16::try_from(packet.len()) else {
                continue;
            };
            match session.allocate_send_packet(len) {
                Ok(mut out) => {
                    out.bytes_mut().copy_from_slice(&packet);
                    session.send_packet(out);
                }
                Err(err) => debug!(%err, "wintun 发送缓冲区满，丢弃一个报文"),
            }
        }
        // 写的一方关了：结束会话，让阻塞在读上的线程退出
        if let Err(err) = session.shutdown() {
            warn!(%err, "关闭 wintun 会话失败");
        }
    });
    Ok(Pipes {
        from_tun: read_rx,
        to_tun: write_tx,
    })
}

/// 给网卡加上 overlay 地址（带前缀长度，系统据此加上整个网段的路由）
fn add_address(luid: NET_LUID_LH, config: &TunConfig) -> io::Result<()> {
    let mut row = MIB_UNICASTIPADDRESS_ROW::default();
    // SAFETY: row 是我们自己的、正确对齐的结构体，Initialize 只往里填默认值
    unsafe { InitializeUnicastIpAddressEntry(&mut row) };
    row.InterfaceLuid = luid;
    row.Address.Ipv4.sin_family = AF_INET;
    row.Address.Ipv4.sin_addr = IN_ADDR {
        S_un: windows_sys::Win32::Networking::WinSock::IN_ADDR_0 {
            S_addr: u32::from_ne_bytes(config.address.octets()),
        },
    };
    row.OnLinkPrefixLength = config.prefix_len;
    // 虚拟网卡上没有别人，不必做重复地址检测，免得地址要等几秒才可用
    row.DadState = IpDadStatePreferred;
    // SAFETY: row 已经完整初始化，函数只读它
    match unsafe { CreateUnicastIpAddressEntry(&row) } {
        // 同名网卡上次没清掉、地址还在：照用
        NO_ERROR | ERROR_OBJECT_ALREADY_EXISTS => Ok(()),
        err => Err(win32("设置网卡地址", err)),
    }
}

/// 设 IPv4 接口的 MTU 和跃点数
fn configure_interface(luid: NET_LUID_LH, config: &TunConfig) -> io::Result<()> {
    let mut row = MIB_IPINTERFACE_ROW::default();
    // SAFETY: 同上，Initialize 只往我们自己的结构体里填默认值
    unsafe { InitializeIpInterfaceEntry(&mut row) };
    row.Family = AF_INET;
    row.InterfaceLuid = luid;
    // SAFETY: row 里 Family 和 LUID 已经填好，函数按它们查出其余字段写回 row
    let err = unsafe { GetIpInterfaceEntry(&mut row) };
    if err != NO_ERROR {
        return Err(win32("读取网卡的 IPv4 设置", err));
    }
    row.NlMtu = config.mtu.into();
    if let Some(metric) = config.metric {
        row.UseAutomaticMetric = false;
        row.Metric = metric;
    }
    // IPv4 接口读出来的 SitePrefixLength 可能不是 0，原样写回会被拒（ERROR_INVALID_PARAMETER）
    row.SitePrefixLength = 0;
    // SAFETY: row 是刚读出来又改过几个字段的完整结构体
    let err = unsafe { SetIpInterfaceEntry(&mut row) };
    if err != NO_ERROR {
        return Err(win32("设置网卡的 MTU 和跃点数", err));
    }
    Ok(())
}

/// 把名叫 `name` 的网卡所在的网络设成"专用网络"（`private` 为假时设回"公用网络"）。
///
/// 没有调 COM 的 INetworkListManager：windows-sys 不带 COM 接口，为这一处拉进整个 windows crate 不值。
/// 用系统自带的 `Set-NetConnectionProfile`：按系统目录的绝对路径起 powershell.exe（客户端以管理员身份运行，
/// 不能按 PATH 找），不弹控制台窗口。
///
/// 网卡刚建好时系统还没把它归到哪个网络（网络位置识别要几秒），这时会失败，调用方要隔一会儿再试
pub(crate) fn set_network_private(name: &str, private: bool) -> io::Result<()> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let powershell = system_directory()?.join(r"WindowsPowerShell\v1.0\powershell.exe");
    let output = std::process::Command::new(powershell)
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(profile_command(name, private))
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if output.status.success() {
        return Ok(());
    }
    // 输出是控制台代码页（中文系统上是 GBK），按 UTF-8 解会有乱码，只当作参考写进错误里
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(io::Error::other(format!(
        "设置网络类别失败（{}）：{}",
        output.status,
        stderr.trim()
    )))
}

/// 设网络类别的 PowerShell 命令。网卡名放进单引号字符串，里面的单引号按 PowerShell 的规矩写两个
fn profile_command(name: &str, private: bool) -> String {
    let category = if private { "Private" } else { "Public" };
    let alias = name.replace('\'', "''");
    format!(
        "Set-NetConnectionProfile -InterfaceAlias '{alias}' -NetworkCategory {category} -ErrorAction Stop"
    )
}

/// 系统目录（通常是 `C:\Windows\System32`）。向系统要，不看环境变量
fn system_directory() -> io::Result<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;

    let mut buf = [0u16; 260];
    // SAFETY: 缓冲区是我们自己的，长度如实告诉函数；它最多写这么多个 u16
    let len = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if len == 0 || len > buf.len() {
        return Err(io::Error::other("拿不到系统目录"));
    }
    Ok(std::ffi::OsString::from_wide(&buf[..len]).into())
}

fn win32(what: &str, err: WIN32_ERROR) -> io::Error {
    let os = io::Error::from_raw_os_error(err as i32);
    io::Error::new(os.kind(), format!("{what}失败：{os}"))
}

fn other(err: wintun::Error) -> io::Error {
    io::Error::other(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_command_quotes_the_adapter_name() {
        assert_eq!(
            profile_command("Meshora", true),
            "Set-NetConnectionProfile -InterfaceAlias 'Meshora' -NetworkCategory Private -ErrorAction Stop"
        );
        assert_eq!(
            profile_command("it's; rm", false),
            "Set-NetConnectionProfile -InterfaceAlias 'it''s; rm' -NetworkCategory Public -ErrorAction Stop"
        );
    }

    #[test]
    fn the_system_directory_holds_powershell() {
        let dir = system_directory().unwrap();
        assert!(
            dir.join(r"WindowsPowerShell\v1.0\powershell.exe").is_file(),
            "{}",
            dir.display()
        );
    }

    /// 真的建一块网卡，设成专用网络、读回来，再设回公用网络。要管理员权限和 wintun.dll，
    /// 和 lib.rs 里的 `packets_flow_both_ways` 一样在 CI 的 Windows 虚拟机上跑
    #[test]
    #[ignore = "需要管理员权限和 wintun.dll。用 cargo test -p meshora-tun -- --ignored 跑"]
    fn the_adapter_can_be_made_a_private_network() {
        use std::time::{Duration, Instant};

        let name = format!("mshprof{}", std::process::id() % 100_000);
        let _tun = crate::Tun::open(&crate::TunConfig {
            name: name.clone(),
            address: std::net::Ipv4Addr::new(198, 18, 78, 1),
            prefix_len: 24,
            mtu: 1280,
            metric: None,
        })
        .unwrap();

        let category = || {
            let script = format!(
                "(Get-NetConnectionProfile -InterfaceAlias '{name}' -ErrorAction Stop).NetworkCategory"
            );
            let output = std::process::Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                .output()
                .unwrap();
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        };

        // 网络位置识别要几秒：没归到网络之前会失败，隔一会儿再试
        let started = Instant::now();
        let set = |private| loop {
            match set_network_private(&name, private) {
                Ok(()) => return,
                Err(err) if started.elapsed() < Duration::from_secs(60) => {
                    eprintln!("还不行，再试：{err}");
                    std::thread::sleep(Duration::from_secs(2));
                }
                Err(err) => panic!("60 秒内没设成：{err}"),
            }
        };
        set(true);
        eprintln!("用了 {:?} 设成专用网络", started.elapsed());
        assert_eq!(category(), "Private");
        set(false);
        assert_eq!(category(), "Public");
    }
}
