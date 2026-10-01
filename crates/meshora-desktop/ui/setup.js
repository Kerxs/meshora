// 安装程序的界面（crates/meshora-setup）。一个窗口，几页：装 → 装着 → 装好了；带 --uninstall 运行时是卸载。
//
// 和客户端同样的规矩：内容一律 textContent，不写内联样式（进度条的宽度走 CSSOM）。
import glassium from "./vendor/glassium/index.js";

glassium.configure({ absorbForComponents: true });

const tauri = () => window.__TAURI__;
const invoke = (command, args) => tauri().core.invoke(command, args);
const SVG_NS = "http://www.w3.org/2000/svg";

function h(tag, props, ...children) {
  const el = document.createElement(tag);
  setProps(el, props);
  append(el, children);
  return el;
}

function s(tag, props, ...children) {
  const el = document.createElementNS(SVG_NS, tag);
  setProps(el, props);
  append(el, children);
  return el;
}

function setProps(el, props) {
  for (const [key, value] of Object.entries(props || {})) {
    if (value === undefined || value === null || value === false) continue;
    if (key === "class") el.setAttribute("class", value);
    else if (key.startsWith("on")) el.addEventListener(key.slice(2), value);
    else el.setAttribute(key, value === true ? "" : value);
  }
}

function append(el, children) {
  for (const child of children.flat()) {
    if (child === null || child === undefined || child === false) continue;
    el.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
}

const LOGO = () =>
  s(
    "svg",
    { viewBox: "0 0 64 64", "aria-hidden": "true" },
    s(
      "g",
      { stroke: "rgba(255,255,255,.6)", "stroke-width": 3.2, "stroke-linecap": "round" },
      [
        [18, 20, 46, 17],
        [18, 20, 20, 46],
        [18, 20, 45, 44],
        [46, 17, 20, 46],
        [46, 17, 45, 44],
        [20, 46, 45, 44],
      ].map(([x1, y1, x2, y2]) => s("line", { x1, y1, x2, y2 })),
    ),
    [
      [18, 20, "#fff"],
      [46, 17, "#fff"],
      [20, 46, "#fff"],
      [45, 44, "#5ff0c0"],
    ].map(([cx, cy, fill]) => s("circle", { cx, cy, r: 6.5, fill })),
  );

function glassSwitch(checked) {
  const el = document.createElement("glass-switch");
  el.checked = checked;
  return el;
}

function option(title, desc, control) {
  return h("label", { class: "opt" }, h("span", { class: "t" }, h("b", {}, title), desc ? h("span", {}, desc) : null), control);
}

const primary = (text, onclick) => h("button", { class: "btn wide", glass: "tinted", "glass-tint": "#3d6bff", type: "button", onclick }, text);
const secondary = (text, onclick) => h("button", { class: "btn wide", glass: "clear", type: "button", onclick }, text);

// ---------- 框架 ----------

const win = () => tauri()?.window?.getCurrentWindow?.();
let busy = false;

function titlebar() {
  const icon = (d) => s("svg", { viewBox: "0 0 12 12", "aria-hidden": "true" }, s("path", { d, stroke: "currentColor", "stroke-width": 1.4, "stroke-linecap": "round", fill: "none" }));
  return h(
    "header",
    { class: "titlebar", "data-tauri-drag-region": "" },
    h("span", { class: "grow", "data-tauri-drag-region": "" }),
    h("button", { class: "tb-btn", type: "button", title: "最小化", "aria-label": "最小化", onclick: () => win()?.minimize() }, icon("M2.5 6h7")),
    (closeBtn = h("button", { class: "tb-btn close", type: "button", title: "关闭", "aria-label": "关闭", onclick: () => !busy && invoke("quit") }, icon("M3 3l6 6M9 3l-6 6"))),
  );
}

let closeBtn;
let panel;

function show(...children) {
  panel.replaceChildren(...children.filter(Boolean));
  // 新的一页里第一个按钮拿焦点：回车就能接着走
  panel.querySelector("button.btn[glass='tinted']")?.focus();
}

function head(title, sub) {
  return [h("div", { class: "setup-logo" }, LOGO()), h("h1", {}, title), sub ? h("p", { class: "sub" }, sub) : null];
}

// ---------- 进度 ----------

const progress = {
  build(title) {
    this.step = h("p", { class: "sub" }, "准备中");
    this.fill = h("i");
    this.pct = h("span", { class: "pct" }, "0%");
    return [...head(title), this.step, h("div", { class: "meter" }, this.fill), this.pct];
  },
  set({ step, percent }) {
    if (!this.fill) return;
    this.step.textContent = step;
    this.fill.style.width = `${percent}%`;
    this.pct.textContent = `${percent}%`;
  },
};

async function work(title, command, args, done) {
  busy = true;
  closeBtn.disabled = true;
  show(...progress.build(title));
  try {
    await invoke(command, args);
    progress.set({ step: "完成", percent: 100 });
    done();
  } catch (err) {
    failed(String(err), () => work(title, command, args, done));
  } finally {
    busy = false;
    closeBtn.disabled = false;
  }
}

function failed(message, retry) {
  show(
    ...head("没能做完", null),
    h("p", { class: "sub err" }, message),
    h("div", { class: "actions" }, secondary("关闭", () => invoke("quit")), primary("再试一次", retry)),
  );
}

// ---------- 装 ----------

function installPage(info) {
  const desktop = glassSwitch(true);
  const open = glassSwitch(true);
  const upgrade = info.installed
    ? info.installed === info.version
      ? `已经装着 ${info.version}：重新装一遍，设置和私钥都留着。`
      : `已经装着 ${info.installed}：就地升级到 ${info.version}，设置和私钥都留着。`
    : "和朋友组成虚拟局域网：不管在哪，联机都像坐在一起。";
  show(
    ...head(`安装 Meshora ${info.version}`, upgrade),
    h(
      "div",
      { class: "opts" },
      h("div", { class: "opt" }, h("span", { class: "t" }, h("b", {}, "装到"), h("span", { class: "mono dir" }, info.dir))),
      option("桌面快捷方式", "开始菜单里总会有一个", desktop),
      option("装完打开", null, open),
    ),
    h(
      "div",
      { class: "actions" },
      secondary("取消", () => invoke("quit")),
      primary("安装", () =>
        work("正在安装", "install", { desktop: desktop.checked }, () => installed(open.checked)),
      ),
    ),
  );
}

async function installed(launch) {
  if (launch) {
    try {
      await invoke("launch");
      invoke("quit");
      return;
    } catch (err) {
      failed(String(err), () => installed(true));
      return;
    }
  }
  show(
    ...head("装好了", "从开始菜单打开 Meshora。它要管理员权限来建虚拟网卡，打开时 Windows 会问一次。"),
    h("div", { class: "actions" }, secondary("完成", () => invoke("quit")), primary("打开 Meshora", () => installed(true))),
  );
}

// ---------- 更新（客户端发起，带 --update）----------

/** 不用点：直接装，桌面快捷方式照旧，装完把 Meshora 打开 */
function updatePage(info) {
  work(`正在更新到 ${info.version}`, "install", { desktop: null }, () => installed(true));
}

// ---------- 卸 ----------

function uninstallPage(info) {
  const purge = glassSwitch(false);
  const warn = h("p", { class: "warn", hidden: true }, "私钥删了就是换了一个身份：再装回来，朋友那边看到的是一台新电脑，网主要重新放你进网络。");
  purge.addEventListener("change", () => {
    warn.hidden = !purge.checked;
  });
  show(
    ...head("卸载 Meshora", `会关掉正在运行的 Meshora，删掉 ${info.dir} 和快捷方式。`),
    h("div", { class: "opts" }, option("同时删除我的私钥和设置", "默认留着：以后装回来还是同一台电脑", purge)),
    warn,
    h(
      "div",
      { class: "actions" },
      secondary("取消", () => invoke("quit")),
      h("button", { class: "btn wide danger", glass: "clear", type: "button", onclick: () => work("正在卸载", "uninstall", { purge: purge.checked }, uninstalled) }, "卸载"),
    ),
  );
}

function uninstalled() {
  show(...head("卸载好了", "Meshora 已经从这台电脑上拿掉了。"), h("div", { class: "actions" }, primary("关闭", () => invoke("quit"))));
}

// ---------- 起 ----------

async function start() {
  panel = h("section", { class: "setup-panel", glass: "", "glass-corner-radius": "26" });
  document.getElementById("setup").replaceChildren(titlebar(), h("main", {}, panel));
  await tauri().event.listen("progress", (event) => progress.set(event.payload));
  const info = await invoke("info");
  if (info.mode === "uninstall") uninstallPage(info);
  else if (info.mode === "update") updatePage(info);
  else installPage(info);
}

start().catch((err) => {
  console.error(err);
  panel?.replaceChildren(h("p", { class: "sub err" }, String(err)));
});
