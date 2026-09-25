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
