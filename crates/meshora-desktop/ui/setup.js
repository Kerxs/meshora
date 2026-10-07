// 安装程序的界面（crates/meshora-setup）。分步：欢迎 → 选项 → 安装中 → 完成；卸载是 确认 → 卸载中 → 卸载好了；
// 客户端发起的更新（--update）直接进"安装中"，装完自己打开。步与步之间左右滑，Logo、进度环、对勾都有动画。
// 动画一个接一个：旧页退完新页才进来；进度一步一步走，每步至少停一会儿；进度环走到头才出结果页。
//
// 和客户端同样的规矩：内容一律 textContent，不写内联样式（位置、进度走 CSSOM）。
// 背景的点阵要在 Glassium 之前建好：它开场就把玻璃后面的背景收进场景
import "./sky.js";
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

/** 依次出现的动画：第几个（CSS 里乘上间隔） */
const nth = (el, i) => {
  el.style.setProperty("--i", String(i));
  return el;
};

/** Logo：连线一条条画出来，四个节点依次弹出来 */
function logo() {
  const lines = [
    [18, 20, 46, 17],
    [18, 20, 20, 46],
    [18, 20, 45, 44],
    [46, 17, 20, 46],
    [46, 17, 45, 44],
    [20, 46, 45, 44],
  ];
  const nodes = [
    [18, 20, "#fff"],
    [46, 17, "#fff"],
    [20, 46, "#fff"],
    [45, 44, "#5ff0c0"],
  ];
  return s(
    "svg",
    { class: "logo logo-draw", viewBox: "0 0 64 64", "aria-hidden": "true" },
    s("g", { stroke: "rgba(255,255,255,.6)", "stroke-width": 3.2, "stroke-linecap": "round" }, lines.map(([x1, y1, x2, y2], i) => nth(s("line", { class: "l", x1, y1, x2, y2, pathLength: 1 }), i))),
    nodes.map(([cx, cy, fill], i) => nth(s("circle", { class: "n", cx, cy, r: 6.5, fill }), i)),
  );
}

/** 关窗口：整页轻轻缩小、淡出，播完再关（要求减少动画时直接关） */
function bye() {
  if (matchMedia("(prefers-reduced-motion: reduce)").matches) return invoke("quit");
  document.getElementById("setup").classList.add("bye");
  setTimeout(() => invoke("quit"), 300);
}

function glassSwitch(checked) {
  const el = document.createElement("glass-switch");
  el.checked = checked;
  return el;
}

function option(title, desc, control) {
  return h("div", { class: "opt" }, h("span", { class: "t" }, h("b", {}, title), desc ? h("span", {}, desc) : null), control);
}

const primary = (text, onclick) => h("button", { class: "btn wide", glass: "tinted", "glass-tint": "#3d6bff", type: "button", onclick }, text);
const secondary = (text, onclick) => h("button", { class: "btn wide", glass: "clear", type: "button", onclick }, text);
const danger = (text, onclick) => h("button", { class: "btn wide danger", glass: "clear", type: "button", onclick }, text);

// ---------- 框架：标题栏、步骤点、会滑动的舞台 ----------

const win = () => tauri()?.window?.getCurrentWindow?.();
let busy = false;
let closeBtn;
let stage;
let dots;
let current = null;

function titlebar() {
  const icon = (d) => s("svg", { viewBox: "0 0 12 12", "aria-hidden": "true" }, s("path", { d, stroke: "currentColor", "stroke-width": 1.4, "stroke-linecap": "round", fill: "none" }));
  dots = h("div", { class: "steps", "aria-hidden": "true" });
  closeBtn = h("button", { class: "tb-btn close", type: "button", title: "关闭", "aria-label": "关闭", onclick: () => !busy && bye() }, icon("M3 3l6 6M9 3l-6 6"));
  return h(
    "header",
    { class: "titlebar", "data-tauri-drag-region": "" },
    h("span", { class: "tb-name", "data-tauri-drag-region": "" }, "Meshora"),
    h("span", { class: "grow", "data-tauri-drag-region": "" }),
    dots,
    h("span", { class: "grow", "data-tauri-drag-region": "" }),
    h("button", { class: "tb-btn", type: "button", title: "最小化", "aria-label": "最小化", onclick: () => win()?.minimize() }, icon("M2.5 6h7")),
    closeBtn,
  );
}

