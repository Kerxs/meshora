//! 两件事：
//!
//! 1. 把要装的文件打成一个载荷，编进安装程序。文件来自环境变量 `MESHORA_SETUP_PAYLOAD` 指的目录
//!    （CI 里放着 release 版的 meshora.exe、核对过的 wintun.dll 和许可证）。没给就是空载荷：
//!    开发时界面照常能看，只是装不了
//! 2. Windows 上：界面文件、图标、要求管理员权限的清单（tauri-build 做）
//!
//! 载荷的格式很简单，整个用 deflate 压一遍：`MSHP1`，文件个数（u32），
//! 每个文件是 名字长度（u16）、名字（UTF-8）、内容长度（u64）、内容。都是小端

use std::io::Write as _;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=MESHORA_SETUP_PAYLOAD");
    let mut raw = Vec::new();
    raw.extend_from_slice(b"MSHP1");
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    if let Some(dir) = std::env::var_os("MESHORA_SETUP_PAYLOAD").map(PathBuf::from) {
        println!("cargo:rerun-if-changed={}", dir.display());
        let mut entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap_or_else(|err| panic!("读不了载荷目录 {}：{err}", dir.display()))
            .map(|entry| entry.expect("载荷目录里的条目").path())
            .filter(|path| path.is_file())
            .collect();
        entries.sort();
        for path in entries {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .expect("载荷里的文件名要是 UTF-8")
                .to_owned();
            let data = std::fs::read(&path).expect("读载荷文件");
            files.push((name, data));
        }
        for required in ["meshora.exe", "wintun.dll"] {
            assert!(
                files.iter().any(|(name, _)| name == required),
                "载荷目录 {} 里缺 {required}",
                dir.display()
            );
        }
    }
    raw.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for (name, data) in &files {
        raw.extend_from_slice(&(name.len() as u16).to_le_bytes());
        raw.extend_from_slice(name.as_bytes());
        raw.extend_from_slice(&(data.len() as u64).to_le_bytes());
        raw.extend_from_slice(data);
    }
    let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(&raw).expect("压缩载荷");
    let packed = encoder.finish().expect("压缩载荷");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo 给的 OUT_DIR"));
    std::fs::write(out.join("payload.bin"), packed).expect("写载荷");

    #[cfg(windows)]
    {
        // 装到 Program Files、写注册表、建所有人的快捷方式：都要管理员权限。
        // 开发时设 MESHORA_SETUP_AS_INVOKER=1 编译就不要求：窗口能开、界面能看，真装会失败
        println!("cargo:rerun-if-env-changed=MESHORA_SETUP_AS_INVOKER");
        let manifest = include_str!("setup.manifest");
        let manifest = if std::env::var_os("MESHORA_SETUP_AS_INVOKER").is_some() {
            manifest.replace("requireAdministrator", "asInvoker")
        } else {
            manifest.to_owned()
        };
        let windows = tauri_build::WindowsAttributes::new().app_manifest(manifest);
        tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
            .expect("tauri-build 失败");
    }
}
