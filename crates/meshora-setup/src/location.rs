//! 装到哪。纯逻辑，能测；读写 Windows 权限的那一半在 `acl.rs`。
//!
//! 客户端以管理员身份运行、从自己旁边加载 wintun.dll：**安装目录必须只有管理员能写**，不然普通程序换掉那个 DLL
//! 就拿到了管理员权限。位置可以随便选，规矩是：
//!
//! - 选的文件夹名字不是 Meshora，就装到它下面的 `Meshora`（免得装进 `D:\` 根目录、和别的东西混在一起）
//! - 选的文件夹要已经存在，而且它和它上面的每一层，普通账户都改不了：不能删、不能改名、不能改权限。
//!   不然普通程序把整个 Meshora 文件夹挪走、放一个假的进去就行了。磁盘根目录和 Program Files 都满足
//! - Meshora 文件夹本身装的时候收紧：只有管理员和 SYSTEM 能写，普通用户只读（`acl::lock_down`）
//! - 已经有东西、又不是 Meshora 装的文件夹，不往里装

use std::path::{Component, Path, PathBuf};

/// 安装目录的名字。
pub const FOLDER: &str = "Meshora";

/// 信得过的账户：管理员组、SYSTEM、TrustedInstaller。
pub const TRUSTED_SIDS: [&str; 3] = [
    "S-1-5-32-544",
    "S-1-5-18",
    "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464",
];

/// "所有者权限"（OWNER RIGHTS）：所有者信得过时，这一条也就信得过。
const OWNER_RIGHTS_SID: &str = "S-1-3-4";

/// DELETE：删掉、改名。
const DELETE: u32 = 0x0001_0000;
/// WRITE_DAC：改权限。
const WRITE_DAC: u32 = 0x0004_0000;
/// WRITE_OWNER：改所有者（改完就能改权限）。
const WRITE_OWNER: u32 = 0x0008_0000;
/// FILE_DELETE_CHILD：删掉、挪走里面的东西（不管那个东西自己的权限）。
const FILE_DELETE_CHILD: u32 = 0x0000_0040;
/// GENERIC_ALL。
const GENERIC_ALL: u32 = 0x1000_0000;
/// 有其中任何一项，就能把这个文件夹（或它里面的）换掉。
const TAMPER: u32 = DELETE | WRITE_DAC | WRITE_OWNER | FILE_DELETE_CHILD | GENERIC_ALL;

/// 权限表里的一条"允许"。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ace {
    /// 给谁（SID 的字符串形式）。
    pub sid: String,
    /// 允许做什么（访问掩码）。
    pub mask: u32,
    /// 只是给里面新建的东西继承用的，对这个文件夹本身不起作用。
    pub inherit_only: bool,
}

/// 一个文件夹的所有者和权限表。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Security {
    /// 所有者的 SID。所有者天生能改权限。
    pub owner: String,
    /// 允许的那些条目；`None` 是没有权限表（谁都能做任何事，FAT 之类的盘也是这样）。
    pub allowed: Option<Vec<Ace>>,
}

/// 普通账户能不能动这个文件夹：能的话说出是谁、能做什么。
///
/// `root` 是磁盘根目录：它删不掉、也改不了名，所以 DELETE 不算（不少盘的根目录给 Authenticated Users 的"修改"
/// 里就有它）；能删里面的东西（FILE_DELETE_CHILD）、能改权限照样算。
pub fn weakness(security: &Security, root: bool) -> Option<String> {
    let tamper = if root { TAMPER & !DELETE } else { TAMPER };
    if !TRUSTED_SIDS.contains(&security.owner.as_str()) {
        return Some(format!("所有者是 {}（不是管理员）", security.owner));
    }
    let Some(aces) = &security.allowed else {
        return Some("没有权限设置，谁都能改（不是 NTFS 的盘？）".into());
    };
    aces.iter()
        .filter(|ace| !ace.inherit_only && ace.mask & tamper != 0)
        .find(|ace| !TRUSTED_SIDS.contains(&ace.sid.as_str()) && ace.sid != OWNER_RIGHTS_SID)
        .map(|ace| format!("{} 能删除、改名或改权限", ace.sid))
}

/// 选的文件夹 → 安装目录。
pub fn target_dir(chosen: &Path) -> Result<PathBuf, String> {
    local_absolute(chosen)?;
    let named = chosen
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(FOLDER));
    Ok(if named {
        chosen.to_path_buf()
    } else {
        chosen.join(FOLDER)
    })
}

/// 本机磁盘上的完整路径（`D:\…`），不是网络位置，没有 `..`
fn local_absolute(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::path::Prefix;
        match path.components().next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(_) | Prefix::VerbatimDisk(_) => {}
                _ => return Err("只能装在这台电脑的磁盘上，不能是网络位置".into()),
            },
            _ => return Err("要一个完整的路径，比如 D:\\Meshora".into()),
        }
    }
    if !path.is_absolute() {
        return Err("要一个完整的路径，比如 D:\\Meshora".into());
    }
    if path
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err("路径里不能有 . 或 ..".into());
    }
    Ok(())
}

