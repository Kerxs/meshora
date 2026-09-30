//! 命令行的 pubkey。不需要管理员权限，Linux 和 Windows 都跑。

use std::fs;
use std::process::Command;

use meshora_types::NodeSecret;

#[test]
fn pubkey_reads_a_key_file() {
    let dir = std::env::temp_dir().join(format!("meshora-coord-cli-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let secret = NodeSecret::generate();

    // 普通的 UTF-8，以及 Windows PowerShell 5.1 的 > 写出来的 UTF-16LE
    let utf8 = format!("{}\n", *secret.to_base64()).into_bytes();
    let utf16: Vec<u8> = [0xFF, 0xFE]
        .into_iter()
        .chain(
            format!("{}\r\n", *secret.to_base64())
                .encode_utf16()
                .flat_map(u16::to_le_bytes),
        )
        .collect();
    for (name, contents) in [("utf8.key", utf8), ("utf16.key", utf16)] {
        let key = dir.join(name);
        fs::write(&key, contents).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_meshora-coord"))
            .arg("pubkey")
            .arg(&key)
            .output()
            .unwrap();
        assert!(output.status.success(), "{name}");
        let printed = String::from_utf8(output.stdout).unwrap();
        assert_eq!(
            printed.trim_end(),
            secret.public_key().to_string(),
            "{name}"
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

/// 真的启动一次、日志开着。库的测试里没有日志订阅者，日志语句里的表达式根本不会执行 ——
/// 曾经有一条启动日志在同一条语句里两次加同一把锁，测试全过，真程序一启动就卡死
#[test]
fn starts_up_with_logging_on() {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    use std::sync::mpsc;
    use std::time::Duration;

    let dir = std::env::temp_dir().join(format!("meshora-coord-start-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let key = dir.join("coord.key");
    fs::write(&key, format!("{}\n", *NodeSecret::generate().to_base64())).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_meshora-coord"))
        .arg("--key")
        .arg(&key)
        .args(["--listen", "127.0.0.1:0", "--state"])
        .arg(dir.join("coord.state"))
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });

    let mut seen = Vec::new();
    let started = loop {
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(line) if line.contains("协调服务启动") => break true,
            Ok(line) => seen.push(line),
            Err(_) => break false,
        }
    };
    let _ = child.kill();
    let _ = child.wait();
    let _ = fs::remove_dir_all(&dir);
    assert!(started, "10 秒内没打出启动日志，之前的输出：{seen:#?}");
    assert!(
        seen.iter()
            .any(|line| line.contains("网络码") && line.contains('#')),
        "启动前应该打出带邀请码的网络码：{seen:#?}"
    );
}
