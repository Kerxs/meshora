//! 读、收紧文件夹的权限（Windows）。判断规则在 [`crate::location`]。

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
    DACL_SECURITY_INFORMATION, GetAce, GetAclInformation, INHERIT_ONLY_ACE,
    OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
};
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;

use crate::location::{Ace, Security, foreign_file, weakness};

/// 不弹控制台窗口
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// SID → 字符串（`S-1-5-32-544`）
fn sid_string(sid: PSID) -> Option<String> {
    let mut text = std::ptr::null_mut();
    // SAFETY: sid 来自 Windows 交给我们的安全描述符，还活着；成功时 text 由系统分配，下面用 LocalFree 还回去
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return None;
    }
    // SAFETY: text 是以 0 结尾的 UTF-16
    let len = (0..).take_while(|&i| unsafe { *text.add(i) } != 0).count();
    // SAFETY: 同上，长度刚量过
    let s = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, len) });
    // SAFETY: text 是 ConvertSidToStringSidW 分配的
    unsafe { LocalFree(text.cast()) };
    Some(s)
}

/// 读一个文件夹的所有者和权限表。
pub fn read(path: &Path) -> Result<Security, String> {
    let name = wide(path);
    let mut owner: PSID = std::ptr::null_mut();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: name 以 0 结尾、活到调用结束；出参都是我们自己的指针变量。成功时 descriptor 归我们，最后 LocalFree
    let status = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!("读不了 {} 的权限（错误 {status}）", path.display()));
    }
    let result = (|| {
        let owner = sid_string(owner).ok_or("读不出所有者")?;
        if dacl.is_null() {
            return Ok(Security {
                owner,
                allowed: None,
            });
        }
        let mut size = ACL_SIZE_INFORMATION {
            AceCount: 0,
            AclBytesInUse: 0,
            AclBytesFree: 0,
        };
        // SAFETY: dacl 指向描述符里的权限表；size 是对应信息类的结构体，长度如实告诉函数
        if unsafe {
            GetAclInformation(
                dacl,
                (&mut size as *mut ACL_SIZE_INFORMATION).cast(),
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        } == 0
        {
            return Err("读不了权限表".to_string());
        }
        let mut allowed = Vec::new();
        for index in 0..size.AceCount {
            let mut ace: *mut c_void = std::ptr::null_mut();
            // SAFETY: index 在 AceCount 之内；ace 指向权限表内部，跟着描述符活
            if unsafe { GetAce(dacl, index, &mut ace) } == 0 {
                continue;
            }
            // SAFETY: 每一条都以 ACE_HEADER 开头
            let header = unsafe { &*(ace as *const ACE_HEADER) };
            if u32::from(header.AceType) != ACCESS_ALLOWED_ACE_TYPE {
                continue;
            }
            // SAFETY: 类型是"允许"，结构就是 ACCESS_ALLOWED_ACE，SID 从 SidStart 开始
            let allowed_ace = unsafe { &*(ace as *const ACCESS_ALLOWED_ACE) };
            let sid = (&allowed_ace.SidStart as *const u32).cast_mut().cast();
            let Some(sid) = sid_string(sid) else {
                continue;
            };
            allowed.push(Ace {
                sid,
                mask: allowed_ace.Mask,
                inherit_only: u32::from(header.AceFlags) & INHERIT_ONLY_ACE != 0,
            });
        }
        Ok(Security {
            owner,
            allowed: Some(allowed),
        })
    })();
    // SAFETY: descriptor 是 GetNamedSecurityInfoW 分配的；owner、dacl 指向它里面，之后不再用
    unsafe { LocalFree(descriptor) };
    result
}

/// 装到 `target` 行不行：它的上一层要已经存在，上面每一层普通账户都动不了；`target` 已经存在的话，
/// 里面要么是空的，要么是 Meshora 装的。
pub fn check_target(target: &Path) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or("要装在一个文件夹里，比如 D:\\Meshora")?;
    if !parent.is_dir() {
        return Err(format!("{} 不存在：选一个已有的文件夹", parent.display()));
    }
    for ancestor in parent.ancestors() {
        let security = read(ancestor)?;
        if let Some(why) = weakness(&security, ancestor.parent().is_none()) {
            return Err(format!(
                "{} 普通账户也能改（{why}），装在这下面不安全：别的程序能把 Meshora 整个换掉。\
                 选磁盘根目录（比如 D:\\）或者 Program Files 下面",
                ancestor.display()
            ));
        }
    }
    if target.exists() {
        if !target.is_dir() {
            return Err(format!("{} 是个文件，不是文件夹", target.display()));
        }
        let names: Vec<String> = std::fs::read_dir(target)
            .map_err(|err| format!("读不了 {}：{err}", target.display()))?
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        if let Some(name) = foreign_file(&names) {
            return Err(format!(
                "{} 里已经有别的东西了（比如 {name}）：选一个空的，或者让它新建一个 Meshora 文件夹",
                target.display()
            ));
        }
    }
    Ok(())
}