/// 这个文件夹里已经有的东西能不能留着往里装：空的，或者本来就是 Meshora 装的（有 meshora.exe 或 uninstall.exe）。
/// 不能的话说出一个不认识的文件名。
pub fn foreign_file(names: &[String]) -> Option<String> {
    let ours = names.iter().any(|name| {
        name.eq_ignore_ascii_case("meshora.exe") || name.eq_ignore_ascii_case("uninstall.exe")
    });
    if ours { None } else { names.first().cloned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ace(sid: &str, mask: u32, inherit_only: bool) -> Ace {
        Ace {
            sid: sid.into(),
            mask,
            inherit_only,
        }
    }

    /// Windows 装好时 C:\ 根目录的样子（简化）：普通用户能在下面建文件夹，删、改别人的不行
    fn drive_root() -> Security {
        Security {
            owner: "S-1-5-18".into(),
            allowed: Some(vec![
                ace("S-1-5-32-544", 0x001F_01FF, false),
                ace("S-1-5-18", 0x001F_01FF, false),
                ace("S-1-5-32-545", 0x0012_00A9, false),
                // Authenticated Users：这一层只能建文件夹；"修改"只给下面继承
                ace("S-1-5-11", 0x0000_0004, false),
                ace("S-1-5-11", 0x0013_01BF, true),
            ]),
        }
    }

    #[test]
    fn a_drive_root_and_program_files_are_safe_parents() {
        assert_eq!(weakness(&drive_root(), true), None);
        let program_files = Security {
            owner: TRUSTED_SIDS[2].into(),
            allowed: Some(vec![
                ace(TRUSTED_SIDS[2], 0x001F_01FF, false),
                ace("S-1-5-32-545", 0x0012_00A9, false),
                ace("S-1-3-0", GENERIC_ALL, true),
            ]),
        };
        assert_eq!(weakness(&program_files, false), None);
    }

    #[test]
    fn a_folder_a_user_made_or_can_change_is_not() {
        // 自己建的文件夹：所有者是自己
        let mine = Security {
            owner: "S-1-5-21-1-2-3-1001".into(),
            ..drive_root()
        };
        assert!(weakness(&mine, false).is_some());
        // 管理员建的，但给了普通用户"修改"（含删除）
        let mut shared = drive_root();
        shared
            .allowed
            .as_mut()
            .unwrap()
            .push(ace("S-1-5-11", 0x0013_01BF, false));
        assert!(weakness(&shared, false).is_some());
        // 普通用户能删里面的东西
        let mut deletes_children = drive_root();
        deletes_children
            .allowed
            .as_mut()
            .unwrap()
            .push(ace("S-1-1-0", FILE_DELETE_CHILD, false));
        assert!(weakness(&deletes_children, false).is_some());
        // 没有权限表（FAT 盘）
        let fat = Security {
            owner: "S-1-5-32-544".into(),
            allowed: None,
        };
        assert!(weakness(&fat, false).is_some());
    }

    #[test]
    fn delete_on_a_drive_root_does_not_count_but_delete_child_does() {
        // 有的盘根目录给 Authenticated Users "修改"（含 DELETE）：根目录删不掉，没关系
        let mut root = drive_root();
        root.allowed
            .as_mut()
            .unwrap()
            .push(ace("S-1-5-11", 0x0013_01BF, false));
        assert_eq!(weakness(&root, true), None);
        assert!(weakness(&root, false).is_some());
        root.allowed
            .as_mut()
            .unwrap()
            .push(ace("S-1-5-11", FILE_DELETE_CHILD, false));
        assert!(weakness(&root, true).is_some());
    }

    #[test]
    fn existing_folders_must_be_empty_or_ours() {
        assert_eq!(foreign_file(&[]), None);
        assert_eq!(
            foreign_file(&["meshora.exe".into(), "wintun.dll".into()]),
            None
        );
        assert_eq!(foreign_file(&["Uninstall.exe".into()]), None);
        assert_eq!(
            foreign_file(&["game.exe".into(), "save.dat".into()]),
            Some("game.exe".into())
        );
    }

    #[cfg(windows)]
    #[test]
    fn the_chosen_folder_gets_a_meshora_subfolder() {
        assert_eq!(
            target_dir(Path::new(r"D:\")).unwrap(),
            PathBuf::from(r"D:\Meshora")
        );
        assert_eq!(
            target_dir(Path::new(r"D:\Apps")).unwrap(),
            PathBuf::from(r"D:\Apps\Meshora")
        );
        assert_eq!(
            target_dir(Path::new(r"D:\Apps\meshora")).unwrap(),
            PathBuf::from(r"D:\Apps\meshora")
        );
        assert!(target_dir(Path::new(r"\\server\share")).is_err());
        assert!(target_dir(Path::new(r"Apps")).is_err());
        assert!(target_dir(Path::new(r"D:\Apps\..\Windows")).is_err());
    }
}
