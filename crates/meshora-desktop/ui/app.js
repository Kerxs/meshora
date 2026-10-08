// Meshora 桌面客户端的界面。所有状态来自 Rust 那一侧的 overview 命令，每秒拉一次；按钮只是把意图转过去。
//
// 两条规矩：
// - 数据一律用 textContent 放进页面（朋友的名字是别人随便填的），不拼 HTML
// - CSP 不许内联样式：不写 style= 属性，要动的位置走 CSSOM（el.style.xxx）
// 背景的点阵要在 Glassium 之前建好：它开场就把玻璃后面的背景收进场景
import { LIQUID_EDGE, LIQUID_GLASS, WELL_GLASS, liquidBubble, liquidPill, useSkyAsGlassScene } from "./sky.js";
import glassium from "./vendor/glassium/index.js";
import { closeLayer, flashCopied, flip, intro, swapView, tweenText } from "./motion.js";

// 安卓客户端（crates/meshora-android）用的也是这份界面：手机上没有标题栏、本机当主机、Windows 的网络设置，
// 导航挪到屏幕底部。一打开就要知道（标题栏在拿到第一份状态之前就画了），所以看 User-Agent
const PHONE = /Android/i.test(navigator.userAgent);
document.documentElement.classList.toggle("phone", PHONE);

// 开关是 Glassium 的组件：它们后面的背景也要收进场景，玻璃才折射得到。
// 手机上玻璃全用 CSS 画（backend: 'css'，不建 GPU 画布）：手机的滚动由合成线程直接做，画在页面底下的 GPU 玻璃
// 会慢一两帧、落在文字后面；CSS 画的和滚动一起走。材质还是 Glassium 的，模糊、着色、亮边、投影照搬，只是没有折射。
// 要在 runtime 启动之前设（import 之后的同一个任务里）
glassium.configure(PHONE ? { backend: "css" } : { absorbForComponents: true });
// 电脑上背景画布交给 Glassium 当场景：玻璃后面真的是那片光和点（见 sky.js）
if (!PHONE) useSkyAsGlassScene(glassium);

const invoke = (command, args) => window.__TAURI__.core.invoke(command, args);
const DEVICE = PHONE ? "这台手机" : "这台电脑";

const POLL_MS = 1000;
/** 延迟历史记多少个点：每秒一个，五分钟 */
const HISTORY = 300;
const SVG_NS = "http://www.w3.org/2000/svg";

// ---------- 小工具 ----------

function h(tag, props, ...children) {
  const el = document.createElement(tag);
  setProps(el, props);
  append(el, children);
  return el;
}

/**
 * 只在变了时写字、换 class。界面每秒按新状态更新一遍，多数时候什么都没变；
 * 照样写一遍的话 DOM 也算变了 —— Glassium 盯着玻璃后面的内容，一变就重画那一块
 */
function setText(el, text) {
  const value = String(text ?? "");
  if (el.textContent !== value) el.textContent = value;
}

function setClass(el, value) {
  if (el.getAttribute("class") !== value) el.setAttribute("class", value);
}

/** 只在 `key` 变了时重建 `slot` 里的东西（`make` 返回要放进去的节点） */
function fill(slot, key, make) {
  if (slot.dataset.key === key) return;
  slot.dataset.key = key;
  slot.replaceChildren(make());
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

let toastTimer = 0;
function toast(text) {
  const old = document.querySelector(".toast:not(.out)");
  if (old) {
    old.classList.add("out");
    setTimeout(() => old.remove(), 240);
  }
  // Glassium 的磨砂玻璃；浮在正文上，标 overlay（Glassium 用 CSS 画，盖得住下面的字）
  const el = h("div", { class: "toast", role: "status", glass: "frosted", overlay: "" }, text);
  document.body.append(el);
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.remove(), 2700);
}

async function copy(text, what) {
  // 点的是按钮：按钮上闪一下"✓ 已复制"（卡片、节点这些不是按钮的，照旧只弹提示）
  const button = document.activeElement?.closest?.("button");
  if (button) flashCopied(button);
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const area = h("textarea", {}, text);
    document.body.append(area);
    area.select();
    document.execCommand("copy");
    area.remove();
  }
  toast(`${what}已复制`);
}

function formatBytes(n) {
  if (n < 1024) return `${Math.round(n)} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = n / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${units[unit]}`;
}

// 网络码里给人看的部分：地址。邀请码是秘密，不往界面上摆；网络 ID 太长，也不摆
function hostOf(code) {
  const text = (code || "").split("#")[0];
  const at = text.lastIndexOf("@");
  const addr = at < 0 ? text : text.slice(at + 1);
  return addr.split("/")[0];
}

// 同一个 ID 永远是同一个颜色，好认人
function colorClass(id) {
  let hash = 0;
  for (const ch of id) hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
  return `c${hash % 6}`;
}
const COLOR_HEX = ["#3d6bff", "#12b886", "#f59f00", "#e64980", "#7950f2", "#1c9fd6"];

const nameOf = (peer) => peer.name || "没起名字";

/** 头像：颜色、首字，电脑上是带颜色的玻璃泡（liquidBubble） */
function paintAvatar(el, peer, size = "") {
  const color = colorClass(peer.id);
  setClass(el, `ava${size} ${color}`);
  for (const [k, v] of Object.entries(liquidBubble(COLOR_HEX[Number(color.slice(1))], false, true))) {
    if (el.getAttribute(k) !== v) el.setAttribute(k, v);
  }
  setText(el, initialOf(peer));
}
const initialOf = (peer) => (peer.name ? [...peer.name][0] : peer.ip.split(".").pop());

/** 延迟分档：好、一般、卡；没有数是 none */
function tone(peer) {
  if (!peer.online || peer.rttMs === null || peer.rttMs === undefined) return "none";
  if (peer.rttMs < 40) return "good";
  if (peer.rttMs < 100) return "mid";
  return "bad";
}
const TONE_HEX = { good: "#4ff0a6", mid: "#ffc155", bad: "#ff7a7a", none: "rgba(232,238,255,.45)" };
const RELAY_HEX = "#b79bff";

function msText(ms) {
  return ms < 1 ? "<1" : String(ms);
}

/** 信号格亮几格 */
const barLevel = (peer) => (!peer.online || peer.rttMs == null ? 0 : peer.rttMs < 30 ? 4 : peer.rttMs < 60 ? 3 : peer.rttMs < 100 ? 2 : 1);

function bars(peer) {
  const on = barLevel(peer);
  return h("span", { class: `bars ${tone(peer)}` }, [1, 2, 3, 4].map((n) => h("i", { class: n <= on ? "on" : "" })));
}

function route(peer) {
  if (!peer.online) return h("span", { class: "route off" }, peer.route === "pending" ? (state.overview?.direct ? "○ 正在打洞" : "○ 等待中") : "○ 不在线");
  if (peer.route === "direct") return h("span", { class: "route direct", title: "两台设备之间直接连通，游戏流量不经过第三方" }, "● 直连");
  return h("span", { class: "route relay", title: "打不通直连，经中继服务器转发（全程加密，中继看不到内容）" }, "◆ 经中继");
}

/** 决定 bars()、route() 画成什么样的那几样，拼成键：键没变就不重建 */
const barsKey = (peer) => `${tone(peer)}|${barLevel(peer)}`;
const routeKey = (peer) => `${peer.online}|${peer.route}|${Boolean(state.overview?.direct)}`;

/** 几个节点（或字）装进一个片段，交给 fill */
function frag(...children) {
  const out = document.createDocumentFragment();
  out.append(...children);
  return out;
}

/** 一条延迟曲线 */
function spark(points, width, height) {
  const svg = s("svg", { class: "spark", viewBox: `0 0 ${width} ${height}`, preserveAspectRatio: "none" });
  const values = points.filter((v) => v !== null);
  if (values.length < 2) {
    svg.append(s("line", { x1: 0, x2: width, y1: height / 2, y2: height / 2, stroke: "rgba(255,255,255,.18)", "stroke-dasharray": "3 4" }));
    return svg;
  }
  const max = Math.max(...values) * 1.15 + 1;
  const min = Math.max(0, Math.min(...values) * 0.8);
  const step = width / (HISTORY - 1);
  const start = HISTORY - points.length;
  let d = "";
  points.forEach((v, i) => {
    if (v === null) return;
    const x = (start + i) * step;
    const y = height - 2 - ((height - 4) * (v - min)) / (max - min);
    d += `${d && points[i - 1] !== null ? "L" : "M"}${x.toFixed(1)} ${y.toFixed(1)}`;
  });
  const last = values[values.length - 1];
  const hex = TONE_HEX[last < 40 ? "good" : last < 100 ? "mid" : "bad"];
  svg.append(s("path", { d, fill: "none", stroke: hex, "stroke-width": 1.6, "vector-effect": "non-scaling-stroke", "stroke-linejoin": "round" }));
  return svg;
}

// ---------- 状态 ----------

const state = {
  overview: null,
  /** 用户选的页：home（网络 / 开始）、friends、admin、settings */
  page: "home",
  /** 引导里选了"建网络"还是"加入网络" */
  startTab: null,
  view: null,
  current: null,
  /** 每个朋友最近的延迟，按 ID */
  history: new Map(),
  /** 上一次的总流量，算速率用 */
  totals: null,
  speed: { rx: 0, tx: 0 },
};

function track(ov) {
  const now = performance.now();
  const peers = ov.phase === "connected" ? ov.peers : [];
  for (const peer of peers) {
    let points = state.history.get(peer.id);
    if (!points) state.history.set(peer.id, (points = []));
    points.push(peer.online && peer.rttMs != null ? peer.rttMs : null);
    if (points.length > HISTORY) points.shift();
  }
  for (const id of state.history.keys()) if (!peers.some((p) => p.id === id)) state.history.delete(id);
  const rx = peers.reduce((sum, p) => sum + p.rx, 0);
  const tx = peers.reduce((sum, p) => sum + p.tx, 0);
  if (state.totals && now > state.totals.at) {
    const seconds = (now - state.totals.at) / 1000;
    state.speed = { rx: Math.max(0, rx - state.totals.rx) / seconds, tx: Math.max(0, tx - state.totals.tx) / seconds };
  }
  state.totals = { rx, tx, at: now };
}

// ---------- 左边：通行证 ----------

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

const STATES = {
  idle: ["", "未连接"],
  connecting: ["busy", "连接中"],
  connected: ["ok", "已连接"],
  failed: ["bad", "未连上"],
};

