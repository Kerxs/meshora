//! 客户端存在本机的东西：私钥和设置，都在一个目录里（Windows 上是 `%LOCALAPPDATA%` 下的应用目录）。

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use meshora_types::NodeSecret;
use serde::{Deserialize, Serialize};
use tracing::warn;

const KEY_FILE: &str = "node.key";
const SETTINGS_FILE: &str = "settings.json";

/// 用户的设置。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// 最近一次加入的网络的网络码。
    pub network: Option<String>,
    /// 让游戏的广播和组播走 Meshora：把虚拟网卡的跃点数压到最低（见 `meshora_tun::TunConfig::metric`）。
    pub prefer_broadcast: bool,
    /// 打开客户端时自动连上次的网络。
    pub auto_connect: bool,
    /// 把 Meshora 的网卡设成 Windows 的"专用网络"（见 `meshora_tun::set_network_private`）。
    /// 有安全上的代价，默认关着，由用户决定。
    pub private_network: bool,
    /// 给网里别人看的名字。第一次打开时取这台电脑的名字（Windows 的计算机名）。
    pub name: String,
    /// 走完了第一次打开时的引导。
    pub onboarded: bool,
    /// 自己添加的服务器（托管很多网络的协调服务）：`公钥@地址:端口`，建网络时可以选。
    pub servers: Vec<String>,
    /// 官方服务器的地址，覆盖客户端里写死的那个（自建了一台"官方服务器"的人用）。
    pub official_server: Option<String>,
    /// 现在的网络是本机当主机建的：连之前先把本机的协调服务、中继起起来。
    pub hosting: bool,
}

/// 这台电脑的名字，取不到是空串。安卓上没有计算机名可取，叫"我的手机"
fn computer_name() -> String {
    if cfg!(target_os = "android") {
        return "我的手机".into();
    }
    let name = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_default();
    name.trim().chars().take(MAX_NAME_CHARS).collect()
}

/// 名字最多多少个字符。协调服务也会截，这里先截，免得界面上显示的和别人看到的不一样
pub const MAX_NAME_CHARS: usize = 32;

impl Default for Settings {
    fn default() -> Self {
        Self {
            network: None,
            prefer_broadcast: true,
            auto_connect: true,
            private_network: false,
            name: computer_name(),
            onboarded: false,
            servers: Vec::new(),
            official_server: None,
            hosting: false,
        }
    }
}

/// 存放私钥和设置的目录。
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// 用这个目录。还不存在也没关系，第一次写的时候建。
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// 这个目录。
    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    /// 本机的私钥。第一次运行时生成一把存起来 —— 私钥就是这台电脑在网里的身份，换了就是另一个人。
    pub fn load_or_create_key(&self) -> io::Result<NodeSecret> {
        let path = self.dir.join(KEY_FILE);
        match fs::read(&path) {
            Ok(contents) => NodeSecret::from_key_file(&contents).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{} 不是合法的私钥", path.display()),
                )
            }),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                fs::create_dir_all(&self.dir)?;
                let secret = NodeSecret::generate();
                // create_new：万一两份客户端同时第一次启动，后到的那个不会覆盖先到的身份
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)?;
                file.write_all(secret.to_base64().as_bytes())?;
                file.write_all(b"\n")?;
                Ok(secret)
            }
            Err(err) => Err(err),
        }
    }

    /// 读设置。文件不在或者坏了就用默认值 —— 设置坏了不该让客户端打不开。
    pub fn load_settings(&self) -> Settings {
        let mut settings = self.read_settings();
        // 1.0.0 的设置文件没有这一项：已经加入过网络的人不用再走引导
        if settings.network.is_some() {
            settings.onboarded = true;
        }
        settings
    }

    fn read_settings(&self) -> Settings {
        let path = self.dir.join(SETTINGS_FILE);
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|err| {
                warn!(%err, path = %path.display(), "设置文件解不开，用默认设置");
                Settings::default()
            }),
            Err(err) => {
                if err.kind() != io::ErrorKind::NotFound {
                    warn!(%err, path = %path.display(), "读设置文件失败，用默认设置");
                }
                Settings::default()
            }
        }
    }

    /// 存设置。先写临时文件再改名，写到一半断电也不会留下半个文件。
    pub fn save_settings(&self, settings: &Settings) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(SETTINGS_FILE);
        let temp = self.dir.join(format!("{SETTINGS_FILE}.tmp"));
        let json = serde_json::to_vec_pretty(settings).map_err(io::Error::other)?;
        fs::write(&temp, json)?;
        fs::rename(&temp, &path)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 测试用的临时目录，用完删掉
    pub(crate) struct TempDir(pub(crate) PathBuf);

    impl TempDir {
        pub(crate) fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("meshora-desktop-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_key_is_created_once_and_kept() {
        let dir = TempDir::new("key");
        let store = Store::new(&dir.0);
        let first = store.load_or_create_key().unwrap();
        let again = store.load_or_create_key().unwrap();
        assert_eq!(first.public_key(), again.public_key());
    }

    #[test]
    fn a_corrupt_key_is_an_error_not_a_new_identity() {
        let dir = TempDir::new("corrupt");
        fs::create_dir_all(&dir.0).unwrap();
        fs::write(dir.0.join(KEY_FILE), "garbage").unwrap();
        assert!(Store::new(&dir.0).load_or_create_key().is_err());
        assert_eq!(fs::read(dir.0.join(KEY_FILE)).unwrap(), b"garbage");
    }

    #[test]
    fn settings_round_trip_and_fall_back_to_defaults() {
        let dir = TempDir::new("settings");
        let store = Store::new(&dir.0);
        assert_eq!(store.load_settings(), Settings::default());

        let settings = Settings {
            network: Some("code".into()),
            prefer_broadcast: false,
            auto_connect: true,
            private_network: true,
            name: "阿杰的台式机".into(),
            onboarded: true,
            servers: vec!["key@play.example.com:7443".into()],
            official_server: None,
            hosting: true,
        };
        store.save_settings(&settings).unwrap();
        assert_eq!(store.load_settings(), settings);

        fs::write(dir.0.join(SETTINGS_FILE), "{ not json").unwrap();
        assert_eq!(store.load_settings(), Settings::default());

        // 老版本存的文件缺字段：缺的用默认值
        fs::write(dir.0.join(SETTINGS_FILE), r#"{"network":"x"}"#).unwrap();
        let loaded = store.load_settings();
        assert_eq!(loaded.network.as_deref(), Some("x"));
        assert!(loaded.prefer_broadcast);
        assert!(!loaded.private_network, "专用网络默认不开");
    }
}
