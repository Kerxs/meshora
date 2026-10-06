// 在普通浏览器里看安装程序界面用的假后端。打开 dev/setup.html?s=<场景>
// 场景：fresh（新装）、upgrade（升级）、update（客户端发起的更新）、fail（装到一半出错）、uninstall、badpick（选了不能装的位置）、
// fast（和真的安装一样：好几步在同一毫秒里报上来，几十毫秒就装完）
"use strict";

(() => {
  const scenario = new URLSearchParams(location.search).get("s") || "fresh";
  const listeners = [];
  const emit = (payload) => listeners.forEach((cb) => cb({ payload }));
  const steps = async (list) => {
    for (const [step, percent] of list) {
      emit({ step, percent });
      await new Promise((resolve) => setTimeout(resolve, 350));
    }
  };
  const handlers = {
    info: () => ({
      mode: scenario === "uninstall" ? "uninstall" : scenario === "update" ? "update" : "install",
      version: "1.1.0",
      dir: "C:\\Program Files\\Meshora",
      installed: ["upgrade", "update", "uninstall"].includes(scenario) ? "1.0.0" : null,
      movable: !["upgrade", "update", "uninstall"].includes(scenario),
    }),
    pick_dir: () => {
      if (scenario === "badpick") throw String.raw`D:\Games 普通账户也能改（所有者是 S-1-5-21-1-2-3-1001（不是管理员）），装在这下面不安全：别的程序能把 Meshora 整个换掉。选磁盘根目录（比如 D:\）或者 Program Files 下面`;
      return String.raw`D:\Meshora`;
    },
    install: async () => {
      if (scenario === "fast") {
        // 真的安装：关进程、卸旧版本、复制文件几乎同时报上来
        for (const [step, percent] of [["关掉正在运行的 Meshora", 5], ["卸掉旧版本", 12], ["复制文件", 20], ["复制文件", 50], ["复制文件", 80], ["建快捷方式", 85], ["放行防火墙", 89]]) emit({ step, percent });
        await new Promise((resolve) => setTimeout(resolve, 30));
        emit({ step: "登记到“应用和功能”", percent: 92 });
        emit({ step: "装好了", percent: 100 });
        return;
      }
      await steps([["关掉正在运行的 Meshora", 5], ["卸掉旧版本", 12], ["复制文件", 20], ["复制文件", 55], ["复制文件", 80]]);
      if (scenario === "fail") throw "写不了 C:\\Program Files\\Meshora\\wintun.dll：拒绝访问。 (os error 5)";
      await steps([["建快捷方式", 85], ["登记到“应用和功能”", 92], ["装好了", 100]]);
    },
    uninstall: () => steps([["关掉正在运行的 Meshora", 10], ["删快捷方式", 25], ["删文件", 45], ["从“应用和功能”里拿掉", 75], ["卸载好了", 100]]),
    launch: () => {
      throw "（预览里不打开）";
    },
    quit: () => console.log("quit"),
  };
  window.__TAURI__ = {
    core: {
      invoke: async (command, args) => {
        console.log("invoke", command, args);
        return handlers[command](args);
      },
    },
    event: {
      listen: async (_name, cb) => listeners.push(cb),
    },
    window: { getCurrentWindow: () => ({ minimize() {} }) },
  };
})();
