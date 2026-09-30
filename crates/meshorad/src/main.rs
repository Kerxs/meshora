//! meshorad：Meshora 的节点守护进程。
//!
//! ```text
//! meshorad genkey [文件]              生成私钥：写进文件并打出公钥；不给文件就把私钥打到标准输出
//! meshorad pubkey [文件]              读私钥（从文件，不给就从标准输入），打出公钥
//! meshorad up --key <文件> --join <网络码> [选项]
//! ```
//!
//! `up` 做的事在库里（[`meshorad::start`]），桌面端也用它。需要管理员权限（建虚拟网卡）。

use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::num::NonZeroU16;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use lexopt::prelude::*;
use meshora_types::{NodeKey, NodeSecret};
use meshorad::NetworkCode;
use tracing::info;
use zeroize::Zeroizing;

const USAGE: &str = "\
用法：
  meshorad genkey [文件]              生成一把私钥。给了文件就写进去（不覆盖已有的文件），
                                      标准输出打出对应的公钥；不给文件就把私钥打到标准输出
  meshorad pubkey [文件]              读私钥，打出对应的公钥。不给文件就从标准输入读
  meshorad up [选项]                  启动节点（需要管理员权限）

up 的选项：
  --key <文件>          私钥文件（必需）
  --join <网络码>       要加入的网络：协调服务的 公钥@地址:端口（网络码，建网络的人给你的）
  --coord <地址:端口>   协调服务的地址。和 --coord-key 一起用，可以代替 --join
  --coord-key <公钥>    协调服务的公钥
  --port <端口>         WireGuard 和控制报文共用的 UDP 端口，默认 41641，0 表示让系统挑
  --tun <名字>          虚拟网卡的名字，默认 meshora0
  --mtu <字节>          虚拟网卡的 MTU，默认 1280
  --keepalive <秒>      persistent keepalive，默认 25，0 表示关闭
  --relay-only          只走中继，不尝试直连
  -v, --verbose         打出调试日志
";

struct Up {
    key: PathBuf,
    network: NetworkCode,
    port: u16,
    tun: String,
    mtu: u16,
    keepalive: Option<NonZeroU16>,
    relay_only: bool,
    verbose: bool,
}

enum Command {
    GenKey(Option<PathBuf>),
    PubKey(Option<PathBuf>),
    Up(Up),
}

/// genkey、pubkey 可以带一个文件名
fn path_argument(parser: &mut lexopt::Parser) -> Result<Option<PathBuf>, lexopt::Error> {
    let mut path = None;
    while let Some(arg) = parser.next()? {
        match arg {
            Value(value) if path.is_none() => path = Some(PathBuf::from(value)),
            Long("help") | Short('h') => return Err(USAGE.into()),
            _ => return Err(arg.unexpected()),
        }
    }
    Ok(path)
}

fn parse() -> Result<Command, lexopt::Error> {
    let mut parser = lexopt::Parser::from_env();
    let command = match parser.next()? {
        Some(Value(value)) => value.string()?,
        Some(Long("help") | Short('h')) | None => return Err(USAGE.into()),
        Some(other) => return Err(other.unexpected()),
    };
    match command.as_str() {
        "genkey" => Ok(Command::GenKey(path_argument(&mut parser)?)),
        "pubkey" => Ok(Command::PubKey(path_argument(&mut parser)?)),
        "up" => {
            let (mut key, mut join, mut coord, mut coord_key) = (None, None, None, None);
            let mut up = Up {
                key: PathBuf::new(),
                network: NetworkCode::new(
                    NodeKey::from_bytes([0; 32]),
                    SocketAddr::from(([0, 0, 0, 0], 0)),
                ),
                port: 41641,
                tun: "meshora0".into(),
                mtu: 1280,
                keepalive: NonZeroU16::new(25),
                relay_only: false,
                verbose: false,
            };
            while let Some(arg) = parser.next()? {
                match arg {
                    Long("key") => key = Some(PathBuf::from(parser.value()?)),
                    Long("join") => join = Some(parser.value()?.parse()?),
                    Long("coord") => coord = Some(parser.value()?.parse()?),
                    Long("coord-key") => coord_key = Some(parser.value()?.parse()?),
                    Long("port") => up.port = parser.value()?.parse()?,
                    Long("tun") => up.tun = parser.value()?.string()?,
                    Long("mtu") => up.mtu = parser.value()?.parse()?,
                    Long("keepalive") => up.keepalive = NonZeroU16::new(parser.value()?.parse()?),
                    Long("relay-only") => up.relay_only = true,
                    Short('v') | Long("verbose") => up.verbose = true,
                    Long("help") | Short('h') => return Err(USAGE.into()),
                    _ => return Err(arg.unexpected()),
                }
            }
            up.key = key.ok_or("缺少 --key")?;
            up.network = match (join, coord, coord_key) {
                (Some(code), None, None) => code,
                (None, Some(coord), Some(coord_key)) => NetworkCode::new(coord_key, coord),
                (None, None, None) => {
                    return Err("缺少 --join（或者 --coord 加 --coord-key）".into());
                }
                (Some(_), _, _) => return Err("--join 和 --coord、--coord-key 只能二选一".into()),
                (None, _, _) => return Err("--coord 和 --coord-key 要一起给".into()),
            };
            Ok(Command::Up(up))
        }
        other => Err(format!("不认识的子命令 {other:?}\n\n{USAGE}").into()),
    }
}

