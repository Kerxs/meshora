//! 更新：查有没有新版本、下载、核对、装。
//!
//! 每个 Release 附一个 `latest.json`（CI 的 release job 生成）：
//!
//! ```json
//! { "version": "1.0.1", "notes": "……",
//!   "windows": { "url": "…/Meshora-1.0.1-setup.exe", "sha256": "…", "signature": "…" },
//!   "android": { "url": "…/Meshora-1.0.1-android.apk", "sha256": "…", "signature": "…" } }
//! ```
//!
//! `signature` 是 Ed25519 签名，签的是 `meshora-update\n<平台>\n<版本>\n<sha256>`。版本号也签进去：
//! 能改 `latest.json` 的人换不成一个旧的（同样签过名、但有漏洞的）安装包冒充新版本。
//! 公钥写死在 [`PUBLIC_KEY`]，私钥只在仓库的 Secret 里，CI 发版时签。
//!
//! - Windows：下载安装程序，核对哈希和签名，带 `--update` 运行它：它关掉客户端、装好、再打开
//! - 安卓：App 不能自己装别的 APK，交给浏览器下载，系统安装器装。系统会核对新 APK 和装着的是同一把钥匙签的

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};

/// 最新版本的更新清单：GitHub 把 `latest` 重定向到最新的那个 Release。
pub const MANIFEST_URL: &str =
    "https://github.com/Kerxs/meshora/releases/latest/download/latest.json";

/// 给更新清单签名的公钥（Ed25519，32 字节，base64）。
pub const PUBLIC_KEY: &str = "cv155EK/WWB2Ymq2UA0OAY5ClaYiHZYJMHIkQ9t2jrU=";

/// 这个客户端在清单里找哪一项。
pub const PLATFORM: &str = if cfg!(target_os = "android") {
    "android"
} else {
    "windows"
};

/// 当前版本。
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// 清单最大多少字节：比这大的一定不对
const MAX_MANIFEST: u64 = 64 * 1024;
/// 安装包最大多少字节
#[cfg(windows)]
const MAX_PACKAGE: u64 = 256 * 1024 * 1024;

/// 清单里一个平台的安装包。
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Package {
    /// 下载地址。
    pub url: String,
    /// SHA-256，小写十六进制。
    pub sha256: String,
    /// 见模块说明。base64。
    pub signature: String,
}

#[derive(Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    notes: String,
    windows: Option<Package>,
    android: Option<Package>,
}

/// 有一个比现在新、签名对得上的版本。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Available {
    /// 版本号。
    pub version: String,
    /// 这一版改了什么（发布说明的摘要）。
    pub notes: String,
    /// 这个平台的安装包。
    pub package: Package,
}

/// 被签名的那段话。
pub fn signed_message(platform: &str, version: &str, sha256: &str) -> String {
    format!("meshora-update\n{platform}\n{version}\n{sha256}")
}

/// `1.2.3`、`v1.2.3`、`1.2.3-beta.1` 拆成（主、次、修订、是不是预发布）
fn parse_version(text: &str) -> Option<(u64, u64, u64, bool)> {
    let text = text.trim().trim_start_matches('v');
    let (core, pre) = match text.split_once('-') {
        Some((core, _)) => (core, true),
        None => (text, false),
    };
    let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??, pre);
    parts.next().is_none().then_some(version)
}

/// `candidate` 比 `current` 新吗。认不出的版本号一律当不新。
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some((a, b, c, pre_new)), Some((x, y, z, pre_old))) => {
            (a, b, c) > (x, y, z) || ((a, b, c) == (x, y, z) && pre_old && !pre_new)
        }
        _ => false,
    }
}

/// 核对签名。
pub fn verify(
    platform: &str,
    version: &str,
    package: &Package,
    public_key: &str,
) -> Result<(), String> {
    let key = BASE64
        .decode(public_key)
        .map_err(|_| "更新公钥坏了".to_string())?;
    let signature = BASE64
        .decode(package.signature.trim())
        .map_err(|_| "更新清单里的签名不是 base64".to_string())?;
    let message = signed_message(platform, version, &package.sha256);
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
        .verify(message.as_bytes(), &signature)
        .map_err(|_| "更新清单的签名对不上：不是 Meshora 发的，不装".to_string())
}

/// 读清单：有没有这个平台的、比 `current` 新的、签名对得上的版本。
pub fn evaluate(
    manifest: &[u8],
    current: &str,
    platform: &str,
    public_key: &str,
) -> Result<Option<Available>, String> {
    let manifest: Manifest =
        serde_json::from_slice(manifest).map_err(|err| format!("更新清单读不懂：{err}"))?;
    if !is_newer(&manifest.version, current) {
        return Ok(None);
    }
    let package = match platform {
        "android" => manifest.android,
        _ => manifest.windows,
    };
    let Some(package) = package else {
        return Ok(None);
    };
    if !package.url.starts_with("https://") {
        return Err("更新清单里的下载地址不是 https".into());
    }
    verify(platform, &manifest.version, &package, public_key)?;
    Ok(Some(Available {
        version: manifest.version,
        notes: manifest.notes.chars().take(2000).collect(),
        package,
    }))
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .into()
}

