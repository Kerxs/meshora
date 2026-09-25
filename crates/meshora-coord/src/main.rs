//! meshora-coord：协调服务的可执行程序。可以顺带在同一个进程里跑一个中继。
//!
//! ```text
//! meshora-coord --key coord.key --listen 0.0.0.0:7443 \
//!     --probe 0.0.0.0:7443 --probe-public 203.0.113.5:7443 \
//!     --relay-listen 0.0.0.0:7444 --relay-public 203.0.113.5:7444 \
//!     --node <节点公钥> --node <节点公钥>
//! ```

use std::io::{self, Read};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ipnet::Ipv4Net;
use lexopt::prelude::*;
use meshora_coord::Config;
use meshora_proto::control::RelayInfo;
use meshora_types::{NodeKey, NodeSecret};
use tokio::net::{TcpListener, UdpSocket};
use tracing::info;
use zeroize::Zeroizing;

const USAGE: &str = "\
用法：meshora-coord [选项]

  --key <文件>              协调服务的私钥文件（必需）。meshorad genkey 可以生成
  --listen <地址:端口>      控制通道监听的 TCP 地址（必需）
  --node <公钥>             允许加入的节点，可以写多次（至少一个）。overlay 地址按这个顺序分配
  --overlay <网段>          overlay 网段，默认 100.64.0.0/10
  --probe <地址:端口>       端点探测监听的 UDP 地址
  --probe-public <地址:端口> 节点从外面访问探测端点用的地址，默认同 --probe
  --relay-listen <地址:端口> 在同一个进程里跑一个中继，监听这个 TCP 地址
  --relay-public <地址:端口> 节点访问这个中继用的地址，默认同 --relay-listen
  --relay <公钥>@<地址:端口> 告诉节点的外部中继，可以写多次。目前节点只用排第一的那个
                            （有 --relay-listen 时就是同一进程里的中继）
  -v, --verbose             打出调试日志

  meshora-coord pubkey [文件] 读私钥（从文件，不给就从标准输入），打出公钥（告诉节点用）
";

struct Args {
    key: PathBuf,
    listen: SocketAddr,
    nodes: Vec<NodeKey>,
    overlay: Ipv4Net,
    probe: Option<SocketAddr>,
    probe_public: Option<SocketAddr>,
    relay_listen: Option<SocketAddr>,
    relay_public: Option<SocketAddr>,
    relays: Vec<RelayInfo>,
    verbose: bool,
}

fn parse_relay(value: &str) -> Result<RelayInfo, String> {
    let (key, addr) = value
        .split_once('@')
        .ok_or_else(|| format!("--relay 要写成 公钥@地址:端口，收到的是 {value:?}"))?;
    Ok(RelayInfo {
        key: key
            .parse()
            .map_err(|err| format!("--relay 的公钥：{err}"))?,
        addr: addr
            .parse()
            .map_err(|err| format!("--relay 的地址：{err}"))?,
    })
}

enum Command {
    Serve(Box<Args>),
    PubKey(Option<PathBuf>),
}

fn parse() -> Result<Command, lexopt::Error> {
    let mut parser = lexopt::Parser::from_env();
    let (mut key, mut listen) = (None, None);
    let mut args = Args {
        key: PathBuf::new(),
        listen: SocketAddr::from(([0, 0, 0, 0], 0)),
        nodes: Vec::new(),
        overlay: "100.64.0.0/10".parse().expect("常量"),
        probe: None,
        probe_public: None,
        relay_listen: None,
        relay_public: None,
        relays: Vec::new(),
        verbose: false,
    };
    while let Some(arg) = parser.next()? {
        match arg {
            Value(value) if value == "pubkey" => {
                let mut path = None;
                while let Some(arg) = parser.next()? {
                    match arg {
                        Value(value) if path.is_none() => path = Some(PathBuf::from(value)),
                        Long("help") | Short('h') => return Err(USAGE.into()),
                        _ => return Err(arg.unexpected()),
                    }
                }
                return Ok(Command::PubKey(path));
            }
            Long("key") => key = Some(PathBuf::from(parser.value()?)),
            Long("listen") => listen = Some(parser.value()?.parse()?),
            Long("node") => args.nodes.push(parser.value()?.parse()?),
            Long("overlay") => args.overlay = parser.value()?.parse()?,
            Long("probe") => args.probe = Some(parser.value()?.parse()?),
            Long("probe-public") => args.probe_public = Some(parser.value()?.parse()?),
            Long("relay-listen") => args.relay_listen = Some(parser.value()?.parse()?),
            Long("relay-public") => args.relay_public = Some(parser.value()?.parse()?),
            Long("relay") => args.relays.push(parse_relay(&parser.value()?.string()?)?),
            Short('v') | Long("verbose") => args.verbose = true,
            Long("help") | Short('h') => return Err(USAGE.into()),
            _ => return Err(arg.unexpected()),
        }
    }
    args.key = key.ok_or("缺少 --key")?;
    args.listen = listen.ok_or("缺少 --listen")?;
    if args.nodes.is_empty() {
        return Err("至少要有一个 --node".into());
    }
    Ok(Command::Serve(Box::new(args)))
}