fn main() -> ExitCode {
    let command = match parse() {
        Ok(command) => command,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(2);
        }
    };
    let result = match command {
        Command::GenKey(path) => genkey(path.as_deref()),
        Command::PubKey(path) => pubkey(path.as_deref()),
        Command::Up(up) => {
            let level = if up.verbose {
                tracing::Level::DEBUG
            } else {
                tracing::Level::INFO
            };
            tracing_subscriber::fmt()
                .with_max_level(level)
                .with_writer(io::stderr)
                .init();
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|err| err.to_string())
                .and_then(|runtime| runtime.block_on(run(up)))
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("meshorad: {err}");
            ExitCode::FAILURE
        }
    }
}

fn genkey(path: Option<&Path>) -> Result<(), String> {
    let secret = NodeSecret::generate();
    match path {
        None => println!("{}", *secret.to_base64()),
        Some(path) => {
            write_key_file(path, &secret)?;
            eprintln!("私钥已写入 {}，下面是对应的公钥：", path.display());
            println!("{}", secret.public_key());
        }
    }
    Ok(())
}

/// 新建私钥文件。同名文件已经存在就报错：覆盖私钥等于换了一个身份。
/// Unix 上建成只有自己能读写；Windows 上沿用所在目录的权限
fn write_key_file(path: &Path, secret: &NodeSecret) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|err| match err.kind() {
        io::ErrorKind::AlreadyExists => format!(
            "{} 已经存在，不覆盖：覆盖私钥等于换了一个身份。确实要换，先删掉它",
            path.display()
        ),
        _ => format!("创建 {} 失败：{err}", path.display()),
    })?;
    file.write_all(secret.to_base64().as_bytes())
        .and_then(|()| file.write_all(b"\n"))
        .map_err(|err| format!("写 {} 失败：{err}", path.display()))
}

fn pubkey(path: Option<&Path>) -> Result<(), String> {
    let secret = match path {
        Some(path) => load_key(path)?,
        None => {
            let mut contents = Zeroizing::new(Vec::new());
            io::stdin()
                .read_to_end(&mut contents)
                .map_err(|err| format!("读标准输入失败：{err}"))?;
            NodeSecret::from_key_file(&contents)
                .map_err(|_| "标准输入里不是合法的私钥：应为 44 个字符的标准 base64".to_string())?
        }
    };
    println!("{}", secret.public_key());
    Ok(())
}

fn load_key(path: &Path) -> Result<NodeSecret, String> {
    let contents = Zeroizing::new(
        std::fs::read(path).map_err(|err| format!("读私钥文件 {} 失败：{err}", path.display()))?,
    );
    warn_if_readable_by_others(path);
    NodeSecret::from_key_file(&contents).map_err(|_| {
        format!(
            "私钥文件 {} 的内容不是合法的私钥：应为 44 个字符的标准 base64（meshorad genkey 生成的那种）",
            path.display()
        )
    })
}

/// 私钥文件别人也能读就提醒一句（R7）。只提醒不拒绝：M1 还不管安装，权限由部署方负责
#[cfg(unix)]
fn warn_if_readable_by_others(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path)
        && meta.permissions().mode() & 0o077 != 0
    {
        tracing::warn!(
            path = %path.display(),
            "私钥文件对其他用户可读，建议 chmod 600"
        );
    }
}

#[cfg(not(unix))]
fn warn_if_readable_by_others(_path: &Path) {}

async fn run(up: Up) -> Result<(), String> {
    let secret = load_key(&up.key)?;
    let coord = up
        .network
        .resolve()
        .await
        .map_err(|err| format!("找不到协调服务 {}：{err}", up.network.host))?;
    let mut node = meshorad::start(meshorad::Options {
        secret,
        coord,
        coord_key: up.network.coord_key,
        port: up.port,
        tun: up.tun,
        mtu: up.mtu,
        metric: None,
        keepalive: up.keepalive,
        relay_only: up.relay_only,
    })
    .await
    .map_err(|err| err.to_string())?;

    tokio::select! {
        result = node.wait() => {
            result.map_err(|err| format!("控制面退出：{err}"))
        }
        _ = shutdown_signal() => {
            info!("收到退出信号");
            Ok(())
        }
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(err) => {
                tracing::error!(%err, "注册 SIGTERM 失败，只响应 Ctrl-C");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