// 左边是一条不带卡片的导航栏，直接压在点阵上：Logo、连接状态、页面、版本。
// 你的地址、网络这些放在网络页顶上那张卡片里（views.overview）
const pass = {
  build() {
    this.state = h("span", { class: "state", glass: "clear" }, h("i"), h("span"));
    // 电脑上：竖着的导航，选中项下面垫一块会飞过去的玻璃（glass-glide）。
    // 手机上：Glassium 的标签栏（底部一条玻璃胶囊，选中的气泡飞过去，按住变透镜）
    this.glide = PHONE ? null : h("span", { class: "nav-glide", glass: "clear", "glass-glide": "", "aria-hidden": "true" });
    this.nav = PHONE
      ? h("glass-tab-bar", { class: "nav", "aria-label": "页面", onchange: (event) => go(event.currentTarget.value) })
      : h("nav", { class: "nav", "aria-label": "页面" }, this.glide);
    this.navKey = "";
    this.updateChip = h("button", { class: "update-chip", glass: "tinted", "glass-tint": "rgba(79, 240, 166, 0.3)", type: "button", hidden: true, onclick: () => applyUpdate(state.overview.update.version) });
    this.version = h("div", { class: "ver" });
    this.el = h(
      "aside",
      { class: "pass" },
      h("div", { class: "brand" }, LOGO(), h("span", { class: "brand-name" }, "Meshora")),
      this.state,
      this.nav,
      h("div", { class: "pass-foot" }, this.updateChip, this.version),
    );
    return this.el;
  },

  update(ov) {
    const [cls, text] = STATES[ov.phase] || STATES.idle;
    // 状态变了：胶囊轻轻弹一下（颜色本身靠 CSS 过渡）
    if (this.stateCls !== undefined && this.stateCls !== cls) {
      this.state.classList.remove("changed");
      void this.state.offsetWidth;
    }
    const changed = this.stateCls !== undefined && this.stateCls !== cls;
    this.stateCls = cls;
    setClass(this.state, `state ${cls}${changed ? " changed" : ""}`);
    setText(this.state.lastChild, text);
    setText(this.version, `Meshora ${ov.version}`);
    const u = ov.update;
    this.updateChip.hidden = !(u.status === "available" || u.status === "downloading");
    this.updateChip.disabled = u.status === "downloading";
    setText(this.updateChip, u.status === "downloading" ? "下载中…" : `可更新到 ${u.version || ""}`);
    this.updateChip.title = u.version ? `新版本 ${u.version}：点一下更新` : "";
    this.updateNav(ov);
  },

  updateNav(ov) {
    const connected = ov.phase === "connected";
    const online = ov.peers.filter((p) => p.online).length;
    const items = connected
      ? [
          ["home", "网络", `${online}/${ov.peers.length}`],
          ["friends", "朋友", String(ov.peers.length)],
          ...(ov.roster ? [["admin", "管理", ""]] : []),
          ["settings", "设置", ""],
        ]
      : [
          ["home", ov.phase === "connecting" ? "连接中" : ov.phase === "failed" ? "没连上" : "开始", ""],
          ["settings", "设置", ""],
        ];
    const key = items.map((i) => i[0] + i[1]).join();
    if (key !== this.navKey) {
      this.navKey = key;
      this.buttons = new Map();
      this.nav.replaceChildren(
        ...(this.glide ? [this.glide] : []),
        ...items.map(([page, label]) => {
          const count = h("span", { class: "n" });
          const button = h("button", { type: "button", value: page, onclick: () => go(page) }, label, count);
          this.buttons.set(page, { button, count });
          return button;
        }),
      );
    }
    for (const [page, , count] of items) setText(this.buttons.get(page).count, count);
    if (!this.buttons.has(state.page)) state.page = "home";
    for (const [page, { button }] of this.buttons) {
      button.classList.toggle("on", page === state.page);
      button.setAttribute("aria-current", page === state.page ? "page" : "false");
    }
    // 手机上的标签栏自己管选中的气泡：跟上现在这一页（程序改 value 不派发 change）
    if (PHONE) {
      if (this.nav.value !== state.page) this.nav.value = state.page;
      return;
    }
    // 垫在选中项下面的玻璃挪过去：位置走 CSSOM，不算内联样式。
    // 放进微任务：第一次渲染时通行证刚建好、还没进文档，量不出位置
    const selected = this.buttons.get(state.page).button;
    queueMicrotask(() => {
      this.glide.style.transform = `translateY(${selected.offsetTop}px)`;
    });
  },
};

// ---------- 网络（概览） ----------

/** 网状图：你在中间，朋友沿椭圆排开；经中继的线绕过中继节点。连线结构变了才重画，延迟数字就地改 */
function meshGraph() {
  const svg = s("svg", { class: "graph first", role: "img", "aria-label": "网状图：你和网里的每个人怎么连着" });
  // 节点的玻璃泡（电脑上）：和 SVG 同一个盒子，泡的位置按 SVG 里的坐标摆
  const bubbles = h("div", { class: "graph-bubbles", "aria-hidden": "true" });
  /** 一个玻璃泡：圆心 (x, y)、半径 r，`from` 是飞出来的起点（新来的人） */
  const bubble = (x, y, r, hex, dim, from, lens = false) => {
    const el = h("span", { class: from ? "node-glass born" : "node-glass", ...liquidBubble(hex, dim, lens) });
    el.style.setProperty("--x", `${x.toFixed(1)}px`);
    el.style.setProperty("--y", `${y.toFixed(1)}px`);
    el.style.setProperty("--d", `${r * 2}px`);
    if (from) el.style.setProperty("--from", from);
    return el;
  };
  /** 延迟胶囊的玻璃：中心 (x, y)，和 SVG 里的胶囊一样大。`relay` 是中继那个方块（26×26，紫色） */
  const pill = (x, y, relay = false) => {
    const el = h("span", relay ? { class: "relay-glass", ...liquidPill("rgba(183, 155, 255, 0.35)", "8") } : { class: "pill-glass", ...liquidPill() });
    el.style.setProperty("--x", `${x.toFixed(1)}px`);
    el.style.setProperty("--y", `${y.toFixed(1)}px`);
    return el;
  };
  let signature = "";
  let labels = new Map();
  // 上一次画的时候每个人走哪条路：新来的从"我"那里飞出来，换了路的连线重新浮现，打洞打通的那条亮一下
  let drawn = null;

  function draw(ov) {
    // 量 SVG 用 getBoundingClientRect：有的安卓 WebView 对 SVG 元素的 clientWidth 一直给 0，
    // 退回默认尺寸就把节点全画到可见范围外面去了
    const box = svg.getBoundingClientRect();
    const W = Math.round(box.width) || 600;
    const H = Math.round(box.height) || 400;
    // 手机上标题和统计数字叠成两行，占得更高：图往下让出这一截
    const head = PHONE ? 64 : 0;
    const cx = W / 2;
    const cy = (H + head) / 2 + 8;
    const R = (H - head) * 0.31;
    const RX = Math.min(W * 0.36, R * 2);
    const peers = ov.peers.slice(0, 12);
    const relay = [cx, cy - R * 0.55];
    const pos = (i) => {
      const a = ((180 + (i * 360) / Math.max(peers.length, 1)) * Math.PI) / 180;
      return [cx + RX * Math.cos(a), cy + R * Math.sin(a)];
    };
    labels = new Map();
    const glass = [];
    /** 从圆心 (x1, y1) 朝 (x2, y2) 走 r，落在圆边上的那一点 */
    const edge = (x1, y1, x2, y2, r) => {
      const len = Math.hypot(x2 - x1, y2 - y1) || 1;
      return `${(x1 + ((x2 - x1) * r) / len).toFixed(1)} ${(y1 + ((y2 - y1) * r) / len).toFixed(1)}`;
    };
    /**
     * 从 (x1, y1) 到 (x2, y2) 的一段线，在中点的延迟胶囊（48×20）那里断开：胶囊是玻璃（透明的），线穿进去就看得见。
     * `gap` 为假时不断开（手机上胶囊是 SVG 画的实底，盖得住线）
     */
    const segment = (x1, y1, x2, y2, gap) => {
      if (!gap) return `M${x1} ${y1}L${x2} ${y2}`;
      const len = Math.hypot(x2 - x1, y2 - y1) || 1;
      const ux = (x2 - x1) / len;
      const uy = (y2 - y1) / len;
      // 沿着线走多远出胶囊：先碰到左右边还是上下边
      const out = Math.min(Math.abs(ux) > 1e-6 ? 24 / Math.abs(ux) : Infinity, Math.abs(uy) > 1e-6 ? 10 / Math.abs(uy) : Infinity) + 2;
      const mx = (x1 + x2) / 2;
      const my = (y1 + y2) / 2;
      const f = (v) => v.toFixed(1);
      return `M${x1} ${y1}L${f(mx - ux * out)} ${f(my - uy * out)}M${f(mx + ux * out)} ${f(my + uy * out)}L${x2} ${y2}`;
    };
    const nodes = [
      s("defs", {}, s("radialGradient", { id: "me-glow" }, s("stop", { offset: 0, "stop-color": "#7a9cff", "stop-opacity": 0.55 }), s("stop", { offset: 1, "stop-color": "#7a9cff", "stop-opacity": 0 }))),
      s("ellipse", { cx, cy, rx: RX, ry: R, fill: "none", stroke: "rgba(255,255,255,.12)", "stroke-dasharray": "2 6" }),
      s("ellipse", { cx, cy, rx: RX * 0.55, ry: R * 0.55, fill: "none", stroke: "rgba(255,255,255,.1)", "stroke-dasharray": "2 6" }),
    ];
    if (peers.some((p) => p.online && p.route === "relay")) {
      nodes.push(
        s("rect", { class: "ms-body", x: relay[0] - 13, y: relay[1] - 13, width: 26, height: 26, rx: 8, fill: "rgba(183,155,255,.18)", stroke: RELAY_HEX, "stroke-width": 1.6 }),
        s("path", { d: `M${relay[0] - 6} ${relay[1] - 3}h12M${relay[0] - 6} ${relay[1] + 3}h12`, stroke: RELAY_HEX, "stroke-width": 1.6, "stroke-linecap": "round" }),
        s("text", { class: "node-sub", x: relay[0], y: relay[1] - 20, "text-anchor": "middle" }, "中继"),
      );
      if (!PHONE) glass.push(pill(relay[0], relay[1], true));
    }
    const before = drawn;
    drawn = new Map(peers.map((peer) => [peer.id, `${peer.online}|${peer.route}`]));
    peers.forEach((peer, i) => {
      const [x, y] = pos(i);
      const viaRelay = peer.online && peer.route === "relay";
      const stroke = !peer.online ? "rgba(255,255,255,.3)" : viaRelay ? RELAY_HEX : TONE_HEX[tone(peer)];
      // 连线在两头的圆边上停住：节点是玻璃（透明的），线穿进去就看得见
      // 在线的连线中间有延迟胶囊（经中继的在中继到对方那一段）：电脑上胶囊是玻璃，线在那里断开
      const gap = peer.online && !PHONE;
      const xy = (text) => text.split(" ").map(Number);
      const [ax, ay] = xy(edge(cx, cy, viaRelay ? relay[0] : x, viaRelay ? relay[1] : y, 27));
      const [bx, by] = xy(edge(x, y, viaRelay ? relay[0] : cx, viaRelay ? relay[1] : cy, 21));
      // 经中继的：电脑上中继方块也是玻璃，线在方块边上断开（离中心 15）
      const [r1x, r1y] = !PHONE ? xy(edge(relay[0], relay[1], ax, ay, 15)) : relay;
      const [r2x, r2y] = !PHONE ? xy(edge(relay[0], relay[1], bx, by, 15)) : relay;
      const d = viaRelay
        ? `M${ax} ${ay}L${r1x} ${r1y}${segment(r2x, r2y, bx, by, gap)}`
        : segment(ax, ay, bx, by, gap);
      const id = `link-${i}`;
      const was = before?.get(peer.id);
      const now = `${peer.online}|${peer.route}`;
      // 第一次画（刚打开这一页）整张图一起淡入，不逐个动
      const born = before && was === undefined;
      const changed = before && was !== undefined && was !== now;
      const punched = changed && was.endsWith("|pending") && peer.online && peer.route === "direct";
      const linkClass = born ? "link-in late" : punched ? "link-in punched" : changed ? "link-in" : null;
      nodes.push(s("path", { id, class: linkClass, d, fill: "none", stroke, "stroke-width": 2.2, "stroke-linecap": "round", "stroke-dasharray": !peer.online ? "1 6" : viaRelay ? "6 5" : null }));
      // 直连模式正在打洞：两个光点从两端往中间跑，像两边在往对方凿
      if (!peer.online && peer.route === "pending" && ov.direct) {
        for (const points of ["0;0.5", "1;0.5"]) {
          nodes.push(
            s(
              "circle",
              { r: 3, fill: "rgba(255,193,85,.9)" },
              s("animateMotion", { dur: "1.4s", repeatCount: "indefinite", keyPoints: points, keyTimes: "0;1", calcMode: "linear" }, s("mpath", { href: `#${id}` })),
            ),
          );
        }
      }
      if (peer.online) {
        // 线上来回跑的小光点：有流量
        const dur = `${(1.2 + (peer.rttMs || 0) / 30).toFixed(2)}s`;
        for (const reverse of [false, true]) {
          nodes.push(
            s(
              "circle",
              { r: 3.2, fill: stroke },
              s(
                "animateMotion",
                reverse ? { dur, begin: "-0.7s", repeatCount: "indefinite", keyPoints: "1;0", keyTimes: "0;1", calcMode: "linear" } : { dur, repeatCount: "indefinite" },
                s("mpath", { href: `#${id}` }),
              ),
            ),
          );
        }
        // 胶囊在连线（圆边到圆边那一段）的正中
        const [mx, my] = viaRelay ? [(r2x + bx) / 2, (r2y + by) / 2] : [(ax + bx) / 2, (ay + by) / 2];
        const text = s("text", { class: "link-ms", x: mx, y: my + 4, "text-anchor": "middle", fill: TONE_HEX[tone(peer)] });
        nodes.push(s("rect", { class: "ms-body", x: mx - 24, y: my - 10, width: 48, height: 20, rx: 10, fill: "rgba(10,16,48,.6)", stroke }), text);
        glass.push(pill(mx, my));
        labels.set(peer.id, text);
      }
      const label = s("text", { class: "node-label", x, y: y + 38, "text-anchor": "middle" }, nameOf(peer));
      const node = s(
        "g",
        { class: born ? "peer-node born" : "peer-node", opacity: peer.online ? 1 : 0.5, onclick: () => copy(peer.ip, "地址") },
        s("title", {}, `${nameOf(peer)} · ${peer.ip}（点一下复制地址）`),
        // 电脑上节点是无色的玻璃：这个人的颜色留在描边上（手机上照旧是色块加白圈）
        s("circle", { class: "node-body", cx: x, cy: y, r: 21, fill: COLOR_HEX[Number(colorClass(peer.id).slice(1))], stroke: PHONE ? "rgba(255,255,255,.85)" : COLOR_HEX[Number(colorClass(peer.id).slice(1))], "stroke-width": 2.5 }),
        s("text", { x, y: y + 5, "text-anchor": "middle", fill: "#fff", "font-weight": 700, "font-size": 14 }, initialOf(peer)),
        label,
        s("text", { class: "node-sub", x, y: y + 53, "text-anchor": "middle" }, peer.ip),
      );
      const from = born ? `translate(${(cx - x).toFixed(1)}px, ${(cy - y).toFixed(1)}px) scale(0.3)` : null;
      if (from) {
        // 从"我"的位置飞出来
        node.style.setProperty("--from", from);
      }
      nodes.push(node);
      glass.push(bubble(x, y, 21, COLOR_HEX[Number(colorClass(peer.id).slice(1))], !peer.online, from, true));
    });
    nodes.push(
      // "我"身后的光晕只在手机上画：电脑上"我"是无色的玻璃，光晕画在 SVG 里会压在玻璃上面、把它染成一片蓝
      PHONE ? s("circle", { cx, cy, r: 64, fill: "url(#me-glow)" }) : null,
      s("circle", { class: "node-body", cx, cy, r: 27, fill: "#3d6bff", stroke: "#fff", "stroke-width": 3 }),
      s("text", { x: cx, y: cy + 5, "text-anchor": "middle", fill: "#fff", "font-weight": 700, "font-size": 14 }, "我"),
      s("text", { class: "node-label", x: cx, y: cy + 46, "text-anchor": "middle" }, ov.name || DEVICE),
      s("text", { class: "node-sub", x: cx, y: cy + 61, "text-anchor": "middle" }, ov.me.ip),
    );
    glass.push(bubble(cx, cy, 27, "#3d6bff", false, null, true));
    svg.replaceChildren(...nodes.filter(Boolean));
    // 电脑上才有玻璃泡（liquidBubble 在手机上是空的）：没有 glass 属性的不放
    bubbles.replaceChildren(...glass.filter((el) => el.hasAttribute("glass")));
    // 第一次画完：之后的重画不再整张淡入，只动变了的
    if (before === null) setTimeout(() => svg.classList.remove("first"), 600);
  }

  return {
    el: svg,
    bubbles,
    update(ov) {
      const box = svg.getBoundingClientRect();
      const size = `${Math.round(box.width)}x${Math.round(box.height)}`;
      const next = [size, ov.name, ...ov.peers.map((p) => `${p.id}|${p.name}|${p.online}|${p.route}|${tone(p)}`)].join(";");
      if (next !== signature) {
        signature = next;
        draw(ov);
      }
      for (const peer of ov.peers) {
        const text = labels.get(peer.id);
        if (text) setText(text, peer.rttMs == null ? "—" : `${msText(peer.rttMs)} ms`);
      }
    },
    redraw() {
      signature = "";
    },
  };
}