fn fetch(url: &str, timeout: Duration, limit: u64) -> Result<Vec<u8>, String> {
    agent(timeout)
        .get(url)
        .call()
        .map_err(|err| format!("连不上 GitHub：{err}"))?
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|err| format!("下载中断了：{err}"))
}

/// 查一次（阻塞，放在 `spawn_blocking` 里调）。
pub fn check() -> Result<Option<Available>, String> {
    let manifest = fetch(MANIFEST_URL, Duration::from_secs(20), MAX_MANIFEST)?;
    evaluate(&manifest, CURRENT, PLATFORM, PUBLIC_KEY)
}

fn sha256_hex(data: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, data)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 下载安装程序、核对哈希，存到临时目录，交回路径（阻塞）。签名在查的时候已经核对过了：
/// 哈希对上，文件就是签过名的那个
#[cfg(windows)]
pub fn download(available: &Available) -> Result<std::path::PathBuf, String> {
    let data = fetch(
        &available.package.url,
        Duration::from_secs(600),
        MAX_PACKAGE,
    )?;
    if sha256_hex(&data) != available.package.sha256.trim().to_ascii_lowercase() {
        return Err("下载下来的安装程序和更新清单里的哈希对不上，不装".into());
    }
    let dir = std::env::temp_dir().join("Meshora-update");
    std::fs::create_dir_all(&dir).map_err(|err| format!("建不了临时目录：{err}"))?;
    let path = dir.join(format!("Meshora-{}-setup.exe", available.version));
    std::fs::write(&path, &data).map_err(|err| format!("存不下安装程序：{err}"))?;
    Ok(path)
}

/// 运行下载好的安装程序（`--update`：不用点，装完自己把 Meshora 打开）。调用方随后退出
#[cfg(windows)]
pub fn launch_installer(path: &std::path::Path) -> Result<(), String> {
    std::process::Command::new(path)
        .arg("--update")
        .spawn()
        .map(drop)
        .map_err(|err| format!("运行安装程序失败：{err}"))
}

/// 更新进行到哪了。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Status {
    /// 还没查过。
    #[default]
    Unknown,
    /// 正在查。
    Checking,
    /// 已经是最新的。
    UpToDate,
    /// 有新版本。
    Available(Available),
    /// 正在下载。
    Downloading(Available),
    /// 查或下载失败了。
    Failed(String),
}

/// 给界面看的。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateView {
    /// `unknown`、`checking`、`upToDate`、`available`、`downloading`、`failed`。
    pub status: &'static str,
    /// 新版本号。
    pub version: Option<String>,
    /// 新版本改了什么。
    pub notes: Option<String>,
    /// 出错时的原因。
    pub error: Option<String>,
}

/// 更新的状态。克隆出来的都是同一份。
#[derive(Clone, Default)]
pub struct Updater {
    status: Arc<Mutex<Status>>,
}

impl Updater {
    fn lock(&self) -> MutexGuard<'_, Status> {
        self.status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 现在的状态。
    pub fn status(&self) -> Status {
        self.lock().clone()
    }

    /// 改状态。
    pub fn set(&self, status: Status) {
        *self.lock() = status;
    }

    /// 给界面看的样子。
    pub fn view(&self) -> UpdateView {
        let status = self.status();
        let (name, available, error) = match status {
            Status::Unknown => ("unknown", None, None),
            Status::Checking => ("checking", None, None),
            Status::UpToDate => ("upToDate", None, None),
            Status::Available(available) => ("available", Some(available), None),
            Status::Downloading(available) => ("downloading", Some(available), None),
            Status::Failed(error) => ("failed", None, Some(error)),
        };
        UpdateView {
            status: name,
            version: available.as_ref().map(|a| a.version.clone()),
            notes: available.map(|a| a.notes),
            error,
        }
    }