fn system32() -> PathBuf {
    let mut buf = [0u16; 260];
    // SAFETY: 缓冲区是我们自己的，长度如实告诉函数
    let len = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if len == 0 || len > buf.len() {
        return PathBuf::from(r"C:\Windows\System32");
    }
    PathBuf::from(String::from_utf16_lossy(&buf[..len]))
}

fn icacls(dir: &Path, args: &[&str]) -> Result<(), String> {
    let output = Command::new(system32().join("icacls.exe"))
        .arg(dir)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|err| format!("运行 icacls 失败：{err}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "设不了 {} 的权限：{}",
            dir.display(),
            String::from_utf8_lossy(&output.stdout).trim()
        ))
    }
}

/// 收紧后只留这几个：管理员组、SYSTEM 完全控制，Users 只读和执行
const KEEP: [&str; 3] = ["S-1-5-32-544", "S-1-5-18", "S-1-5-32-545"];

/// 收紧安装目录：所有者改成管理员组，去掉继承来的权限，只给管理员、SYSTEM 完全控制，普通用户只读和执行；
/// 别的账户单独的条目（比如装的那个管理员账户自己的 —— 它没提权时也能用）一律删掉。里面已有的文件重置成
/// 跟着目录继承。改完连里面的文件一起读回来核对。SID 写成 `*S-1-…`：不受系统语言影响
pub fn lock_down(dir: &Path) -> Result<(), String> {
    icacls(dir, &["/setowner", "*S-1-5-32-544", "/C", "/Q"])?;
    icacls(
        dir,
        &[
            "/inheritance:r",
            "/grant:r",
            "*S-1-5-32-544:(OI)(CI)F",
            "*S-1-5-18:(OI)(CI)F",
            "*S-1-5-32-545:(OI)(CI)RX",
            "/C",
            "/Q",
        ],
    )?;
    let mut stray: Vec<String> = read(dir)?
        .allowed
        .unwrap_or_default()
        .into_iter()
        .map(|ace| ace.sid)
        .filter(|sid| !KEEP.contains(&sid.as_str()))
        .collect();
    stray.sort();
    stray.dedup();
    for sid in stray {
        icacls(dir, &["/remove:g", &format!("*{sid}"), "/C", "/Q"])?;
    }
    let children: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|err| format!("读不了 {}：{err}", dir.display()))?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    for child in &children {
        icacls(child, &["/setowner", "*S-1-5-32-544", "/T", "/C", "/Q"])?;
        icacls(child, &["/reset", "/T", "/C", "/Q"])?;
    }
    for path in std::iter::once(dir).chain(children.iter().map(PathBuf::as_path)) {
        if let Some(why) = weakness(&read(path)?, false) {
            return Err(format!("收紧 {} 的权限之后还是不对：{why}", path.display()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_drive_and_program_files_are_safe_places() {
        let program_files = std::env::var_os("ProgramW6432")
            .or_else(|| std::env::var_os("ProgramFiles"))
            .map(PathBuf::from)
            .unwrap();
        check_target(&program_files.join("Meshora-test-not-created")).unwrap();
        // 这台电脑上别的盘的根目录（有的话）也该能装
        for letter in ['D', 'E'] {
            let root = PathBuf::from(format!("{letter}:\\"));
            if root.is_dir()
                && let Err(err) = check_target(&root.join("Meshora-test-not-created"))
            {
                println!("{letter}: 盘不能装：{err}");
            }
        }
        let windows = system32();
        let root = windows.ancestors().last().unwrap();
        assert_eq!(
            weakness(&read(root).unwrap(), true),
            None,
            "{}",
            root.display()
        );
    }

    #[test]
    fn a_folder_the_user_owns_is_refused() {
        // 测试程序以普通用户跑时，临时目录归自己；以管理员跑（CI）时临时目录归管理员组 —— 那就跳过
        let dir = std::env::temp_dir().join(format!("meshora-acl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let security = read(&dir).unwrap();
        if !crate::location::TRUSTED_SIDS.contains(&security.owner.as_str()) {
            assert!(check_target(&dir.join("Meshora")).is_err());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 要管理员权限（改所有者）。CI 的 Windows 机器上跑：`cargo test -p meshora-setup -- --ignored`
    #[test]
    #[ignore = "要管理员权限"]
    fn lock_down_leaves_only_admins_able_to_change_it() {
        let dir = std::env::temp_dir().join(format!("meshora-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("wintun.dll"), b"x").unwrap();
        lock_down(&dir).unwrap();
        assert_eq!(weakness(&read(&dir).unwrap(), false), None);
        assert_eq!(
            weakness(&read(&dir.join("wintun.dll")).unwrap(), false),
            None
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
