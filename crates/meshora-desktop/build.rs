//! Windows 上：把界面文件、图标和应用清单编进可执行文件（tauri-build 做）。

fn main() {
    #[cfg(windows)]
    {
        // 清单要求管理员权限：建虚拟网卡需要。自己写清单就得自己带上 Common Controls v6，
        // Tauri 的原生对话框依赖它。
        //
        // 开发时设 MESHORA_DESKTOP_AS_INVOKER=1 编译，就不要求管理员：界面照常能开、能点，
        // 只是连网络会停在建虚拟网卡那一步。发出去的版本不要这样编
        println!("cargo:rerun-if-env-changed=MESHORA_DESKTOP_AS_INVOKER");
        let manifest = include_str!("meshora.manifest");
        let manifest = if std::env::var_os("MESHORA_DESKTOP_AS_INVOKER").is_some() {
            manifest.replace("requireAdministrator", "asInvoker")
        } else {
            manifest.to_owned()
        };
        let windows = tauri_build::WindowsAttributes::new().app_manifest(manifest);
        tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
            .expect("tauri-build 失败");
    }
}