function friendCard(peer) {
  const avatar = h("div", { class: "ava" });
  const name = h("div", { class: "fname" });
  const ip = h("div", { class: "fip" });
  const barsSlot = h("span");
  const ms = h("span", { class: "fms" });
  const routeSlot = h("span");
  const sparkSlot = h("div");
  const why = h("div", { class: "fwhy", hidden: true });
  const el = h(
    "div",
    { class: "fcard", ...LIQUID_GLASS, "glass-corner-radius": "22", "glass-jelly": "", role: "button", tabindex: "0", title: "点一下复制地址" },
    h("div", { class: "ftop" }, avatar, h("div", { class: "who" }, name, ip), barsSlot),
    h("div", { class: "fmid" }, ms, routeSlot),
    sparkSlot,
    why,
  );
  let current = peer;
  const copyIp = () => copy(current.ip, "地址");
  el.addEventListener("click", copyIp);
  el.addEventListener("keydown", (event) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      copyIp();
    }
  });
  return {
    el,
    update(next) {
      current = next;
      el.classList.toggle("off", !next.online);
      paintAvatar(avatar, next);
      setText(name, nameOf(next));
      setText(ip, next.ip);
      fill(barsSlot, barsKey(next), () => bars(next));
      setClass(ms, `fms t-${tone(next)}`);
      const shown = next.online && next.rttMs != null;
      fill(ms, shown ? String(next.rttMs) : "", () => (shown ? frag(msText(next.rttMs), h("small", {}, "ms")) : frag("—")));
      fill(routeSlot, routeKey(next), () => route(next));
      const points = state.history.get(next.id) || [];
      fill(sparkSlot, points.join(), () => spark(points, 200, 30));
      const stuck = state.overview?.direct && next.route === "pending";
      why.hidden = !stuck;
      if (stuck) setText(why, punchHint(next));
    },
  };
}

/** 直连打洞没通时，卡在哪一边 */
/** 直连没通的朋友卡片上那句：最可能的原因 */
function punchHint(peer) {
  return punchReasons(peer, state.overview?.direct || {})[0];
}

/**
 * 直连为什么还没通：按可能性从大到小排。看的是每个地址试得怎么样（probes）和本机的情况（STUN、NAT 类型、
 * 路由器开没开端口、有没有公网 IPv6、是不是开着代理）。卡片上写第一条，诊断里全写上
 */
function punchReasons(peer, d) {
  const probes = peer.probes || [];
  const reasons = [];
  if (!probes.length) return ["码里没有能试的地址：重新交换一次连接码"];
  const v6 = probes.filter((x) => x.family === "ipv6");
  const myV6 = (d.endpoints || []).some((e) => e.startsWith("["));
  const heardFrom = probes.filter((x) => x.heard);
  const answered = probes.some((x) => x.answered);
  if (heardFrom.length && !answered) {
    reasons.push(`对方的报文到得了这里（从 ${heardFrom[0].addr} 来），我们的回不过去：对方那边的路由器或防火墙挡了入站`);
  }
  if (myV6 && v6.length && !v6.some((x) => x.answered)) {
    reasons.push(
      v6.some((x) => x.heard)
        ? "IPv6：对方的到了这里，我们的到不了对方 —— 对方光猫（或路由器）的 IPv6 防火墙挡了入站，可以在那里放行 UDP 41641"
        : "两边都有公网 IPv6 却不通：多半是光猫（或路由器）的 IPv6 防火墙挡了入站。在光猫里放行 UDP 41641，或者关掉 IPv6 防火墙",
    );
  }
  if (!myV6 && v6.length) reasons.push("对方有公网 IPv6、这边没有：这边的路由器打开 IPv6 的话，多一条好打通的路");
  if (myV6 && !v6.length) {
    reasons.push(
      d.symmetric
        ? "这边有公网 IPv6、对方没有（码里没有对方的 IPv6 地址）：这边又是对称型 NAT，IPv6 几乎是唯一能打通的路。让对方在光猫（或路由器）里打开 IPv6，再重新交换连接码"
        : "这边有公网 IPv6、对方没有（码里没有对方的 IPv6 地址）：对方在光猫（或路由器）里打开 IPv6 的话，多一条好打通的路",
    );
  }
  if (d.proxied) reasons.push("这边像是开着代理（Clash 之类）：公网 IPv4 用不了。关掉代理的 TUN 模式，或者让 Meshora 不走代理");
  if (d.symmetric && !d.mapped) {
    reasons.push(
      "这边是对称型 NAT（每个目的地换一个端口）：公网 IPv4 很难打通。用手机热点、移动流量时多半是这样，换家里的宽带试试；家里的路由器可以打开 UPnP，或者用 IPv6",
    );
  }
  if (d.checked && !d.publicEndpoint) reasons.push("没问到这边的公网地址（STUN 服务器连不上）：只有同一个局域网里的人连得上");
  if (d.upnpTried && !d.mapped) reasons.push("这边的路由器没开端口（UPnP / PCP / NAT-PMP 都没成）：在路由器里打开 UPnP 会好打很多");
  if (!heardFrom.length) reasons.push("对方的报文一次都没到：可能是你这边的路由器挡了，或者两边都是难打通的 NAT");
  reasons.push("实在打不通：换官方服务器建网络，打不通时有中继兜底");
  reasons.push("点地址卡片上的\"诊断\"，把信息发给帮你看的人");
  return reasons;
}

/**
 * 一位朋友的各个地址试得怎么样，IPv6、公网 IPv4、局域网分开写：一眼看出哪条通了、哪条被挡。
 * "发出去有回应"是往对方那边通，"对方发来过"是从对方那边到这里通
 */
function probeLines(p) {
  const probes = p.probes || [];
  if (!probes.length) return [`  （没有地址可试）`];
  const state = (x) =>
    x.working ? "通着" : [x.answered ? "回应过、现在不通" : "没回应", x.heard ? "对方发来过" : ""].filter(Boolean).join("，");
  const groups = [
    ["IPv6", "ipv6"],
    ["公网 IPv4", "ipv4"],
    ["局域网", "lan"],
  ];
  return groups
    .map(([label, family]) => {
      const list = probes.filter((x) => x.family === family);
      return list.length ? `  ${label}：${list.map((x) => `${x.addr}（${state(x)}）`).join("，")}` : null;
    })
    .filter(Boolean);
}