/** 步骤点：一共几步、现在第几步（从 0 数） */
function step(total, at) {
  if (dots.children.length !== total) dots.replaceChildren(...Array.from({ length: total }, () => h("i")));
  [...dots.children].forEach((dot, i) => dot.classList.toggle("on", i === at));
}

const still = () => matchMedia("(prefers-reduced-motion: reduce)").matches;
/** 等一会儿（用定时器：窗口在后台时 rAF 不来）。要求减少动画时不等 */
const wait = (ms) => new Promise((resolve) => setTimeout(resolve, still() ? 0 : ms));

/** 旧页退场多久（setup.css 的 .page.to-left / .to-right） */
const LEAVE_MS = 260;

/** 换页排着队：上一次换完才换下一次 */
let showing = Promise.resolve();

/**
 * 换一页。forward：新页从右边滑进来；后退反过来。
 * 旧页先退场、退完新页才进来，不叠在一起；旧页里还在播的进场动画（点得快的时候）直接跳到结尾再退场
 */
function show(page, forward = true) {
  const swap = async () => {
    const old = current;
    current = page;
    if (old) {
      for (const animation of old.getAnimations({ subtree: true })) {
        try {
          animation.finish();
        } catch {
          // 无限循环的动画跳不到结尾：随它，反正马上要摘掉
        }
      }
      old.classList.add(forward ? "to-left" : "to-right");
      old.setAttribute("aria-hidden", "true");
      old.inert = true;
      await wait(LEAVE_MS);
      old.remove();
    }
    page.classList.add("page", forward ? "from-right" : "from-left");
    stage.append(page);
    // 主按钮拿焦点（回车就能接着走），但不画焦点框：不是用户按 Tab 过来的
    setTimeout(() => page.querySelector("button.btn[glass='tinted']")?.focus({ focusVisible: false }), 60);
  };
  showing = showing.then(swap, swap);
  return showing;
}

// ---------- 进度 ----------

/** 每一步至少显示这么久：真的安装时好几步在同一毫秒里报上来，字来不及换就被下一步打断 */
const STEP_MIN_MS = 400;
/** 进度环走一段要多久（setup.css 的 .ring .arc，--t-slow） */
const RING_MS = 640;

/**
 * 环形进度：描边画到当前百分比，中间的数字滚上去，下面一行是当前在做什么。
 * 报上来的进度排队一步步显示（set），settled() 等到都显示完、环走到头
 */
