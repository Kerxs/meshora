// 在普通浏览器里看安装程序界面用的假后端。打开 dev/setup.html?s=<场景>
// 场景：fresh（新装）、upgrade（升级）、fail（装到一半出错）、uninstall
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
      mode: scenario === "uninstall" ? "uninstall" : "install",
      version: "1.1.0",
      dir: "C:\\Program Files\\Meshora",
      installed: scenario === "upgrade" || scenario === "uninstall" ? "1.0.0" : null,
    }),
    install: async () => {
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