/** 直连模式的诊断信息：一段纯文本，复制了发给帮忙排查的人。不含私钥，有 IP 地址 */
async function directDiagnostics() {
  const ov = state.overview;
  const d = ov.direct || {};
  const lines = [
    `Meshora ${ov.version} · ${ov.platform} · 直连${d.host ? "（房主）" : "（朋友）"}`,
    `本机地址 ${ov.me?.ip || "—"}`,
    `公网端点（STUN）${d.publicEndpoint || "没问到"}${d.symmetric ? " · 两个 STUN 看到的不一样：像是对称型 NAT" : ""}`,
    `路由器开端口（UPnP / NAT-PMP）${d.mapped ? `开了 ${d.mapped}` : d.upnpTried ? "没开成" : "还在试"}`,
    `端口预测 ${d.portHint || (d.symmetric ? "没有：端口像是随机分配的，猜不出来" : "不需要（不是对称型 NAT）")}`,
    `代理 ${d.proxied ? "像是开着（STUN 域名被解析成 198.18.x.x，或者出口不止一个）：公网 IPv4 多半用不了" : "没看出来"}`,
    `本机候选 ${(d.endpoints || []).join(", ") || "（没有）"}`,
    "",
    ...ov.peers.flatMap((p) => [
      `${nameOf(p)} ${p.ip} · ${p.route}${p.online ? " · 在线" : ""} · ${p.heard ? "收到过对方报文" : "没收到过对方报文"}`,
      ...probeLines(p),
      ...(p.route === "pending" ? ["  可能的原因：", ...punchReasons(p, d).slice(0, -1).map((r) => `  - ${r}`)] : []),
    ]),
  ];
  let logs = [];
  try {
    logs = (await invoke("logs")).filter((line) => /直连|探测|报文|切换路径|STUN|UPnP|NAT-PMP|IPv6/.test(line)).slice(-40);
  } catch {}
  return [...lines, "", "日志：", ...logs].join("\n");
}

const views = {};

/** 本机当主机时，朋友连不连得进来 */
const HOST_REACH = {
  open: "本机当主机 · 朋友连得进来",
  cgnat: "本机当主机 · 运营商级 NAT，外面连不进来",
  unknown: "本机当主机 · 只有局域网里的人进得来",
};
const HOST_REACH_HINT = {
  cgnat: "你的宽带没有公网地址（运营商级 NAT）：外面的朋友连不进来。换官方服务器或者你自己的服务器建网络。",
  unknown: "路由器不支持（或者没开）UPnP，没法自动开端口：只有和你在同一个局域网的人进得来。在路由器上打开 UPnP，或者换服务器建网络。",
};

/** 网络页顶上的卡片：你的局域网地址（最要紧的一行）、你是谁、在哪个网络、断开 */
function meCard() {
  const ip = h("div", { class: "ip mono" });
  const who = h("b");
  const host = h("span", { class: "mono" });
  const coord = h("span");
  // 直连模式：房主邀请朋友，朋友再要一次回执码
  const directButton = h("button", { class: "btn", ...LIQUID_EDGE, glass: "clear", type: "button", hidden: true });
  const diagButton = h("button", { class: "btn", ...LIQUID_EDGE, glass: "clear", type: "button", hidden: true, onclick: async () => copy(await directDiagnostics(), "诊断信息") }, "诊断");
  directButton.addEventListener("click", async () => {
    if (state.overview.direct?.host) return inviteDirect();
    try {
      showReply(await invoke("direct_reply"));
    } catch (err) {
      toast(String(err));
    }
  });
  const el = h(
    "section",
    { class: "card me-card", ...LIQUID_GLASS, "glass-corner-radius": "22" },
    h(
      "div",
      { class: "me-main" },
      h("div", { class: "k" }, "你的局域网地址"),
      ip,
      h("div", { class: "who" }, "你是 ", who, " · 网络 ", host, " · ", coord),
    ),
    h(
      "div",
      { class: "me-actions" },
      h("button", { class: "btn", ...LIQUID_EDGE, glass: "tinted", "glass-tint": "#3d6bff", type: "button", onclick: () => copy(state.overview.me.ip, "地址") }, "复制地址"),
      directButton,
      diagButton,
      h("button", { class: "btn", ...LIQUID_EDGE, glass: "clear", type: "button", onclick: () => act("disconnect") }, "断开"),
    ),
  );
  return {
    el,
    update(ov) {
      setText(ip, ov.me.ip);
      setText(who, ov.name || "没起名字");
      const d = ov.direct;
      directButton.hidden = !d;
      diagButton.hidden = !d;
      if (d) {
        setText(directButton, d.host ? "邀请朋友" : "回执码");
        setText(host, d.host ? "直连（你是房主）" : "直连");
        const nat = d.publicEndpoint ? `公网 ${d.publicEndpoint}` : d.checked ? "没问到公网地址" : "正在问公网地址…";
        const upnp = d.mapped ? " · 路由器开了端口" : "";
        const v6 = (d.endpoints || []).some((e) => e.startsWith("[")) ? " · 有公网 IPv6" : "";
        const proxy = d.proxied ? " · 像是开着代理，公网 IPv4 可能用不了" : "";
        setText(coord, `${nat}${v6}${upnp}${proxy}${d.symmetric ? (d.portHint ? " · 对称型 NAT，会试着预测端口" : " · 像是对称型 NAT，可能打不通") : ""} · 网卡 ${ov.me.tun}`);
      } else {
        setText(host, hostOf(ov.network));
        setText(coord, `${ov.coordConnected ? "协调服务正常" : "协调服务重连中…"} · 网卡 ${ov.me.tun}`);
      }
    },
  };
}

views.overview = {
  mount(ov) {
    this.me = meCard();
    this.down = h("b");
    this.up = h("b");
    this.paths = h("b");
    this.graph = meshGraph();
    this.empty = h("div", { class: "empty-map" }, "还没有别人。朋友凭网络码加入后，就会出现在这里。");
    this.grid = h("div", { class: "friends" });
    this.cards = new Map();
    this.hostChip = h("span", { class: "chip", glass: "clear" });
    this.invite = h(
      "button",
      { class: "btn sm", ...LIQUID_EDGE, glass: "tinted", "glass-tint": "#3d6bff", type: "button", onclick: () => state.overview.roster?.code && copy(state.overview.roster.code, "网络码") },
      "邀请朋友",
    );
    this.extras = h("div", { class: "map-extras" }, this.invite, this.hostChip);
    this.onResize = () => {
      this.graph.redraw();
      this.graph.update(state.overview);
    };
    addEventListener("resize", this.onResize);
    const el = h(
      "div",
      { class: "view" },
      this.me.el,
      h(
        "section",
        { class: "map", ...LIQUID_GLASS, "glass-corner-radius": "26" },
        h(
          "div",
          { class: "map-head" },
          h("div", {}, h("b", {}, "你的网"), h("span", {}, "点一个人复制他的地址"), this.extras),
          h("div", { class: "stats" }, h("div", {}, h("span", {}, "下行"), this.down), h("div", {}, h("span", {}, "上行"), this.up), h("div", {}, h("span", {}, "直连 / 中继"), this.paths)),
        ),
        this.graph.bubbles,
        this.graph.el,
        this.empty,
        h("div", { class: "legend" }, h("span", { class: "l-direct" }, "直连"), h("span", { class: "l-relay" }, "经中继"), h("span", { class: "l-off" }, "不在线")),
      ),
      this.grid,
    );
    this.update(ov);
    return el;
  },
  unmount() {
    removeEventListener("resize", this.onResize);
  },
  update(ov) {
    this.me.update(ov);
    const online = ov.peers.filter((p) => p.online);
    tweenText(this.down, state.speed.rx, (v) => `${formatBytes(v)}/s`);
    tweenText(this.up, state.speed.tx, (v) => `${formatBytes(v)}/s`);
    setText(this.paths, `${online.filter((p) => p.route === "direct").length} / ${online.filter((p) => p.route === "relay").length}`);
    this.empty.hidden = ov.peers.length > 0;
    this.invite.hidden = !ov.roster?.code;
    const reach = ov.hosting?.reach;
    this.hostChip.hidden = !reach;
    setClass(this.hostChip, `chip ${reach === "open" ? "ok" : "warn"}`);
    setText(this.hostChip, HOST_REACH[reach] || "");
    this.hostChip.title = reach === "open" ? `公网地址 ${ov.hosting.publicIp}` : HOST_REACH_HINT[reach] || "";
    // 第一次 update 在 mount 里，SVG 还没进文档、量不出尺寸：放进微任务，那时已经插进去了。
    // 不用 requestAnimationFrame：窗口藏在托盘里时它不触发
    queueMicrotask(() => this.graph.update(ov));
    syncList(this.grid, this.cards, ov.peers, friendCard);
  },
};

/** 按 ID 就地更新一组行或卡片：已有的不重建（玻璃不闪），没了的拿掉 */
function syncList(container, rows, peers, make) {
  const seen = new Set();
  const ordered = peers.map((peer) => {
    seen.add(peer.id);
    let row = rows.get(peer.id);
    if (!row) {
      rows.set(peer.id, (row = make(peer)));
      // 新来的人浮上来；一开始就有的那批跟着切页的动画走，不再单独动
      if (container.isConnected) {
        row.el.classList.add("enter-item");
        setTimeout(() => row.el.classList.remove("enter-item"), 600);
      }
    }
    row.update(peer);
    return row.el;
  });
  const gone = [];
  for (const [id, row] of [...rows]) {
    if (!seen.has(id)) {
      rows.delete(id);
      gone.push(row.el);
    }
  }
  const same = ordered.length === container.children.length && ordered.every((el, i) => container.children[i] === el);
  if (same) return;
  if (!gone.length || !container.isConnected) {
    const settle = container.isConnected ? flip(container) : () => {};
    container.replaceChildren(...ordered);
    settle();
    return;
  }
  // 有人走了：先让他淡出、缩小，播完再重排，其余的从原位置滑过去
  for (const el of gone) el.classList.add("leave-item");
  setTimeout(() => {
    const settle = flip(container);
    container.replaceChildren(...ordered);
    settle();
  }, 300);
}

// ---------- 朋友 ----------

function friendRow(peer) {
  const avatar = h("div", { class: "ava sm" });
  const name = h("span");
  const cells = Array.from({ length: 7 }, () => h("td"));
  setClass(cells[3], "spark-cell");
  let current = peer;
  const el = h("tr", { title: "点一下复制地址", onclick: () => copy(current.ip, "地址") }, h("td", {}, h("div", { class: "who" }, avatar, name)), ...cells);
  return {
    el,
    update(next) {
      current = next;
      paintAvatar(avatar, next, " sm");
      setText(name, nameOf(next));
      setText(cells[0], next.ip);
      setClass(cells[0], "mono");
      fill(cells[1], routeKey(next), () => route(next));
      setClass(cells[2], `t-${tone(next)}`);
      setText(cells[2], next.online && next.rttMs != null ? `${msText(next.rttMs)} ms` : "—");
      const points = state.history.get(next.id) || [];
      fill(cells[3], points.join(), () => spark(points, 140, 24));
      setText(cells[4], next.online && next.jitterMs != null ? `${next.jitterMs} ms` : "—");
      setText(cells[5], next.online && next.lossPercent != null ? `${next.lossPercent}%` : "—");
      setClass(cells[6], "t-none");
      setText(cells[6], `${formatBytes(next.rx)} / ${formatBytes(next.tx)}`);
      el.title = `${next.id}（点一下复制地址）`;
    },
  };
}

