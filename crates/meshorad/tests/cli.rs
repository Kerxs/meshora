//! 命令行的 genkey / pubkey。不需要管理员权限，Linux 和 Windows 都跑。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn meshorad(args: &[&str], stdin: Option<&[u8]>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_meshorad"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    if let Some(bytes) = stdin {
        input.write_all(bytes).unwrap();
    }
    drop(input);
    child.wait_with_output().unwrap()
}

/// 成功，并且标准输出是一行 44 个字符的密钥
fn key_output(output: Output) -> String {
    assert!(
        output.status.success(),
        "失败了：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    let key = text.trim_end().to_string();
    assert_eq!(key.len(), 44, "{text:?}");
    key
}

/// 测试用的临时目录，用完删掉
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("meshorad-cli-{}-{name}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn arg(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[test]
fn genkey_writes_a_new_key_file_and_prints_its_public_key() {
    let dir = TempDir::new("genkey");
    let key = dir.file("node.key");

    let public = key_output(meshorad(&["genkey", arg(&key)], None));
    assert_eq!(key_output(meshorad(&["pubkey", arg(&key)], None)), public);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&key).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "私钥文件只有自己能读写");
    }

    // 已有的私钥文件不覆盖
    let before = fs::read(&key).unwrap();
    let again = meshorad(&["genkey", arg(&key)], None);
    assert!(!again.status.success());
    assert_eq!(fs::read(&key).unwrap(), before);
}

#[test]
fn genkey_without_a_file_prints_a_private_key_pubkey_accepts() {
    let private = key_output(meshorad(&["genkey"], None));
    let public = key_output(meshorad(&["pubkey"], Some(private.as_bytes())));
    assert_ne!(public, private);
}

/// Windows PowerShell 5.1 里 `meshorad genkey > node.key` 写出来的是带 BOM 的 UTF-16LE
#[test]
fn key_files_written_by_windows_powershell_work() {
    let dir = TempDir::new("utf16");
    let private = key_output(meshorad(&["genkey"], None));
    let expected = key_output(meshorad(&["pubkey"], Some(private.as_bytes())));

    let key = dir.file("node.key");
    let utf16: Vec<u8> = [0xFF, 0xFE]
        .into_iter()
        .chain(
            format!("{private}\r\n")
                .encode_utf16()
                .flat_map(u16::to_le_bytes),
        )
        .collect();
    fs::write(&key, utf16).unwrap();
    assert_eq!(key_output(meshorad(&["pubkey", arg(&key)], None)), expected);
}

#[test]
fn a_bad_key_file_is_reported_clearly() {
    let dir = TempDir::new("bad");
    let key = dir.file("node.key");
    fs::write(&key, "this is not a key\n").unwrap();
    let output = meshorad(&["pubkey", arg(&key)], None);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("不是合法的私钥"), "{stderr}");
}
