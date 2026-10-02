//! Windows 防火墙：放行 Meshora 收 UDP。
//!
//! 打洞要对方的报文能进来。客户端以管理员身份运行，第一次在"公用网络"上收 UDP 时，
//! Windows 可能直接给它记一条"阻止"规则（也可能弹窗问，被点了取消）。连官方服务器时打不通会退回中继，
//! 看不出来；直连模式没有中继，就一直卡在"正在打洞"。所以装的时候加一条只针对 `meshora.exe`、只放 UDP 的入站规则。
//!
//! 放进来的报文客户端都先认证：WireGuard 握手认公钥，控制报文用双方公钥加密，STUN 回应只认自己发过的事务。
//!
//! 用 `netsh advfirewall`，从 system32 的绝对路径调用（同 `icacls`）。规则按名字管：先删同名的再加，装几遍都只有一条。

use std::os::windows::process::CommandExt as _;
use std::path::Path;
use std::process::{Command, Output};

use crate::acl::system32;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 规则的名字，在"高级安全 Windows Defender 防火墙"的入站规则里看得到。
pub const RULE: &str = "Meshora";

fn netsh(args: &str) -> std::io::Result<Output> {
    Command::new(system32().join("netsh.exe"))
        // 参数原样传：program="C:\Program Files\..." 要带引号，标准库的转义会把引号放错地方
        .raw_arg(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
}

fn failed(what: &str, output: &Output) -> String {
    let text = String::from_utf8_lossy(&output.stdout);
    format!("{what}失败：{}", text.trim())
}

/// 放行 `program` 收 UDP（所有网络类别）。已经有同名规则的话先删掉，换成指向这个程序的。
pub fn allow(name: &str, program: &Path) -> Result<(), String> {
    remove(name);
    let args = format!(
        "advfirewall firewall add rule name=\"{name}\" dir=in action=allow program=\"{}\" protocol=UDP profile=any enable=yes",
        program.display()
    );
    let output = netsh(&args).map_err(|err| format!("运行 netsh 失败：{err}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(failed("添加防火墙规则", &output))
    }
}

/// 删掉这条规则。没有也不算错。
pub fn remove(name: &str) {
    let _ = netsh(&format!("advfirewall firewall delete rule name=\"{name}\""));
}

/// 这条规则在不在（测试和排查用）。在的话交回 netsh 打出来的详情。
pub fn show(name: &str) -> Option<String> {
    let output = netsh(&format!(
        "advfirewall firewall show rule name=\"{name}\" verbose"
    ))
    .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "要管理员权限"]
    fn a_rule_is_added_replaced_and_removed() {
        let name = format!("Meshora test {}", std::process::id());
        let program = Path::new(r"C:\Program Files\Meshora Test\meshora.exe");
        allow(&name, program).unwrap();
        // 再加一遍：还是只有一条
        allow(&name, program).unwrap();
        let shown = show(&name).expect("规则应该在");
        assert!(
            shown.contains(r"C:\Program Files\Meshora Test\meshora.exe"),
            "{shown}"
        );
        assert_eq!(
            shown.matches(&name).count(),
            1,
            "同名规则只应有一条：{shown}"
        );
        assert!(shown.contains("UDP"), "{shown}");
        remove(&name);
        assert!(show(&name).is_none(), "删掉之后不该还在");
    }
}