views.friends = {
  mount(ov) {
    this.count = h("span");
    this.body = h("tbody");
    this.rows = new Map();
    this.empty = h("div", { class: "empty" }, "还没有别人。");
    this.table = h(
      "table",
      {},
      h("thead", {}, h("tr", {}, ["名字", "地址", "路径", "延迟", "最近 5 分钟", "抖动", "丢包", "收 / 发"].map((t) => h("th", {}, t)))),
      this.body,
    );
    const el = h(
      "div",
      { class: "view" },
      h("section", { class: "card", ...LIQUID_GLASS, "glass-corner-radius": "22" }, h("div", { class: "card-head" }, h("b", {}, "朋友"), this.count), this.table, this.empty),
    );
    this.update(ov);
    return el;
  },
  update(ov) {
    const online = ov.peers.filter((p) => p.online).length;
    setText(this.count, `${ov.peers.length} 人 · ${online} 在线 · 名字是对方自己起的，认人以地址为准`);
    this.table.hidden = !ov.peers.length;
    this.empty.hidden = ov.peers.length > 0;
    syncList(this.body, this.rows, ov.peers, friendRow);
  },
};

// ---------- 设置 ----------

/** 装新版本：先问一句。Windows 上客户端会退出、装好再打开；安卓上交给浏览器下载 */
async function applyUpdate(version, { found = false } = {}) {
  const ok = await confirmBox(
    found ? `发现新版本 ${version}` : `更新到 ${version}？`,
    PHONE ? "用浏览器下载新版本的安装包，下载完点开它安装。设置和私钥都留着。" : "下载、核对新版本，装好后 Meshora 会自己重新打开。连接会断开几秒，设置和私钥都留着。",
    PHONE ? "下载" : "更新",
    { cancelLabel: found ? "以后再说" : "取消", danger: false },
  );
  if (!ok) return;
  if (!PHONE) toast("正在下载新版本…");
  try {
    await invoke("apply_update");
  } catch (err) {
    toast(String(err));
  }
  refresh();
}

/** 更新的状态说成一句话 */
function updateText(ov) {
  const u = ov.update;
  if (u.status === "checking") return "正在检查…";
  if (u.status === "available") return `有新版本 ${u.version}`;
  if (u.status === "downloading") return `正在下载 ${u.version}…`;
  if (u.status === "upToDate") return "已经是最新的";
  if (u.status === "failed") return `检查失败：${u.error}`;
  return ov.checkUpdates ? "还没检查过" : "自动检查关着";
}

/** 一行：标题和说明在左，开关、小按钮靠右 */
function settingRow(title, desc, ...controls) {
  return h("div", { class: "set" }, h("div", { class: "t" }, h("b", {}, title), desc ? h("span", {}, desc) : null), ...controls);
}

/** 一行：标题和说明在上，输入框、长内容和它的按钮在下面一整行（横着放会把字挤成一列） */
function stackRow(title, desc, ...controls) {
  return h("div", { class: "set stack" }, h("div", { class: "t" }, h("b", {}, title), desc ? h("span", {}, desc) : null), h("div", { class: "ctl" }, ...controls));
}

/** 一页的标题 */
function pageHead(title, sub) {
  return h("header", { class: "page-head" }, h("h1", {}, title), sub ? h("p", {}, sub) : null);
}

/** 一张卡片：一组设置。title 可以是文字，也可以是节点（比如带人数） */
function panel(title, ...rows) {
  return h("section", { class: "card panel", ...LIQUID_GLASS, "glass-corner-radius": "22" }, title ? h("h2", { class: "panel-title" }, title) : null, ...rows);
}

/** Glassium 的开关。用户切换时调后端，失败就拨回去 */
function toggle(command, checked) {
  const el = document.createElement("glass-switch");
  el.checked = checked;
  el.addEventListener("change", async () => {
    const on = el.checked;
    try {
      await invoke(command, { on });
    } catch (err) {
      el.checked = !on;
      toast(String(err));
    }
    refresh();
  });
  return el;
}