/// 告诉节点的地址必须是节点访问得到的具体地址，不能是 0.0.0.0 这种监听用的地址
fn advertised(
    listen: SocketAddr,
    public: Option<SocketAddr>,
    flag: &str,
) -> Result<SocketAddr, String> {
    let addr = public.unwrap_or(listen);
    if addr.ip().is_unspecified() {
        return Err(format!(
            "监听在 {listen}，节点不知道该往哪连：请用 {flag} 指定节点能访问到的地址"
        ));
    }
    Ok(addr)
}

fn main() -> ExitCode {
    let args = match parse() {
        Ok(Command::Serve(args)) => *args,
        Ok(Command::PubKey(path)) => return pubkey(path.as_deref()),
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(2);
        }
    };
    let level = if args.verbose {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_writer(io::stderr)
        .init();
    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| err.to_string())
        .and_then(|runtime| runtime.block_on(run(args)));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("meshora-coord: {err}");
            ExitCode::FAILURE
        }
    }
}

fn pubkey(path: Option<&Path>) -> ExitCode {
    let secret = match path {
        Some(path) => load_key(path),
        None => {
            let mut contents = Zeroizing::new(Vec::new());
            match io::stdin().read_to_end(&mut contents) {
                Ok(_) => NodeSecret::from_key_file(&contents).map_err(|_| {
                    "标准输入里不是合法的私钥：应为 44 个字符的标准 base64".to_string()
                }),
                Err(err) => Err(format!("读标准输入失败：{err}")),
            }
        }
    };
    match secret {
        Ok(secret) => {
            println!("{}", secret.public_key());
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

/// 读私钥文件。和 meshorad 一样认 BOM 和 UTF-16LE（Windows PowerShell 的 > 写出来的）
fn load_key(path: &Path) -> Result<NodeSecret, String> {
    let contents = Zeroizing::new(
        std::fs::read(path).map_err(|err| format!("读私钥文件 {} 失败：{err}", path.display()))?,
    );
    NodeSecret::from_key_file(&contents).map_err(|_| {
        format!(
            "私钥文件 {} 的内容不是合法的私钥：应为 44 个字符的标准 base64（meshorad genkey 生成的那种）",
            path.display()
        )
    })
}

async fn run(args: Args) -> Result<(), String> {
    let secret = load_key(&args.key)?;

    let listener = TcpListener::bind(args.listen)
        .await
        .map_err(|err| format!("监听 {} 失败：{err}", args.listen))?;

    let (probe_socket, probe) = match args.probe {
        Some(addr) => {
            let socket = UdpSocket::bind(addr)
                .await
                .map_err(|err| format!("绑定探测端口 {addr} 失败：{err}"))?;
            let public = advertised(addr, args.probe_public, "--probe-public")?;
            (Some(socket), Some(public))
        }
        None => (None, None),
    };

    let mut relays = args.relays;
    if let Some(addr) = args.relay_listen {
        let public = advertised(addr, args.relay_public, "--relay-public")?;
        let relay_listener = TcpListener::bind(addr)
            .await
            .map_err(|err| format!("中继监听 {addr} 失败：{err}"))?;
        // 中继用协调服务同一把密钥：两边的 Noise 前导不同（通道类型 C 和 R），握手互不相通
        let relay_config = meshora_relay::Config {
            secret: secret.clone(),
            nodes: args.nodes.clone(),
        };
        tokio::spawn(meshora_relay::serve(relay_config, relay_listener));
        info!(%addr, %public, "同一进程里的中继已启动");
        relays.insert(
            0,
            RelayInfo {
                key: secret.public_key(),
                addr: public,
            },
        );
    }

    let config = Config {
        secret,
        nodes: args.nodes,
        overlay: args.overlay,
        probe,
        relays,
    };
    tokio::select! {
        result = meshora_coord::serve(config, listener, probe_socket) => {
            result.map_err(|err| format!("协调服务退出：{err}"))
        }
        _ = tokio::signal::ctrl_c() => Ok(()),
    }
}
