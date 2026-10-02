//! 装、卸。只在 Windows 上编。
//!
//! - 装到 `Program Files\Meshora`，不让改：客户端以管理员身份运行、从自己旁边加载 wintun.dll，
//!   这个目录必须只有管理员能写（和不出免安装版是同一个理由，见威胁模型）
//! - 正在跑的 Meshora 先关掉；1.0.0 用 NSIS 装的，先静默跑它的卸载器
//! - 开始菜单的快捷方式一定建，桌面的看用户选；卸载项写进 HKLM，“应用和功能”里看得到
//! - 安装程序把自己复制成 `uninstall.exe`：卸载时就是同一个程序、同一个界面

use std::os::windows::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use winreg::RegKey;
use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_ALL_ACCESS};

use meshora_setup::{acl, firewall, payload};

/// 不弹控制台窗口
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// 卸载项的位置
const UNINSTALL_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Meshora";
/// 我们写的卸载项里有这个值；没有的是 1.0.0 的 NSIS 写的
const OURS: &str = "MeshoraSetup";

/// 默认的安装目录：`Program Files\Meshora`
pub fn default_dir() -> PathBuf {
    let base = std::env::var_os("ProgramW6432")
        .or_else(|| std::env::var_os("ProgramFiles"))
        .map_or_else(|| PathBuf::from(r"C:\Program Files"), PathBuf::from);
    base.join("Meshora")
}

/// 已经装着的话，装在哪（卸载项里的 InstallLocation）。升级、更新都装回原处
pub fn installed_dir() -> Option<PathBuf> {
    let key = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(UNINSTALL_KEY)
        .ok()?;
    let dir: String = key.get_value("InstallLocation").ok()?;
    let dir = PathBuf::from(dir.trim_matches('"'));
    dir.is_absolute().then_some(dir)
}

/// 现在该用的安装目录：装着的就是原处，没装过就是默认位置
pub fn current_dir() -> PathBuf {
    installed_dir().unwrap_or_else(default_dir)
}

/// 已经装着的版本（卸载项里的 DisplayVersion）
pub fn installed_version() -> Option<String> {
    let key = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(UNINSTALL_KEY)
        .ok()?;
    key.get_value("DisplayVersion").ok()
}

fn system32() -> PathBuf {
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
    let mut buf = [0u16; 260];
    // SAFETY: 缓冲区是我们自己的，长度如实告诉函数
    let len = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if len == 0 || len > buf.len() {
        return PathBuf::from(r"C:\Windows\System32");
    }
    PathBuf::from(String::from_utf16_lossy(&buf[..len]))
}

fn quiet(program: &str) -> Command {
    let mut command = Command::new(system32().join(program));
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

/// meshora.exe 在不在跑
fn running() -> bool {
    quiet("tasklist.exe")
        .args(["/FI", "IMAGENAME eq meshora.exe", "/NH"])
        .output()
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .to_lowercase()
                .contains("meshora.exe")
        })
        .unwrap_or(false)
}