views.settings = {
  mount(ov) {
    this.name = h("input", { ...WELL_GLASS, class: "field", maxlength: "32", spellcheck: "false", "aria-label": "你的名字" });
    this.name.value = ov.name;
    const saveName = async () => {
      const wanted = this.name.value;
      if (wanted === state.overview.name) return;
      try {
        this.name.value = await invoke("set_name", { name: wanted });
        toast("名字已保存");
      } catch (err) {
        toast(String(err));
      }
      refresh();
    };
    this.name.addEventListener("change", saveName);
    this.name.addEventListener("keydown", (event) => event.key === "Enter" && this.name.blur());

    this.broadcast = toggle("set_prefer_broadcast", ov.preferBroadcast);
    this.private = toggle("set_private_network", ov.privateNetwork);
    this.auto = toggle("set_auto_connect", ov.autoConnect);
    this.checkUpdates = toggle("set_check_updates", ov.checkUpdates);
    this.updateStatus = h("span");
    this.updateButton = h("button", {
      class: "btn sm",
      ...LIQUID_EDGE,
      glass: "clear",
      type: "button",
      onclick: async () => {
        const u = state.overview.update;
        if (u.status === "available") return applyUpdate(u.version);
        this.updateButton.disabled = true;
        try {
          await invoke("check_update");
        } catch (err) {
          toast(String(err));
        }
        this.updateButton.disabled = false;
        refresh();
      },
    });
    this.versionRow = h("div", { class: "set" }, h("div", { class: "t" }, h("b", {}, `Meshora ${ov.version}`), this.updateStatus), this.updateButton);

    this.code = h("span", { class: "code" });
    this.codeNote = h("span");
    const codeRow = stackRow(
      "网络码",
      null,
      h("span", { class: "grow" }),
      h("button", { class: "btn sm", ...LIQUID_EDGE, glass: "clear", type: "button", onclick: () => copy(state.overview.network, "网络码") }, "复制"),
      h("button", { class: "btn sm danger", ...LIQUID_EDGE, glass: "clear", type: "button", onclick: () => act("forget") }, "离开这个网络"),
    );
    codeRow.querySelector(".t").append(this.code, this.codeNote);
    this.network = panel("网络", codeRow);

    this.serverList = h("div");
    const newServer = h("input", { ...WELL_GLASS, class: "field mono-field", spellcheck: "false", placeholder: "公钥@地址:端口", "aria-label": "服务器地址" });
    const addServer = async () => {
      try {
        await invoke("add_server", { code: newServer.value });
        newServer.value = "";
        toast("记下了");
      } catch (err) {
        toast(String(err));
      }
      refresh();
    };
    newServer.addEventListener("keydown", (event) => event.key === "Enter" && addServer());
    this.serverGroup = panel(
      "我的服务器",
      this.serverList,
      stackRow("添加一台", "自己架的 meshora-coord（--hub）：公钥@地址:端口。建网络时可以选它", newServer, h("button", { class: "btn sm", ...LIQUID_EDGE, glass: "clear", type: "button", onclick: addServer }, "添加")),
    );

    this.logs = h("pre", { class: "logs", glass: "tinted", "glass-tint": "rgba(0, 0, 0, 0.22)" }, "…");
    this.logs.hidden = true;
    let timer = 0;
    const loadLogs = async () => {
      const lines = await invoke("logs");
      const atBottom = this.logs.scrollTop + this.logs.clientHeight >= this.logs.scrollHeight - 8;
      setText(this.logs, lines.length ? lines.join("\n") : "还没有日志");
      if (atBottom) this.logs.scrollTop = this.logs.scrollHeight;
    };
    const showLogs = h(
      "button",
      {
        class: "btn sm",
        ...LIQUID_EDGE,
        glass: "clear",
        type: "button",
        onclick: () => {
          const show = this.logs.hidden;
          this.logs.hidden = !show;
          setText(showLogs, show ? "收起" : "查看");
          clearInterval(timer);
          if (show) {
            loadLogs();
            timer = setInterval(loadLogs, POLL_MS);
          }
        },
      },
      "查看",
    );
    this.stopLogs = () => clearInterval(timer);

    const idRow = settingRow("你的 ID", "建网络的人要把它加进名单时用。只代表这台设备，不是密码", h("button", { class: "btn sm", ...LIQUID_EDGE, glass: "clear", type: "button", onclick: () => copy(state.overview.id, "ID") }, "复制"));
    idRow.querySelector(".t").append(h("span", { class: "code" }, ov.id));
    const el = h(
      "div",
      { class: "view" },
      pageHead("设置", "你的名字、联机方式、网络和更新"),
      panel("你", stackRow("你的名字", "网里的人看到的就是它。改了马上生效", this.name), idRow),
      panel(
        "联机",
        // 这两项是 Windows 的网卡设置（跃点数、网络类别），手机上没有
        PHONE ? null : settingRow("让游戏的广播走 Meshora（推荐）", "朋友的房间才会出现在局域网列表里。改了会重新连接", this.broadcast),
        PHONE ? null : settingRow("把 Meshora 设为专用网络", "朋友连不进你开的房间时再打开。代价：你对专用网络共享的东西（比如共享文件夹），网里的人也能访问", this.private),
        settingRow("打开时自动连接", "启动客户端时自动连上次的网络", this.auto),
      ),
      this.network,
      this.serverGroup,
      panel("更新", settingRow("自动检查更新", "每次打开时问一次 GitHub 有没有新版本。新版本有签名，核对过才装", this.checkUpdates), this.versionRow),
      panel(
        "排查",
        settingRow("日志", "出问题时复制下来，发给帮你排查的人。里面有 IP 地址，没有密钥", showLogs, h("button", { class: "btn sm", ...LIQUID_EDGE, glass: "clear", type: "button", onclick: async () => copy((await invoke("logs")).join("\n"), "日志") }, "复制")),
        this.logs,
      ),
    );
    this.update(ov);
    return el;
  },
  unmount() {
    this.stopLogs();
  },
  update(ov) {
    if (document.activeElement !== this.name && this.name.value !== ov.name) this.name.value = ov.name;
    // 程序改 checked 不派发 change：只是跟上别处的改动（比如通行证上、或者后端拒绝了）
    if (this.broadcast.checked !== ov.preferBroadcast) this.broadcast.checked = ov.preferBroadcast;
    if (this.private.checked !== ov.privateNetwork) this.private.checked = ov.privateNetwork;
    if (this.auto.checked !== ov.autoConnect) this.auto.checked = ov.autoConnect;
    const serverKey = ov.servers.join();
    if (serverKey !== this.serverKey) {
      this.serverKey = serverKey;
      this.serverList.replaceChildren(
        ...ov.servers.map((server) =>
          settingRow(hostOf(server), null, h("button", { class: "btn sm danger", ...LIQUID_EDGE, glass: "clear", type: "button", onclick: async () => { await invoke("remove_server", { code: server }); refresh(); } }, "忘掉")),
        ),
      );
    }
    if (this.checkUpdates.checked !== ov.checkUpdates) this.checkUpdates.checked = ov.checkUpdates;
    setText(this.updateStatus, updateText(ov));
    const u = ov.update;
    setText(this.updateButton, u.status === "available" ? `更新到 ${u.version}` : u.status === "checking" ? "检查中…" : "检查更新");
    if (u.status === "downloading" || u.status === "checking") this.updateButton.disabled = true;
    else if (!this.updateButton.matches(":active")) this.updateButton.disabled = false;
    this.network.hidden = !ov.network;
    setText(this.code, ov.network ? ov.network.replace(/#.*$/, "#••••••") : "");
    setText(this.codeNote, ov.network && ov.network.includes("#") ? "带着邀请码：发给谁，谁就能加入这个网络。只发给要一起玩的人" : "");
  },
};

// ---------- 加入、连接中、出错 ----------

// ---------- 确认框 ----------

/** 问一句"确定吗"。返回用户点没点确定 */
// ---------- 直连模式：交换连接码 ----------

/** NAT 情况的提醒：没问到公网地址、像是对称型 NAT */
function natWarnings(result) {
  const lines = [];
  if (result.mapped) return [h("p", { class: "ok-line" }, "路由器为你开了端口：对方不用打洞也连得进来。")];
  if (!result.public) lines.push("没问到你的公网地址（STUN 服务器连不上）：码里只有局域网地址，只有和你在同一个局域网的人连得上。");
  if (result.symmetric) lines.push("你的路由器像是对称型 NAT：直连多半打不通。打不通的话，换官方服务器建网络。");
  if (result.proxied) lines.push("像是开着代理（Clash 之类）：公网 IPv4 那条路多半用不了。同一个局域网、公网 IPv6 不受影响；要走公网 IPv4，让代理对 Meshora 直连、不接管。");
  return lines.map((line) => h("p", { class: "warn-line" }, line));
}

/** 一个盖在正文上的弹窗（CSS 毛玻璃，同 confirmBox），返回 { body, close } */
function sheet(title, text) {
  const body = h("div", { class: "sheet-body" });
  const close = () => {
    closeLayer(scrim);
    document.removeEventListener("keydown", onKey);
  };
  const onKey = (event) => event.key === "Escape" && close();
  const scrim = h(
    "div",
    { class: "modal-scrim", onclick: (event) => event.target === scrim && close() },
    h("section", { class: "dialog sheet", glass: "frosted", overlay: "", "glass-corner-radius": "26", role: "dialog", "aria-modal": "true" }, h("h2", {}, title), h("p", {}, text), body),
  );
  document.addEventListener("keydown", onKey);
  document.body.append(scrim);
  return { body, close, scrim };
}

/** 只读的连接码和"复制"按钮 */
function codeField(code, what) {
  const area = h("textarea", { ...WELL_GLASS, class: "code-area mono", readonly: "", spellcheck: "false", "aria-label": what }, code);
  area.addEventListener("focus", () => area.select());
  return h("div", { class: "code-box" }, area, h("button", { class: "btn", ...LIQUID_EDGE, glass: "tinted", "glass-tint": "#3d6bff", type: "button", onclick: () => copy(code, what) }, `复制${what}`));
}

/** 朋友：把回执码发给房主 */
function showReply(result) {
  const { body, close } = sheet("把回执码发给房主", "房主贴进去之后，两边同时开始打洞，通了就在网络页上看得到房主。");
  body.append(codeField(result.code, "回执码"), ...natWarnings(result), h("div", { class: "row end-row" }, h("button", { class: "btn", ...LIQUID_EDGE, glass: "clear", type: "button", onclick: close }, "完成")));
}

/** 房主：给一位朋友生成房主码，再收他的回执码 */
async function inviteDirect() {
  const { body, close } = sheet("邀请一位朋友", "每位朋友一个房主码：发给他，他贴进\"加入网络\"，再把回执码发回来贴在下面。");
  body.append(h("p", { class: "hint" }, "正在问公网地址…"));
  let result;
  try {
    result = await invoke("direct_offer");
  } catch (err) {
    body.replaceChildren(h("p", { class: "field-error" }, String(err)), h("div", { class: "row end-row" }, h("button", { class: "btn", ...LIQUID_EDGE, glass: "clear", type: "button", onclick: close }, "关闭")));
    return;
  }
  const reply = h("textarea", { ...WELL_GLASS, class: "code-area mono", spellcheck: "false", placeholder: "meshora-reply: 开头的回执码", "aria-label": "回执码" });
  const error = h("div", { class: "field-error", role: "alert" });
  const accept = h("button", { class: "btn", ...LIQUID_EDGE, glass: "tinted", "glass-tint": "#3d6bff", type: "button" }, "加进来");
  accept.addEventListener("click", async () => {
    setText(error, "");
    accept.disabled = true;
    try {
      const name = await invoke("direct_accept", { code: reply.value });
      toast(`${name || "朋友"}加进来了，正在打洞`);
      close();
      refresh();
    } catch (err) {
      setText(error, String(err));
    } finally {
      accept.disabled = false;
    }
  });
  reply.addEventListener("input", () => (error.textContent = ""));
  body.replaceChildren(
    h("div", { class: "step" }, h("b", {}, "1. 发给朋友"), codeField(result.code, "房主码")),
    ...natWarnings(result),
    h("div", { class: "step" }, h("b", {}, "2. 贴上他发回来的回执码"), reply),
    error,
    h("div", { class: "row end-row" }, h("button", { class: "btn", ...LIQUID_EDGE, glass: "clear", type: "button", onclick: close }, "以后再说"), accept),
  );
}

// 盖在正文上的层（确认框）不用 Glassium：它的玻璃画在页面底下，盖不住上面的字，字会透上来。
// 框是 Glassium 的玻璃，标了 overlay：盖在正文上，Glassium 用 CSS 画（GPU 玻璃在页面底下，盖不住上面的字）；
// 里面的按钮跟着一起用 CSS 画
function confirmBox(title, text, okLabel, { cancelLabel = "取消", danger = true } = {}) {
  return new Promise((resolve) => {
    const close = (answer) => {
      closeLayer(scrim);
      document.removeEventListener("keydown", onKey);
      resolve(answer);
    };
    const onKey = (event) => event.key === "Escape" && close(false);
    const ok = h("button", { class: `btn wide${danger ? " danger" : ""}`, ...LIQUID_EDGE, glass: danger ? "clear" : "tinted", "glass-tint": danger ? null : "#3d6bff", type: "button", onclick: () => close(true) }, okLabel);
    const scrim = h(
      "div",
      { class: "modal-scrim", onclick: (event) => event.target === scrim && close(false) },
      h(
        "section",
        { class: "dialog narrow", glass: "frosted", overlay: "", "glass-corner-radius": "26", role: "alertdialog", "aria-modal": "true" },
        h("h2", {}, title),
        h("p", {}, text),
        h("div", { class: "row center-row" }, h("button", { class: "btn", ...LIQUID_EDGE, glass: "clear", type: "button", onclick: () => close(false) }, cancelLabel), ok),
      ),
    );
    document.addEventListener("keydown", onKey);
    document.body.append(scrim);
    setTimeout(() => ok.focus(), 30);
  });
}

// ---------- 第一次打开：引导 ----------

views.onboarding = {
  mount(ov) {
    this.step = 0;
    this.body = h("div", { class: "welcome" });
    const el = h("div", { class: "view center" }, h("section", { class: "dialog welcome-card", ...LIQUID_GLASS, "glass-corner-radius": "28" }, this.body));
    this.draw(ov);
    return el;
  },
  draw(ov) {
    if (this.step === 0) {
      const name = h("input", { ...WELL_GLASS, class: "field wide-field", maxlength: "32", spellcheck: "false", "aria-label": "你的名字" });
      name.value = ov.name;
      const next = async () => {
        const wanted = name.value.trim();
        if (!wanted) {
          toast("起个名字吧，朋友看到的就是它");
          name.focus();
          return;
        }
        try {
          await invoke("set_name", { name: wanted });
        } catch (err) {
          toast(String(err));
          return;
        }
        this.step = 1;
        this.draw(ov);
      };
      name.addEventListener("keydown", (event) => event.key === "Enter" && next());
      this.body.replaceChildren(
        h("div", { class: "welcome-logo" }, LOGO()),
        h("h1", {}, "欢迎使用 Meshora"),
        h("p", {}, "和朋友组成一个虚拟局域网：不管在哪，打局域网游戏就像坐在一起。"),
        h("label", { class: "label" }, "朋友会看到你叫"),
        name,
        h("div", { class: "row center-row" }, h("button", { class: "btn wide", ...LIQUID_EDGE, glass: "tinted", "glass-tint": "#3d6bff", type: "button", onclick: next }, "下一步")),
      );
      setTimeout(() => name.focus(), 50);
      return;
    }
    const pick = async (tab) => {
      state.startTab = tab;
      try {
        await invoke("set_onboarded");
      } catch (err) {
        toast(String(err));
      }
      refresh();
    };
    this.body.replaceChildren(
      h("h1", {}, "你想做什么？"),
      h("p", {}, "随时都能换：建了网络也能去加入别人的。"),
      h(
        "div",
        { class: "paths" },
        h("button", { class: "path", glass: "clear", "glass-corner-radius": "22", type: "button", onclick: () => pick("create") }, h("b", {}, "建一个网络"), h("span", {}, "我来当网主，把网络码发给朋友")),
        h("button", { class: "path", glass: "clear", "glass-corner-radius": "22", type: "button", onclick: () => pick("join") }, h("b", {}, "加入朋友的网络"), h("span", {}, "朋友已经发给我一个网络码")),
      ),
    );
  },
  update() {},
};

// ---------- 没连上时：建网络 / 加入网络 ----------

/** 建网络选位置：一组单选按钮 */
function choices(options, selected, onPick) {
  const buttons = options.map((option) =>
    h(
      "button",
      { class: "choice", type: "button", role: "radio", "aria-checked": String(option.value === selected), disabled: option.disabled || null, onclick: () => pick(option.value) },
      h("b", {}, option.label),
      h("span", {}, option.hint),
    ),
  );
  // 选中的那一项下面垫一块 Glassium 的玻璃，换选中时滑过去（app.css 的 transition；玻璃每帧跟着元素走）
  const glide = h("span", { class: "choice-glide", glass: "tinted", "glass-tint": "rgba(122, 156, 255, 0.22)", "aria-hidden": "true" });
  const place = () => {
    const on = buttons.find((button) => button.getAttribute("aria-checked") === "true");
    glide.hidden = !on;
    if (!on) return;
    glide.style.setProperty("--y", `${on.offsetTop}px`);
    glide.style.setProperty("--h", `${on.offsetHeight}px`);
  };
  const pick = (value) => {
    buttons.forEach((button, i) => button.setAttribute("aria-checked", String(options[i].value === value)));
    place();
    onPick(value);
  };
  const group = h("div", { class: "choices", role: "radiogroup" }, glide, buttons);
  // 第一次量要等它进了文档、排好版
  setTimeout(place, 0);
  return group;
}

views.start = {
  mount(ov) {
    // ---- 建网络 ----
    this.netName = h("input", { ...WELL_GLASS, class: "field wide-field", maxlength: "32", spellcheck: "false", "aria-label": "网络名" });
    this.netName.value = `${ov.name || "我"}的网络`;
    this.where = ov.officialServer ? "official" : PHONE ? "server" : "thisPc";
    this.server = h("input", { ...WELL_GLASS, class: "field wide-field mono-field", spellcheck: "false", placeholder: "公钥@地址:端口", "aria-label": "服务器地址" });
    this.server.value = ov.servers[0] || "";
    const serverRow = h("div", { class: "server-row" }, this.server);
    serverRow.hidden = this.where !== "server";
    const createError = h("div", { class: "field-error", role: "alert" });
    const createButton = h("button", { class: "btn wide", ...LIQUID_EDGE, glass: "tinted", "glass-tint": "#3d6bff", type: "button" }, "建网络");
    const options = [
      { value: "official", label: "官方服务器", hint: ov.officialServer ? "最省事：朋友在哪都能连进来" : "还没上线", disabled: !ov.officialServer },
      // 手机多半在运营商级 NAT 后面，换个网络地址就变，当不了主机
      PHONE ? null : { value: "thisPc", label: "本机当主机", hint: "不用服务器。路由器要支持 UPnP，这台电脑开着网络才在" },
      { value: "server", label: "我的服务器", hint: "自己架的 meshora-coord（--hub）" },
      { value: "direct", label: "不用服务器（直连）", hint: "和每位朋友互发一段连接码，靠打洞直连。没有中继兜底，对称型 NAT 连不上" },
    ].filter(Boolean);
    const picker = choices(options, this.where, (value) => {
      this.where = value;
      serverRow.hidden = value !== "server";
      setText(createError, "");
    });
    createButton.addEventListener("click", async () => {
      setText(createError, "");
      if (this.where === "direct") {
        createButton.disabled = true;
        try {
          await invoke("direct_host");
          state.page = "home";
          refresh();
          inviteDirect();
        } catch (err) {
          setText(createError, String(err));
        } finally {
          createButton.disabled = false;
        }
        return;
      }
      const name = this.netName.value.trim();
      if (!name) {
        setText(createError, "给网络起个名字");
        return;
      }
      let at = { kind: this.where };
      if (this.where === "server") {
        try {
          at = { kind: "server", code: await invoke("add_server", { code: this.server.value }) };
        } catch (err) {
          setText(createError, String(err));
          return;
        }
      }
      createButton.disabled = true;
      try {
        await invoke("create", { at, name });
        state.page = "home";
        refresh();
      } catch (err) {
        setText(createError, String(err));
      } finally {
        createButton.disabled = false;
      }
    });
    this.create = h(
      "section",
      { class: "dialog", ...LIQUID_GLASS, "glass-corner-radius": "26" },
      h("h2", {}, "建一个网络"),
      h("p", {}, "你当网主，把网络码发给朋友。"),
      h("label", { class: "label" }, "网络名"),
      this.netName,
      h("label", { class: "label" }, "建在哪"),
      picker,
      serverRow,
      createError,
      h("div", { class: "row end-row" }, createButton),
    );

    // ---- 加入网络 ----
    const area = h("textarea", { ...WELL_GLASS, placeholder: "网络码，或者房主发来的房主码（meshora-offer: 开头）", spellcheck: "false", "aria-label": "网络码" });
    const error = h("div", { class: "field-error", role: "alert" });
    const button = h("button", { class: "btn wide", ...LIQUID_EDGE, glass: "tinted", "glass-tint": "#3d6bff", type: "button" }, "加入");
    const submit = async () => {
      const code = area.value.trim();
      if (!code) {
        setText(error, "先粘贴一个网络码");
        area.focus();
        return;
      }
      setText(error, "");
      if (code.startsWith("meshora-reply:")) {
        setText(error, "这是回执码：要贴在房主的\"邀请朋友\"里，不是这里");
        return;
      }
      button.disabled = true;
      try {
        if (code.startsWith("meshora-offer:")) {
          const result = await invoke("direct_join", { code });
          state.page = "home";
          refresh();
          showReply(result);
          return;
        }
        await invoke("connect", { code });
        refresh();
      } catch (err) {
        setText(error, String(err));
      } finally {
        button.disabled = false;
      }
    };
    button.addEventListener("click", submit);
    area.addEventListener("keydown", (event) => {
      if (event.key === "Enter" && !event.shiftKey) {
        event.preventDefault();
        submit();
      }
    });
    area.addEventListener("input", () => (error.textContent = ""));
    this.area = area;
    this.savedHost = h("span", { class: "mono" });
    this.saved = h(
      "div",
      { class: "row split" },
      h("span", { class: "hint" }, "上次的网络 ", this.savedHost),
      h("button", { class: "btn sm", ...LIQUID_EDGE, glass: "clear", type: "button", onclick: () => act("connect", {}) }, "重新连接"),
      h("button", { class: "btn sm danger", ...LIQUID_EDGE, glass: "clear", type: "button", onclick: () => act("forget") }, "忘掉"),
    );
    this.join = h(
      "section",
      { class: "dialog", ...LIQUID_GLASS, "glass-corner-radius": "26" },
      h("h2", {}, "加入朋友的网络"),
      h("p", {}, "把朋友发给你的网络码（或者直连的房主码）贴进来。"),
      area,
      error,
      h("div", { class: "row" }, h("span", { class: "hint" }, "网络码里没有邀请码？", h("button", { class: "link", type: "button", onclick: () => copy(state.overview.id, "ID") }, "复制你的 ID"), " 发给建网络的人"), button),
      this.saved,
    );
    const el = h(
      "div",
      { class: "view" },
      pageHead("开始联机", "建一个网络把朋友拉进来，或者贴上朋友发来的网络码加入。"),
      h("div", { class: "start" }, this.create, this.join),
    );
    this.update(ov);
    // 引导里选了哪个，就先把光标放在哪
    const tab = state.startTab;
    state.startTab = null;
    setTimeout(() => {
      const target = tab === "create" ? this.netName : area;
      target.focus();
      // 手机上两张卡上下排，加入那张在下面：滚过去，不然弹出来的键盘正好把它挡住
      target.scrollIntoView({ block: "center" });
    }, 50);
    return el;
  },
  update(ov) {
    this.saved.hidden = !ov.network && !ov.direct;
    setText(this.savedHost, ov.direct ? (ov.direct.host ? "直连网络（你是房主）" : "直连网络") : hostOf(ov.network));
  },
};

// ---------- 网主：管理网络 ----------

function inviteText(invite) {
  const parts = [];
  if (invite.usesLeft !== null && invite.usesLeft !== undefined) parts.push(invite.usesLeft === 1 ? "一次性" : `还能用 ${invite.usesLeft} 次`);
  if (invite.expires) {
    const left = invite.expires * 1000 - Date.now();
    const hours = Math.max(0, Math.round(left / 3600_000));
    parts.push(hours >= 24 ? `${Math.round(hours / 24)} 天后过期` : hours >= 1 ? `${hours} 小时后过期` : "快过期了");
  }
  return parts.join(" · ") || "长期有效";
}

async function admin(action, done) {
  try {
    const code = await invoke("admin", { action });
    if (done) done(code);
  } catch (err) {
    toast(String(err));
  }
  refresh();
}

views.admin = {
  mount(ov) {
    this.title = h("input", { ...WELL_GLASS, class: "field", maxlength: "32", spellcheck: "false", "aria-label": "网络名" });
    this.title.addEventListener("change", () => {
      const name = this.title.value.trim();
      if (name && name !== state.overview.roster?.name) admin({ kind: "rename", name }, () => toast("网络名已保存"));
    });
    this.title.addEventListener("keydown", (event) => event.key === "Enter" && this.title.blur());
    this.code = h("span", { class: "code" });
    this.invites = h("div", { class: "invite-list" });
    this.members = h("div", { class: "member-list" });
    this.memberCount = h("span");
    const copyCode = () => state.overview.roster?.code && copy(state.overview.roster.code, "网络码");
    const el = h(
      "div",
      { class: "view" },
      pageHead("管理", "你是这个网络的网主：邀请朋友、管成员"),
      panel("网络", stackRow("网络名", "网里的人在你的网络码旁边看到的名字", this.title)),
      panel(
        "邀请朋友",
        stackRow(
          "网络码",
          "发给谁，谁就能加入。泄露了就换一个：旧的立刻作废，已经在网里的人不受影响",
          h("span", { class: "grow" }),
          h("button", { class: "btn sm", ...LIQUID_EDGE, glass: "clear", type: "button", onclick: copyCode }, "复制"),
          h(
            "button",
            {
              class: "btn sm",
              ...LIQUID_EDGE,
              glass: "clear",
              type: "button",
              onclick: async () => {
                if (await confirmBox("换一个网络码？", "旧的网络码立刻作废。已经在网里的人不受影响，还没加入的要用新的。", "换"))
                  admin({ kind: "rotateInvite" }, (code) => code && copy(code, "新的网络码"));
              },
            },
            "换一个",
          ),
        ),
        this.invites,
        h(
          "div",
          { class: "row" },
          h("span", { class: "hint" }, "只想邀请一个人？发一个用一次就作废、或者到时候就过期的："),
          h("button", { class: "btn sm", ...LIQUID_EDGE, glass: "clear", type: "button", onclick: () => admin({ kind: "newInvite", uses: 1, hours: null }, (code) => code && copy(code, "一次性网络码")) }, "一次性"),
          h("button", { class: "btn sm", ...LIQUID_EDGE, glass: "clear", type: "button", onclick: () => admin({ kind: "newInvite", uses: null, hours: 24 }, (code) => code && copy(code, "24 小时网络码")) }, "24 小时"),
        ),
      ),
      panel(h("span", {}, "成员 ", this.memberCount), this.members),
      panel(
        "解散",
        settingRow(
          "解散这个网络",
          "所有人立刻断开，网络码作废，网络删掉。不能撤销",
          h(
            "button",
            {
              class: "btn sm danger",
              ...LIQUID_EDGE,
              glass: "clear",
              type: "button",
              onclick: async () => {
                if (await confirmBox("解散这个网络？", "所有人立刻断开，网络码作废，网络删掉。不能撤销。", "解散")) {
                  state.page = "home";
                  admin({ kind: "delete" }, () => toast("网络解散了"));
                }
              },
            },
            "解散",
          ),
        ),
      ),
    );
    el.querySelectorAll(".set.stack .t")[1].append(this.code);
    this.update(ov);
    return el;
  },
  update(ov) {
    const roster = ov.roster;
    if (!roster) return;
    if (document.activeElement !== this.title && this.title.value !== roster.name) this.title.value = roster.name;
    setText(this.code, roster.code ? roster.code.replace(/#.*$/, "#••••••") : "");
    const inviteKey = roster.invites.map((i) => i.invite + i.usesLeft + i.expires).join();
    if (inviteKey !== this.inviteKey) {
      this.inviteKey = inviteKey;
      this.invites.replaceChildren(
        ...roster.invites.map((invite) =>
          h(
            "div",
            { class: "set" },
            h("div", { class: "t" }, h("b", {}, inviteText(invite)), h("span", { class: "code" }, `#${invite.invite.slice(0, 6)}…`)),
            h("button", { class: "btn sm", ...LIQUID_EDGE, glass: "clear", type: "button", onclick: () => copy(invite.code, "网络码") }, "复制"),
            h("button", { class: "btn sm danger", ...LIQUID_EDGE, glass: "clear", type: "button", onclick: () => admin({ kind: "revokeInvite", invite: invite.invite }, () => toast("作废了")) }, "作废"),
          ),
        ),
      );
    }
    setText(this.memberCount, `${roster.members.length} 人 · ${roster.members.filter((m) => m.online).length} 在线`);
    const memberKey = roster.members.map((m) => m.id + m.name + m.online).join();
    if (memberKey !== this.memberKey) {
      this.memberKey = memberKey;
      this.members.replaceChildren(
        ...roster.members.map((member) =>
          h(
            "div",
            { class: "set" },
            h("div", { class: `ava sm ${colorClass(member.id)}` }, initialOf(member)),
            h("div", { class: "t" }, h("b", {}, nameOf(member), member.owner ? h("span", { class: "tag" }, "网主") : null), h("span", { class: "mono" }, member.ip)),
            h("span", { class: member.online ? "route direct" : "route off" }, member.online ? "● 在线" : "○ 不在线"),
            member.owner
              ? null
              : h(
                  "button",
                  {
                    class: "btn sm danger",
                    ...LIQUID_EDGE,
                    glass: "clear",
                    type: "button",
                    onclick: async () => {
                      if (await confirmBox(`把 ${nameOf(member)} 移出网络？`, "他马上断开。手里的网络码还有效的话，他还能再加回来 —— 想让他回不来，接着换一个网络码。", "移出"))
                        admin({ kind: "kick", id: member.id }, () => toast("移出了"));
                    },
                  },
                  "移出",
                ),
          ),
        ),
      );
    }
  },
};

views.connecting = {
  mount(ov) {
    this.host = h("p", { class: "mono" });
    const el = h(
      "div",
      { class: "view center" },
      h(
        "section",
        { class: "dialog narrow", ...LIQUID_GLASS, "glass-corner-radius": "26" },
        h("div", { class: "pulse", role: "progressbar", "aria-label": "正在连接" }, h("i"), h("i"), h("b")),
        h("h2", {}, "正在连接"),
        this.host,
        h("button", { class: "btn", ...LIQUID_EDGE, glass: "clear", type: "button", onclick: () => act("disconnect") }, "取消"),
      ),
    );
    this.update(ov);
    return el;
  },
  update(ov) {
    setText(this.host, hostOf(ov.network));
  },
};

const FAILURES = {
  rejected: {
    title: "还没被加进这个网络",
    hint: "这个网络码里没有邀请码，网络只认名单。向建网络的人要一个带邀请码的网络码；或者把你的 ID 发给对方，加进名单后再点「重试」。",
    showId: true,
  },
  // 下面几种是 rejected 的细分，按协调服务给的原因认
  rejectedInvite: {
    title: "邀请码不对",
    hint: "邀请码可能已经换过了。向建网络的人要一个最新的网络码，点「换一个网络」贴进去。",
  },
  rejectedClosed: {
    title: "这个网络不接受邀请",
    hint: "网络码里带着邀请码，但这个网络只认名单。把你的 ID 发给建网络的人，加进名单后再点「重试」。",
    showId: true,
  },
  rejectedKicked: {
    title: "你已被移出这个网络",
    hint: "建网络的人把你移出了。想回来的话，向对方要一个新的网络码。",
  },
  rejectedDeleted: {
    title: "这个网络解散了",
    hint: "网主解散了这个网络。点「换一个网络」去建一个新的，或者加入别的网络。",
  },
  rejectedNoNetwork: {
    title: "找不到这个网络",
    hint: "网络可能已经被网主解散了，或者网络码抄错了。向建网络的人要一个新的网络码。",
  },
  rejectedCreate: {
    title: "建不了网络",
    hint: "服务器不让建：看下面的原因（比如建的网络太多了）。",
  },
  rejectedFull: {
    title: "网络已满",
    hint: "这个网络的人数到上限了。联系建网络的人。",
  },
  unreachable: {
    title: "连不上协调服务",
    hint: "检查网络码里的地址和端口对不对、服务器开着没有、本机能不能上网。",
  },
  resolve: {
    title: "找不到这个地址",
    hint: "网络码里的域名解析不出来。检查有没有拼错，或者本机的网络。",
  },
  tun: {
    title: "虚拟网卡没建起来",
    hint: "客户端要以管理员身份运行，程序旁边还要有 wintun.dll。重新安装一次通常就好。",
  },
  // 下面两种是 tun 的细分，按出错信息认
  tunDriver: {
    title: "缺少 wintun.dll",
    hint: "Meshora 的虚拟网卡靠 wintun.dll，它应该和 meshora.exe 在同一个文件夹里。重新安装一次通常就好。",
  },
  tunAdmin: {
    title: "需要管理员权限",
    hint: "建虚拟网卡需要管理员权限。关掉客户端，右键「以管理员身份运行」。",
  },
  // 安卓上 tun 的两种：没给 VPN 权限、别的原因
  vpnDenied: {
    title: "没有允许建立 VPN 连接",
    hint: "Meshora 靠系统的 VPN 功能组网（只接管网里的地址，上网不受影响）。点「重试」，在弹出的对话框里点「确定」。",
  },
  vpn: {
    title: "VPN 连接没建起来",
    hint: "手机上开着别的 VPN 的话，先关掉它再点「重试」。系统同一时间只让一个 VPN 工作。",
  },
  bind: {
    title: "端口被占用",
    hint: "有别的程序占着 Meshora 要用的 UDP 端口。关掉它再试。",
  },
  stopped: {
    title: "连接断了",
    hint: "和网络的连接意外结束了。点「重试」重新连接。",
  },
  other: {
    title: "出了点问题",
    hint: "点「重试」再试一次。还不行的话，在设置里复制日志，发给建网络的人或者开发者。",
  },
};

function failureKind(error) {
  const { kind, message } = error;
  if (kind === "rejected" && message.includes("移出")) return "rejectedKicked";
  if (kind === "rejected" && message.includes("邀请码不对")) return "rejectedInvite";
  if (kind === "rejected" && message.includes("不接受凭邀请码")) return "rejectedClosed";
  if (kind === "rejected" && message.includes("网络已满")) return "rejectedFull";
  if (kind === "rejected" && message.includes("解散")) return "rejectedDeleted";
  if (kind === "rejected" && message.includes("找不到这个网络")) return "rejectedNoNetwork";
  if (kind === "rejected" && /建网络|网络已经满|不让别人建|最多 \d+ 个/.test(message)) return "rejectedCreate";
  if (kind === "tun" && PHONE) return /没有允许/.test(message) ? "vpnDenied" : "vpn";
  if (kind === "tun" && /wintun\.dll/i.test(message)) return "tunDriver";
  if (kind === "tun" && /拒绝访问|access is denied|\(os error 5\)/i.test(message)) return "tunAdmin";
  return kind;
}

views.failed = {
  mount(ov) {
    this.title = h("h2", { class: "err" });
    this.hint = h("p");
    this.detail = h("div", { class: "detail", glass: "tinted", "glass-tint": "rgba(0, 0, 0, 0.22)" });
    this.copyId = h("button", { class: "link", type: "button", onclick: () => copy(state.overview.id, "ID") }, "复制你的 ID");
    const el = h(
      "div",
      { class: "view center" },
      h(
        "section",
        { class: "dialog", ...LIQUID_GLASS, "glass-corner-radius": "26" },
        this.title,
        this.hint,
        this.detail,
        h(
          "div",
          { class: "row" },
          h("button", { class: "btn wide", ...LIQUID_EDGE, glass: "tinted", "glass-tint": "#3d6bff", type: "button", onclick: () => act("connect", {}) }, "重试"),
          h("button", { class: "btn", ...LIQUID_EDGE, glass: "clear", type: "button", onclick: () => act("forget") }, "换一个网络"),
          h("span", { class: "grow" }),
          this.copyId,
        ),
      ),
    );
    this.update(ov);
    return el;
  },
  update(ov) {
    const info = FAILURES[failureKind(ov.error)] || FAILURES.other;
    setText(this.title, info.title);
    setText(this.hint, info.hint);
    setText(this.detail, ov.error.message);
    this.copyId.hidden = !info.showId;
  },
};

// ---------- 主循环 ----------

let main;

/** 自绘的标题栏：拖它移动窗口；最小化、关（关 = 藏到托盘，连接不断） */
function titlebar() {
  const win = () => window.__TAURI__?.window?.getCurrentWindow?.();
  const icon = (d) => s("svg", { viewBox: "0 0 12 12", "aria-hidden": "true" }, s("path", { d, stroke: "currentColor", "stroke-width": 1.4, "stroke-linecap": "round", fill: "none" }));
  return h(
    "header",
    { class: "titlebar", "data-tauri-drag-region": "" },
    h("span", { class: "grow", "data-tauri-drag-region": "" }),
    h("button", { class: "tb-btn", type: "button", title: "最小化", "aria-label": "最小化", onclick: () => win()?.minimize() }, icon("M2.5 6h7")),
    h("button", { class: "tb-btn close", type: "button", title: "关到托盘（连接不断）", "aria-label": "关闭", onclick: () => win()?.hide() }, icon("M3 3l6 6M9 3l-6 6")),
  );
}

function shell() {
  main = h("main");
  document.getElementById("app").replaceChildren(pass.build(), h("div", { class: "column" }, PHONE ? null : titlebar(), main));
}

/** 切页的动画：新页面里的卡片依次浮上来（样式在 app.css 的"动画"一节）。跑完把 class 摘掉，后面的更新不再触发 */
function enter(view) {
  [...view.children].forEach((child, i) => child.style.setProperty("--i", String(Math.min(i, 8))));
  // 再播一遍（启动动画走完时）：先摘掉、让浏览器认一次，再加上
  view.classList.remove("enter");
  void view.offsetWidth;
  view.classList.add("enter");
  setTimeout(() => view.classList.remove("enter"), 1400);
  return view;
}

function viewFor(ov) {
  if (!ov.onboarded) return "onboarding";
  if (state.page === "settings") return "settings";
  if (ov.phase === "connected") {
    if (state.page === "admin" && ov.roster) return "admin";
    return state.page === "friends" ? "friends" : "overview";
  }
  if (ov.phase === "connecting") return "connecting";
  if (ov.phase === "failed") return "failed";
  return "start";
}

/** 打开客户端后查到新版本：问一次要不要更新。点了"以后再说"，这次运行里不再问，左上角的"可更新"还在 */
let updateAsked = false;
function offerUpdate(ov) {
  if (updateAsked || ov.update?.status !== "available" || !ov.onboarded) return;
  updateAsked = true;
  applyUpdate(ov.update.version, { found: true });
}

/** 页面的先后：往后翻时新页从右边来，往回翻时从左边来 */
const PAGE_ORDER = ["onboarding", "start", "connecting", "failed", "overview", "friends", "admin", "settings"];

function render(ov) {
  state.overview = ov;
  offerUpdate(ov);
  document.getElementById("app").classList.toggle("solo", !ov.onboarded);
  pass.update(ov);
  const name = viewFor(ov);
  if (name !== state.view) {
    state.current?.unmount?.();
    const dir = PAGE_ORDER.indexOf(name) >= PAGE_ORDER.indexOf(state.view) ? 1 : -1;
    state.view = name;
    state.current = Object.create(views[name]);
    const next = state.current.mount(ov);
    // 启动动画还盖着：直接换上，不播换页和卡片浮上来 —— 动画走完时统一浮一次
    if (document.documentElement.classList.contains("booting")) main.replaceChildren(next);
    else swapView(main, next, dir, enter);
    main.scrollTop = 0;
  } else {
    state.current.update(ov);
  }
}

function go(page) {
  state.page = page;
  if (state.overview) render(state.overview);
}

/** 上一次的状态（JSON）：没变就不重画 */
let lastOverview = "";

async function refresh() {
  try {
    const ov = await invoke("overview");
    track(ov);
    const json = JSON.stringify(ov);
    // 连着的时候延迟曲线每秒往前走，照样要画；别的时候状态没变就什么都不动
    if (json === lastOverview && ov.phase !== "connected") return;
    lastOverview = json;
    render(ov);
  } catch (err) {
    console.error(err);
  }
}

/** 每秒问一次状态；窗口藏起来（托盘里、最小化、手机切到后台）时不问，回来马上问一次 */
let polling = 0;
function poll() {
  clearInterval(polling);
  polling = document.hidden ? 0 : setInterval(refresh, POLL_MS);
}
document.addEventListener("visibilitychange", () => {
  if (!document.hidden) refresh();
  poll();
});

async function act(command, args) {
  try {
    await invoke(command, args);
  } catch (err) {
    toast(String(err));
  }
  // 断开、换网络之后回到首页
  if (command !== "connect") state.page = "home";
  refresh();
}

shell();
// 启动动画：状态定下来了（不在"连接中"，或者等够了），Logo 飞到左上角，界面浮上来，当前这一页的卡片依次浮一遍。
// 动画期间换页不播动画，只在最后浮这一次
let settled;
const ready = new Promise((resolve) => (settled = resolve));
intro(ready).then((played) => {
  clearInterval(settling);
  const view = main.firstElementChild;
  if (played && view) enter(view);
});
const firstLook = () => {
  if (state.overview?.phase !== "connecting") settled();
};
refresh().then(firstLook, settled);
// 自动连接时头一两秒在"连接中"：启动动画等它连上（最多等到动画的上限），免得刚浮上来就又换一页
const settling = setInterval(() => {
  refresh().then(firstLook);
}, 200);
ready.then(() => clearInterval(settling));
poll();
