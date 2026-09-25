//! meshorad：Meshora 的节点守护进程。
//!
//! ```text
//! meshorad genkey [文件]              生成私钥：写进文件并打出公钥；不给文件就把私钥打到标准输出
//! meshorad pubkey [文件]              读私钥（从文件，不给就从标准输入），打出公钥
//! meshorad up --key <文件> --coord <地址:端口> --coord-key <公钥> [选项]
//! ```
//!
//! `up` 的顺序：向协调服务注册拿到 overlay 地址 → 按这个地址建虚拟网卡 → 起数据面 → 跑控制面。
//! 需要管理员权限（建虚拟网卡）。

use std::io::{self, Read, Write};
use std::net::{SocketAddr, UdpSocket};
use std::num::NonZeroU16;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use lexopt::prelude::*;
use meshora_control::{Config, Session};
use meshora_tun::{Pipes, Tun, TunConfig};
use meshora_types::{NodeKey, NodeSecret};
use meshora_wg::{TunChannels, UserspaceDataPlane};
use tokio::sync::mpsc;
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
  --coord <地址:端口>   协调服务的地址（必需）
  --coord-key <公钥>    协调服务的公钥（必需）
  --port <端口>         WireGuard 和控制报文共用的 UDP 端口，默认 41641，0 表示让系统挑
  --tun <名字>          虚拟网卡的名字，默认 meshora0
  --mtu <字节>          虚拟网卡的 MTU，默认 1280
  --keepalive <秒>      persistent keepalive，默认 25，0 表示关闭
  --relay-only          只走中继，不尝试直连
  -v, --verbose         打出调试日志
";

struct Up {
    key: PathBuf,
    coord: SocketAddr,
    coord_key: NodeKey,
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
            let (mut key, mut coord, mut coord_key) = (None, None, None);
            let mut up = Up {
                key: PathBuf::new(),
                coord: SocketAddr::from(([0, 0, 0, 0], 0)),
                coord_key: NodeKey::from_bytes([0; 32]),
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
            up.coord = coord.ok_or("缺少 --coord")?;
            up.coord_key = coord_key.ok_or("缺少 --coord-key")?;
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
    info!(key = %secret.public_key(), "本机身份");

    let socket = UdpSocket::bind(("0.0.0.0", up.port))
        .map_err(|err| format!("绑定 UDP 端口 {} 失败：{err}", up.port))?;
    // --port 0 时由系统挑端口，上报给协调服务的得是实际绑上的那个
    let local_port = socket
        .local_addr()
        .map_err(|err| format!("读取 UDP 端口失败：{err}"))?
        .port();

    let session = Session::connect(Config {
        secret: secret.clone(),
        coord: up.coord,
        coord_key: up.coord_key,
        local_port,
        keepalive: up.keepalive,
        relay_only: up.relay_only,
    })
    .await
    .map_err(|err| format!("注册失败：{err}"))?;
    let welcome = session.welcome().clone();

    let tun = Tun::open(&TunConfig {
        name: up.tun.clone(),
        address: welcome.overlay_ip,
        prefix_len: welcome.prefix_len,
        mtu: up.mtu,
    })
    .map_err(|err| format!("创建虚拟网卡 {} 失败：{err}", up.tun))?;
    info!(
        tun = tun.name(),
        ip = %welcome.overlay_ip,
        prefix = welcome.prefix_len,
        "虚拟网卡已就绪"
    );
    let Pipes { from_tun, to_tun } = tun
        .spawn()
        .map_err(|err| format!("启动虚拟网卡失败：{err}"))?;

    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let sink = move |event| {
        let _ = events_tx.send(event);
    };
    let dataplane = UserspaceDataPlane::start(
        &secret,
        socket,
        TunChannels { from_tun, to_tun },
        Arc::new(sink),
    )
    .map_err(|err| format!("启动数据面失败：{err}"))?;

    tokio::select! {
        result = session.run(Arc::new(dataplane), events_rx) => {
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