const progress = {
  build(title) {
    const R = 52;
    this.length = 2 * Math.PI * R;
    this.arc = s("circle", { class: "arc", cx: 60, cy: 60, r: R, transform: "rotate(-90 60 60)" });
    this.arc.style.strokeDasharray = String(this.length);
    this.arc.style.strokeDashoffset = String(this.length);
    this.number = h("b", {}, "0");
    this.step = h("p", { class: "sub step" }, "准备中");
    this.shown = 0;
    this.queue = [];
    this.pumping = null;
    this.appliedAt = 0;
    this.percent = 0;
    this.visible = Promise.resolve();
    return h(
      "section",
      { class: "progress" },
      h("div", { class: "ring" }, s("svg", { viewBox: "0 0 120 120", "aria-hidden": "true" }, s("circle", { class: "track", cx: 60, cy: 60, r: R }), this.arc), h("div", { class: "pct" }, this.number, h("span", {}, "%"))),
      h("h1", {}, title),
      this.step,
    );
  },
  set(report) {
    if (!this.arc) return;
    this.queue.push(report);
    if (!this.pumping) this.pumping = this.pump();
  },
  /** 一步一步显示排着的进度。同一步接连报上来的只看最后一个（复制文件 20% → 55% → 80%） */
  async pump() {
    // 进度页换上来了才开始走（换页排着队，旧页还在退场时报上来的进度先存着）。
    // 也顺带让出一下：this.pumping 赋上值之后才往下走，走完时清掉的才是它
    await this.visible;
    while (this.queue.length) {
      // 上一步还没停够就等一等（不管下一步是不是早就报上来了）
      const left = this.appliedAt ? STEP_MIN_MS - (performance.now() - this.appliedAt) : 0;
      if (left > 0) await wait(left);
      let next = this.queue.shift();
      while (this.queue[0]?.step === next.step) next = this.queue.shift();
      this.apply(next);
    }
    this.pumping = null;
  },
  /** 排着的都显示完了，最后一步也停够了、环也走到头了 */
  async settled() {
    while (this.pumping) await this.pumping;
    const since = performance.now() - this.appliedAt;
    await wait(Math.max(STEP_MIN_MS, RING_MS) - since);
  },
  apply({ step, percent }) {
    this.appliedAt = performance.now();
    this.percent = percent;
    this.arc.style.strokeDashoffset = String(this.length * (1 - percent / 100));
    if (this.step.textContent !== step) {
      // 换步骤时那行字淡出再淡入
      this.step.classList.remove("swap");
      void this.step.offsetWidth;
      this.step.classList.add("swap");
      this.step.textContent = step;
    }
    this.count(percent);
  },
  /** 数字滚到 target：跟着帧走；窗口在后台不出帧时，到点直接写上 */
  count(target) {
    const from = this.shown;
    const start = performance.now();
    const token = (this.token = {});
    const tick = (now) => {
      if (this.token !== token) return;
      const t = Math.min(1, (now - start) / 360);
      this.shown = Math.round(from + (target - from) * (1 - (1 - t) ** 3));
      this.number.textContent = String(this.shown);
      if (t < 1) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
    setTimeout(() => {
      if (this.token !== token) return;
      this.shown = target;
      this.number.textContent = String(target);
    }, 420);
  },
};

/** 做一件要等的事（装、卸），期间不能关窗口；成功走 done，失败给出错页（能重试） */
async function work(title, command, args, total, at, done) {
  busy = true;
  closeBtn.disabled = true;
  step(total, at);
  progress.visible = show(progress.build(title));
  try {
    await invoke(command, args);
    // 后端最后会报一个 100%（"装好了"、"卸载好了"）；没报到的话补一个
    const last = progress.queue.at(-1)?.percent ?? progress.percent;
    if (last < 100) progress.set({ step: "完成", percent: 100 });
    // 进度环走满、"完成"停一下，再出结果页
    await progress.settled();
    await wait(200);
    done();
  } catch (err) {
    // 出错前报上来的几步也照样走完，再换成出错页
    await progress.settled();
    failed(String(err), () => work(title, command, args, total, at, done));
  } finally {
    busy = false;
    closeBtn.disabled = false;
  }
}

// ---------- 结果页 ----------

/** 画出来的对勾 */
const tick = () => s("svg", { class: "mark ok", viewBox: "0 0 64 64", "aria-hidden": "true" }, s("circle", { cx: 32, cy: 32, r: 28, pathLength: 1 }), s("path", { d: "M20 33l8 8 16-17", pathLength: 1 }));
/** 抖一下的叉 */
const cross = () => s("svg", { class: "mark bad", viewBox: "0 0 64 64", "aria-hidden": "true" }, s("circle", { cx: 32, cy: 32, r: 28, pathLength: 1 }), s("path", { d: "M23 23l18 18M41 23L23 41", pathLength: 1 }));

function result(mark, title, sub, ...actions) {
  return h("section", { class: "result" }, mark, h("h1", {}, title), sub ? h("p", { class: "sub" }, sub) : null, h("div", { class: "actions" }, ...actions));
}

function failed(message, retry) {
  show(result(cross(), "没能做完", message, secondary("关闭", bye), primary("再试一次", retry)));
}

// ---------- 装 ----------

function welcome(info, forward = true) {
  step(4, 0);
  const upgrade = info.installed
    ? info.installed === info.version
      ? `已经装着 ${info.version}：重新装一遍，网络、名字、私钥都留着。`
      : `已经装着 ${info.installed}：升级到 ${info.version}，网络、名字、私钥都留着。`
    : "和朋友组成虚拟局域网：不管在哪，联机都像坐在一起。";
  show(
    h(
      "section",
      { class: "welcome" },
      logo(),
      h("h1", {}, info.installed ? "升级 Meshora" : "安装 Meshora"),
      h("p", { class: "ver" }, info.installed && info.installed !== info.version ? `${info.installed} → ${info.version}` : info.version),
      h("p", { class: "sub" }, upgrade),
      h("div", { class: "actions" }, primary(info.installed ? "开始升级" : "开始安装", () => options(info))),
    ),
    forward,
  );
}

function options(info) {
  step(4, 1);
  const desktop = glassSwitch(true);
  const open = glassSwitch(true);
  const safe = "只有管理员能改这个文件夹，别的程序换不掉里面的文件";
  const dir = h("span", { class: "mono dir" }, info.dir);
  const note = h("span", { class: "note" }, info.movable ? safe : "已经装在这里，升级装回原处");
  const error = h("p", { class: "where-err", hidden: true });
  const change = info.movable
    ? h(
        "button",
        {
          class: "btn sm",
          glass: "clear",
          type: "button",
          onclick: async () => {
            try {
              const picked = await invoke("pick_dir");
              if (!picked) return;
              info.dir = picked;
              dir.textContent = picked;
              error.hidden = true;
            } catch (err) {
              // 原来选的位置不变，只说新选的为什么不行
              error.textContent = String(err);
              error.hidden = false;
            }
          },
        },
        "更改",
      )
    : null;
  show(
    h(
      "section",
      { class: "options" },
      h("h1", {}, "安装选项"),
      h(
        "div",
        { class: "opts", glass: "tinted", "glass-tint": "rgba(0, 0, 0, 0.22)" },
        h("div", { class: "opt where" }, h("span", { class: "t" }, h("b", {}, "装到"), dir, note), change, error),
        option("桌面快捷方式", "开始菜单里总会有一个", desktop),
        option("装完打开", null, open),
      ),
      h(
        "div",
        { class: "actions" },
        secondary("上一步", () => welcome(info, false)),
        primary(info.installed ? "升级" : "安装", () =>
          work(info.installed ? `正在升级到 ${info.version}` : "正在安装", "install", { desktop: desktop.checked, dir: info.dir }, 4, 2, () => installed(open.checked, 4)),
        ),
      ),
    ),
  );
}

/** 装好了。`launch`：顺手打开 Meshora。`total`：一共几步（装是 4 步，客户端发起的更新是 2 步），这是最后一步 */
async function installed(launch, total) {
  step(total, total - 1);
  const later = (note) =>
    show(result(tick(), "装好了", note, secondary("完成", bye), primary("打开 Meshora", () => installed(true, total))));
  if (!launch) {
    later("从开始菜单打开 Meshora。它要管理员权限来建虚拟网卡，打开时 Windows 会问一次。");
    return;
  }
  try {
    await invoke("launch");
  } catch (err) {
    // 装是装好了，只是没打开：别给"没能做完、再试一次"（那会重装一遍）
    later(`没能自动打开 Meshora（${err}）。可以从开始菜单打开。`);
    return;
  }
  show(result(tick(), "装好了", "Meshora 正在打开。", primary("完成", bye)));
  setTimeout(bye, 1600);
}

// ---------- 更新（客户端发起，带 --update）----------

/** 不用点：直接装，桌面快捷方式照旧，装完把 Meshora 打开 */
function updatePage(info) {
  work(`正在更新到 ${info.version}`, "install", { desktop: null, dir: null }, 2, 0, () => installed(true, 2));
}

// ---------- 卸 ----------

function uninstallPage(info) {
  step(3, 0);
  const purge = glassSwitch(false);
  const warn = h("p", { class: "warn" }, "私钥删了就是换了一个身份：再装回来，朋友那边看到的是一台新电脑，网主要重新放你进网络。");
  purge.addEventListener("change", () => warn.classList.toggle("on", purge.checked));
  show(
    h(
      "section",
      { class: "options" },
      h("h1", {}, "卸载 Meshora"),
      h("p", { class: "sub" }, "会关掉正在运行的 Meshora，删掉快捷方式和安装目录：", h("span", { class: "mono dir" }, info.dir)),
      h("div", { class: "opts", glass: "tinted", "glass-tint": "rgba(0, 0, 0, 0.22)" }, option("同时删除我的私钥和设置", "默认留着：以后装回来还是同一台电脑", purge)),
      warn,
      h(
        "div",
        { class: "actions" },
        secondary("取消", bye),
        danger("卸载", () => work("正在卸载", "uninstall", { purge: purge.checked }, 3, 1, uninstalled)),
      ),
    ),
  );
}

function uninstalled() {
  step(3, 2);
  show(result(tick(), "卸载好了", "Meshora 已经从这台电脑上拿掉了。", primary("关闭", bye)));
}

// ---------- 起 ----------

async function start() {
  stage = h("div", { class: "stage" });
  document.getElementById("setup").replaceChildren(titlebar(), h("main", {}, h("section", { class: "setup-panel", glass: "frosted", "glass-corner-radius": "26" }, stage)));
  await tauri().event.listen("progress", (event) => progress.set(event.payload));
  const info = await invoke("info");
  if (info.mode === "uninstall") uninstallPage(info);
  else if (info.mode === "update") updatePage(info);
  else welcome(info);
}

start().catch((err) => {
  console.error(err);
  stage?.replaceChildren(h("p", { class: "sub err" }, String(err)));
});