    /// 查一次，结果记下来。正在下载时不查
    pub async fn check(&self) {
        if matches!(self.status(), Status::Downloading(_)) {
            return;
        }
        self.set(Status::Checking);
        let result = tokio::task::spawn_blocking(check)
            .await
            .unwrap_or_else(|err| Err(format!("查更新的任务出错：{err}")));
        self.set(match result {
            Ok(Some(available)) => {
                tracing::info!(version = %available.version, "有新版本");
                Status::Available(available)
            }
            Ok(None) => Status::UpToDate,
            Err(err) => {
                tracing::warn!(%err, "查更新失败");
                Status::Failed(err)
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use ring::signature::{Ed25519KeyPair, KeyPair};

    use super::*;

    fn keypair() -> (Ed25519KeyPair, String) {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let public = BASE64.encode(pair.public_key().as_ref());
        (pair, public)
    }

    fn manifest(pair: &Ed25519KeyPair, signed_version: &str, version: &str) -> Vec<u8> {
        let sha = sha256_hex(b"installer");
        let sign = |platform| {
            BASE64.encode(
                pair.sign(signed_message(platform, signed_version, &sha).as_bytes())
                    .as_ref(),
            )
        };
        serde_json::json!({
            "version": version,
            "notes": "修了一些东西",
            "windows": { "url": "https://example.com/setup.exe", "sha256": sha, "signature": sign("windows") },
            "android": { "url": "https://example.com/app.apk", "sha256": sha, "signature": sign("android") },
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn versions_compare() {
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(is_newer("v1.1.0", "1.0.9"));
        assert!(is_newer("2.0.0", "1.99.99"));
        assert!(is_newer("1.0.0", "1.0.0-beta.1"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("1.0.0-beta.1", "1.0.0"));
        assert!(!is_newer("0.9.0", "1.0.0"));
        assert!(!is_newer("garbage", "1.0.0"));
        assert!(!is_newer("1.0", "0.1.0"));
    }

    #[test]
    fn a_signed_newer_version_is_offered_for_this_platform() {
        let (pair, public) = keypair();
        let found = evaluate(
            &manifest(&pair, "1.0.1", "1.0.1"),
            "1.0.0",
            "windows",
            &public,
        )
        .unwrap()
        .unwrap();
        assert_eq!(found.version, "1.0.1");
        assert_eq!(found.package.url, "https://example.com/setup.exe");
        let found = evaluate(
            &manifest(&pair, "1.0.1", "1.0.1"),
            "1.0.0",
            "android",
            &public,
        )
        .unwrap()
        .unwrap();
        assert_eq!(found.package.url, "https://example.com/app.apk");
    }

    #[test]
    fn the_same_or_older_version_is_not_offered() {
        let (pair, public) = keypair();
        assert_eq!(
            evaluate(
                &manifest(&pair, "1.0.0", "1.0.0"),
                "1.0.0",
                "windows",
                &public
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn a_wrong_key_or_a_relabelled_version_is_refused() {
        let (pair, _) = keypair();
        let (_, other) = keypair();
        assert!(
            evaluate(
                &manifest(&pair, "1.0.1", "1.0.1"),
                "1.0.0",
                "windows",
                &other
            )
            .is_err()
        );
        // 签的是 1.0.1，清单上写成 9.9.9：拿旧包冒充新版本
        let (pair, public) = keypair();
        assert!(
            evaluate(
                &manifest(&pair, "1.0.1", "9.9.9"),
                "1.0.0",
                "windows",
                &public
            )
            .is_err()
        );
    }

    /// CI 用 `openssl pkeyutl -sign -rawin` 签（.github/workflows/package.yml），这里用 ring 验：
    /// 这一条是拿真的发版私钥签的，两边对得上
    #[test]
    fn a_signature_made_by_openssl_with_the_release_key_verifies() {
        let package = Package {
            url: "https://example.com/setup.exe".into(),
            sha256: "abc".into(),
            signature: "I75TdpM85f532WMBmbVyP3bLacDtnpD3/P5Edyvph9VfXmip6EEBSZAPhyG8K7PXiifMgMR8QPcVry+xvApoBw==".into(),
        };
        verify("windows", "0.0.1", &package, PUBLIC_KEY).unwrap();
        assert!(verify("windows", "0.0.2", &package, PUBLIC_KEY).is_err());
    }

    /// 和 CI 的 release job 一样的命令（openssl 签、同样的格式）拿真的发版私钥生成的清单：
    /// 两个平台都认得出、签名都对得上
    #[test]
    fn a_manifest_in_the_release_format_verifies_with_the_shipped_key() {
        let manifest = r#"{"version": "9.9.9", "notes": "Meshora 9.9.9", "windows": {"url": "https://github.com/Kerxs/meshora/releases/download/v9.9.9/Meshora-9.9.9-setup.exe", "sha256": "6ce765c53e97f87ed2607ed3604a36bdc9b40d83ce1b642f46659be869c0c5bc", "signature": "y9+5VGt5k0TOFOa/EmqPjetVl0mHJmXX8tw2dNpfiioQmmQLRN9rm2sY/Zv8q4VVh7vjP6zpXBZsM07uNQwUAg=="}, "android": {"url": "https://github.com/Kerxs/meshora/releases/download/v9.9.9/Meshora-9.9.9-android.apk", "sha256": "889f328a46135baf1649a2a70638b179354535550150076e5a6c986f7beb663b", "signature": "SVyzxbeZaJuVskH1K75e10Yl+ViDzoVa6t+MA4pIPEpn5jr+OXkZF4LWNvAw3jkpC9sJKF68+b8bQehjDrzfBw=="}}"#;
        for platform in ["windows", "android"] {
            let found = evaluate(manifest.as_bytes(), "1.0.0", platform, PUBLIC_KEY)
                .unwrap()
                .unwrap();
            assert_eq!(found.version, "9.9.9");
        }
    }

    #[test]
    fn the_shipped_public_key_is_a_valid_ed25519_key() {
        assert_eq!(BASE64.decode(PUBLIC_KEY).unwrap().len(), 32);
    }

    #[test]
    fn sha256_is_lowercase_hex() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