/// 关掉正在跑的 Meshora（它的虚拟网卡随进程一起消失）。等它真的退出
fn stop_running(patience: Duration) -> Result<(), String> {
    // 更新时客户端是自己退出的（退出时把网卡收拾干净）：先等它一会儿，等不到再强行结束
    let waiting = Instant::now() + patience;
    while running() && Instant::now() < waiting {
        std::thread::sleep(Duration::from_millis(200));
    }
    if !running() {
        return Ok(());
    }
    let _ = quiet("taskkill.exe")
        .args(["/IM", "meshora.exe", "/F"])
        .output();
    let deadline = Instant::now() + Duration::from_secs(10);
    while running() {
        if Instant::now() > deadline {
            return Err("关不掉正在运行的 Meshora：先从托盘里退出它再试".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}

/// 1.0.0 用 NSIS 装的：静默跑它的卸载器，等它把文件删完
fn remove_nsis_install(dir: &Path) -> Result<(), String> {
    let Ok(key) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(UNINSTALL_KEY) else {
        return Ok(());
    };
    if key.get_value::<u32, _>(OURS).is_ok() {
        return Ok(());
    }
    let Ok(uninstaller) = key.get_value::<String, _>("UninstallString") else {
        return Ok(());
    };
    let uninstaller = uninstaller.trim_matches('"').to_owned();
    if !Path::new(&uninstaller).is_file() {
        return Ok(());
    }
    Command::new(&uninstaller)
        .arg("/S")
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|err| format!("旧版本的卸载程序跑不起来：{err}"))?;
    // NSIS 的卸载器会把自己复制到临时目录再跑，上面那一步马上就返回：等它删完
    let deadline = Instant::now() + Duration::from_secs(30);
    while dir.join("meshora.exe").exists() || Path::new(&uninstaller).exists() {
        if Instant::now() > deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    Ok(())
}

fn start_menu_link() -> PathBuf {
    let data = std::env::var_os("ProgramData")
        .map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from);
    data.join(r"Microsoft\Windows\Start Menu\Programs\Meshora.lnk")
}

fn desktop_link() -> PathBuf {
    let public =
        std::env::var_os("PUBLIC").map_or_else(|| PathBuf::from(r"C:\Users\Public"), PathBuf::from);
    public.join(r"Desktop\Meshora.lnk")
}

fn shortcut(target: &Path, link: &Path) -> Result<(), String> {
    let mut lnk = mslnk::ShellLink::new(target).map_err(|err| format!("建快捷方式失败：{err}"))?;
    if let Some(dir) = target.parent() {
        lnk.set_working_dir(Some(dir.display().to_string()));
    }
    lnk.create_lnk(link)
        .map_err(|err| format!("建快捷方式 {} 失败：{err}", link.display()))
}

/// 装的时候怎么选。
pub struct Options {
    /// 装到哪（已经按 `location::target_dir` 换算过）。
    pub dir: PathBuf,
    /// 桌面上放不放快捷方式。`None` 是照旧（更新时：原来有就留着，没有也不加）。
    pub desktop: Option<bool>,
    /// 是客户端自己发起的更新：先等它退出。
    pub update: bool,
}

/// 装。`progress(说明, 百分比)` 报进度
pub fn install(options: &Options, progress: &dyn Fn(&str, u8)) -> Result<(), String> {
    let files = payload::files()?;
    if !files.iter().any(|f| f.name == "meshora.exe") {
        return Err("这是开发时编的安装程序，没带着 Meshora 本身：用发布页上下载的那个".into());
    }
    let dir = options.dir.clone();
    // 界面传来的位置不直接信：再查一遍它和上面每一层普通账户都动不了
    acl::check_target(&dir)?;

    progress("关掉正在运行的 Meshora", 5);
    stop_running(if options.update {
        Duration::from_secs(15)
    } else {
        Duration::ZERO
    })?;
    progress("卸掉旧版本", 12);
    remove_nsis_install(&dir)?;

    progress("复制文件", 20);
    std::fs::create_dir_all(&dir).map_err(|err| format!("建不了 {}：{err}", dir.display()))?;
    // 先收紧权限再写文件：wintun.dll 一写进去，这个目录就得只有管理员能改
    acl::lock_down(&dir)?;
    let total = files.iter().map(|f| f.data.len()).sum::<usize>().max(1);
    let mut done = 0;
    for file in &files {
        let path = dir.join(&file.name);
        // 先写临时文件再改名：写到一半出错，不会留下半个 meshora.exe
        let temp = dir.join(format!("{}.new", file.name));
        std::fs::write(&temp, &file.data)
            .and_then(|()| std::fs::rename(&temp, &path))
            .map_err(|err| format!("写不了 {}：{err}", path.display()))?;
        done += file.data.len();
        progress("复制文件", 20 + (60 * done / total) as u8);
    }
    // 自己就是卸载程序
    let me = std::env::current_exe().map_err(|err| format!("找不到安装程序自己：{err}"))?;
    std::fs::copy(&me, dir.join("uninstall.exe"))
        .map_err(|err| format!("写不了卸载程序：{err}"))?;

    progress("建快捷方式", 85);
    let exe = dir.join("meshora.exe");
    shortcut(&exe, &start_menu_link())?;
    let desktop = desktop_link();
    match options.desktop {
        Some(true) => shortcut(&exe, &desktop)?,
        Some(false) => {
            let _ = std::fs::remove_file(&desktop);
        }
        // 照旧：原来的快捷方式指着同一个位置，不用动
        None => {}
    }

    progress("放行防火墙", 89);
    // 加不上不拦着装：官方服务器模式照样能用（打不通走中继），只是直连模式多半打不通
    let _ = firewall::allow(firewall::RULE, &exe);

    progress("登记到“应用和功能”", 92);
    let size_kb = (total / 1024) as u32;
    let (key, _) = RegKey::predef(HKEY_LOCAL_MACHINE)
        .create_subkey(UNINSTALL_KEY)
        .map_err(|err| format!("写不了注册表：{err}"))?;
    let uninstall = format!("\"{}\" --uninstall", dir.join("uninstall.exe").display());
    let set = |name: &str, value: &str| key.set_value(name, &value.to_owned());
    set("DisplayName", "Meshora")
        .and_then(|()| set("DisplayVersion", env!("CARGO_PKG_VERSION")))
        .and_then(|()| set("Publisher", "Meshora contributors"))
        .and_then(|()| set("DisplayIcon", &exe.display().to_string()))
        .and_then(|()| set("InstallLocation", &dir.display().to_string()))
        .and_then(|()| set("UninstallString", &uninstall))
        .and_then(|()| set("URLInfoAbout", "https://kerxs.github.io/meshora/"))
        .and_then(|()| key.set_value("NoModify", &1u32))
        .and_then(|()| key.set_value("NoRepair", &1u32))
        .and_then(|()| key.set_value("EstimatedSize", &size_kb))
        .and_then(|()| key.set_value(OURS, &1u32))
        .map_err(|err| format!("写不了注册表：{err}"))?;

    progress("装好了", 100);
    Ok(())
}

/// 卸。`purge` 时连本机的私钥和设置一起删（删了就是换了一个身份）
pub fn uninstall(purge: bool, progress: &dyn Fn(&str, u8)) -> Result<(), String> {
    // 卸载程序就在安装目录里：用它自己所在的目录，不靠注册表
    let dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
        .filter(|dir| dir.join("meshora.exe").exists())
        .unwrap_or_else(current_dir);
    progress("关掉正在运行的 Meshora", 10);
    stop_running(Duration::ZERO)?;

    progress("删快捷方式", 25);
    let _ = std::fs::remove_file(start_menu_link());
    let _ = std::fs::remove_file(desktop_link());

    progress("删文件", 45);
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            // 正在跑的就是 uninstall.exe：删不掉，退出之后再删
            if path.file_name().is_some_and(|n| n == "uninstall.exe") {
                continue;
            }
            let _ = if path.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
        }
    }

    progress("从“应用和功能”里拿掉", 75);
    firewall::remove(firewall::RULE);
    let _ = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
            KEY_ALL_ACCESS,
        )
        .and_then(|key| key.delete_subkey_all("Meshora"));

    if purge {
        progress("删私钥和设置", 88);
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let _ = std::fs::remove_dir_all(PathBuf::from(local).join("io.github.kerxs.meshora"));
        }
    }

    // 自己（uninstall.exe）和安装目录：等这个进程退出之后再删
    let script = format!(
        "ping 127.0.0.1 -n 3 > nul & del /f /q \"{}\" & rmdir \"{}\"",
        dir.join("uninstall.exe").display(),
        dir.display()
    );
    let _ = quiet("cmd.exe").arg("/c").raw_arg(&script).spawn();
    progress("卸载好了", 100);
    Ok(())
}

/// 装完打开 Meshora
pub fn launch() -> Result<(), String> {
    Command::new(current_dir().join("meshora.exe"))
        .spawn()
        .map(drop)
        .map_err(|err| format!("打不开 Meshora：{err}"))
}

/// WebView2 运行时装了没有。安装程序的界面靠它，客户端也靠它
pub fn webview2_installed() -> bool {
    const CLIENT: &str = r"Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
    let present = |hive: winreg::HKEY, path: String| {
        RegKey::predef(hive)
            .open_subkey(path)
            .and_then(|key| key.get_value::<String, _>("pv"))
            .is_ok_and(|v| !v.is_empty() && v != "0.0.0.0")
    };
    present(
        HKEY_LOCAL_MACHINE,
        format!(r"SOFTWARE\WOW6432Node\{CLIENT}"),
    ) || present(HKEY_LOCAL_MACHINE, format!(r"SOFTWARE\{CLIENT}"))
        || present(
            winreg::enums::HKEY_CURRENT_USER,
            format!(r"Software\{CLIENT}"),
        )
}
