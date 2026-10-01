// Meshora 桌面客户端的界面。所有状态来自 Rust 那一侧的 overview 命令，每秒拉一次；按钮只是把意图转过去。
//
// 两条规矩：
// - 数据一律用 textContent 放进页面（朋友的名字是别人随便填的），不拼 HTML
// - CSP 不许内联样式：不写 style= 属性，要动的位置走 CSSOM（el.style.xxx）
import glassium from "./vendor/glassium/index.js";

// 开关是 Glassium 的组件：它们后面的背景也要收进场景，玻璃才折射得到
glassium.configure({ absorbForComponents: true });

const invoke = (command, args) => window.__TAURI__.core.invoke(command, args);

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
  document.querySelector(".toast")?.remove();
  const el = h("div", { class: "toast", role: "status" }, text);
  document.body.append(el);
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.remove(), 1900);
}

async function copy(text, what) {
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

function bars(peer) {
  const on = !peer.online || peer.rttMs == null ? 0 : peer.rttMs < 30 ? 4 : peer.rttMs < 60 ? 3 : peer.rttMs < 100 ? 2 : 1;
  return h("span", { class: `bars ${tone(peer)}` }, [1, 2, 3, 4].map((n) => h("i", { class: n <= on ? "on" : "" })));
}

function route(peer) {
  if (!peer.online) return h("span", { class: "route off" }, peer.route === "pending" ? "○ 等待中" : "○ 不在线");
  if (peer.route === "direct") return h("span", { class: "route direct", title: "两台电脑之间直接连通，游戏流量不经过第三方" }, "● 直连");
  return h("span", { class: "route relay", title: "打不通直连，经中继服务器转发（全程加密，中继看不到内容）" }, "◆ 经中继");
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

const pass = {
  build() {
    this.state = h("span", { class: "state" }, h("i"), h("span"));
    this.ip = h("div", { class: "ip" });
    this.who = h("b");
    this.host = h("div", { class: "host" });
    this.coord = h("div");
    this.online = h("div", { class: "me" }, h("div", { class: "k" }, "你的局域网地址"), this.ip, h("div", { class: "who" }, "你是 ", this.who), h("button", { class: "btn", glass: "clear", type: "button", onclick: () => copy(state.overview.me.ip, "地址") }, "复制地址"));
    this.offlineName = h("b");
    this.offline = h("div", { class: "me" }, h("div", { class: "k" }, "还没加入网络"), h("div", { class: "who" }, "你是 ", this.offlineName, "。贴一个网络码，就和朋友在同一个局域网里了。"));
    this.net = h("div", { class: "netbox" }, h("div", {}, "网络"), this.host, this.coord);

    this.glide = h("span", { class: "nav-glide", glass: "clear", "glass-glide": "", "aria-hidden": "true" });
    this.nav = h("nav", { class: "nav", "aria-label": "页面" }, this.glide);
    this.navKey = "";

    this.leave = h("button", { class: "btn", glass: "clear", type: "button", onclick: () => act("disconnect") }, "断开");
    this.el = h(
      "aside",
      { class: "pass", glass: "tinted", "glass-tint": "#1b2a66", "glass-corner-radius": "26" },
      h("div", { class: "brand" }, LOGO(), "Meshora", this.state),
      this.online,
      this.offline,
      this.net,
      this.nav,
      h("div", { class: "pass-foot" }, this.leave, h("div", { class: "ver" })),
    );
    this.version = this.el.querySelector(".ver");
    return this.el;
  },

  update(ov) {
    const [cls, text] = STATES[ov.phase] || STATES.idle;
    this.state.className = `state ${cls}`;
    this.state.lastChild.textContent = text;
    const connected = ov.phase === "connected";
    this.online.hidden = !connected;
    this.offline.hidden = connected;
    this.offlineName.textContent = ov.name || "这台电脑";
    if (connected) {
      this.ip.textContent = ov.me.ip;
      this.who.textContent = ov.name || "没起名字";
    }
    this.net.hidden = !ov.network || ov.phase === "idle";
    this.host.textContent = hostOf(ov.network);
    this.coord.textContent = connected ? `${ov.coordConnected ? "协调服务正常" : "协调服务重连中…"} · 网卡 ${ov.me.tun}` : ov.phase === "connecting" ? "正在连接" : "没连上";
    this.leave.hidden = !(connected || ov.phase === "connecting");
    this.version.textContent = `Meshora ${ov.version}`;
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
        this.glide,
        ...items.map(([page, label]) => {
          const count = h("span", { class: "n" });
          const button = h("button", { type: "button", onclick: () => go(page) }, label, count);
          this.buttons.set(page, { button, count });
          return button;
        }),
      );
    }
    for (const [page, , count] of items) this.buttons.get(page).count.textContent = count;
    if (!this.buttons.has(state.page)) state.page = "home";
    for (const [page, { button }] of this.buttons) {
      button.classList.toggle("on", page === state.page);
      button.setAttribute("aria-current", page === state.page ? "page" : "false");
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
  const svg = s("svg", { class: "graph", role: "img", "aria-label": "网状图：你和网里的每个人怎么连着" });
  let signature = "";
  let labels = new Map();

  function draw(ov) {
    const W = svg.clientWidth || 600;
    const H = svg.clientHeight || 400;
    const cx = W / 2;
    const cy = H / 2 + 8;
    const R = H * 0.31;
    const RX = Math.min(W * 0.36, R * 2);
    const peers = ov.peers.slice(0, 12);
    const relay = [cx, cy - R * 0.55];
    const pos = (i) => {
      const a = ((180 + (i * 360) / Math.max(peers.length, 1)) * Math.PI) / 180;
      return [cx + RX * Math.cos(a), cy + R * Math.sin(a)];
    };
    labels = new Map();
    const nodes = [
      s("defs", {}, s("radialGradient", { id: "me-glow" }, s("stop", { offset: 0, "stop-color": "#7a9cff", "stop-opacity": 0.55 }), s("stop", { offset: 1, "stop-color": "#7a9cff", "stop-opacity": 0 }))),
      s("ellipse", { cx, cy, rx: RX, ry: R, fill: "none", stroke: "rgba(255,255,255,.12)", "stroke-dasharray": "2 6" }),
      s("ellipse", { cx, cy, rx: RX * 0.55, ry: R * 0.55, fill: "none", stroke: "rgba(255,255,255,.1)", "stroke-dasharray": "2 6" }),
    ];
    if (peers.some((p) => p.online && p.route === "relay")) {
      nodes.push(
        s("rect", { x: relay[0] - 13, y: relay[1] - 13, width: 26, height: 26, rx: 8, fill: "rgba(183,155,255,.18)", stroke: RELAY_HEX, "stroke-width": 1.6 }),
        s("path", { d: `M${relay[0] - 6} ${relay[1] - 3}h12M${relay[0] - 6} ${relay[1] + 3}h12`, stroke: RELAY_HEX, "stroke-width": 1.6, "stroke-linecap": "round" }),
        s("text", { class: "node-sub", x: relay[0], y: relay[1] - 20, "text-anchor": "middle" }, "中继"),
      );
    }
    peers.forEach((peer, i) => {
      const [x, y] = pos(i);
      const viaRelay = peer.online && peer.route === "relay";
      const stroke = !peer.online ? "rgba(255,255,255,.3)" : viaRelay ? RELAY_HEX : TONE_HEX[tone(peer)];
      const d = viaRelay ? `M${cx} ${cy}L${relay[0]} ${relay[1]}L${x} ${y}` : `M${cx} ${cy}L${x} ${y}`;
      const id = `link-${i}`;
      nodes.push(s("path", { id, d, fill: "none", stroke, "stroke-width": 2.2, "stroke-linecap": "round", "stroke-dasharray": !peer.online ? "1 6" : viaRelay ? "6 5" : null }));
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
        const [mx, my] = viaRelay ? [(relay[0] + x) / 2, (relay[1] + y) / 2] : [(cx + x) / 2, (cy + y) / 2];
        const text = s("text", { class: "link-ms", x: mx, y: my + 4, "text-anchor": "middle", fill: TONE_HEX[tone(peer)] });
        nodes.push(s("rect", { x: mx - 24, y: my - 10, width: 48, height: 20, rx: 10, fill: "rgba(10,16,48,.6)", stroke }), text);
        labels.set(peer.id, text);
      }
      const label = s("text", { class: "node-label", x, y: y + 38, "text-anchor": "middle" }, nameOf(peer));
      nodes.push(
        s(
          "g",
          { class: "peer-node", opacity: peer.online ? 1 : 0.5, onclick: () => copy(peer.ip, "地址") },
          s("title", {}, `${nameOf(peer)} · ${peer.ip}（点一下复制地址）`),
          s("circle", { cx: x, cy: y, r: 21, fill: COLOR_HEX[Number(colorClass(peer.id).slice(1))], stroke: "rgba(255,255,255,.85)", "stroke-width": 2.5 }),
          s("text", { x, y: y + 5, "text-anchor": "middle", fill: "#fff", "font-weight": 700, "font-size": 14 }, initialOf(peer)),
          label,
          s("text", { class: "node-sub", x, y: y + 53, "text-anchor": "middle" }, peer.ip),
        ),
      );
    });
    nodes.push(
      s("circle", { cx, cy, r: 64, fill: "url(#me-glow)" }),
      s("circle", { cx, cy, r: 27, fill: "#3d6bff", stroke: "#fff", "stroke-width": 3 }),
      s("text", { x: cx, y: cy + 5, "text-anchor": "middle", fill: "#fff", "font-weight": 700, "font-size": 14 }, "我"),
      s("text", { class: "node-label", x: cx, y: cy + 46, "text-anchor": "middle" }, ov.name || "这台电脑"),
      s("text", { class: "node-sub", x: cx, y: cy + 61, "text-anchor": "middle" }, ov.me.ip),
    );
    svg.replaceChildren(...nodes);
  }

  return {
    el: svg,
    update(ov) {
      const size = `${svg.clientWidth}x${svg.clientHeight}`;
      const next = [size, ov.name, ...ov.peers.map((p) => `${p.id}|${p.name}|${p.online}|${p.route}|${tone(p)}`)].join(";");
      if (next !== signature) {
        signature = next;
        draw(ov);
      }
      for (const peer of ov.peers) {
        const text = labels.get(peer.id);
        if (text) text.textContent = peer.rttMs == null ? "—" : `${msText(peer.rttMs)} ms`;
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
  const el = h(
    "div",
    { class: "fcard", glass: "clear", "glass-corner-radius": "22", "glass-jelly": "", role: "button", tabindex: "0", title: "点一下复制地址" },
    h("div", { class: "ftop" }, avatar, h("div", { class: "who" }, name, ip), barsSlot),
    h("div", { class: "fmid" }, ms, routeSlot),
    sparkSlot,
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
      avatar.className = `ava ${colorClass(next.id)}`;
      avatar.textContent = initialOf(next);
      name.textContent = nameOf(next);
      ip.textContent = next.ip;
      barsSlot.replaceChildren(bars(next));
      ms.className = `fms t-${tone(next)}`;
      if (next.online && next.rttMs != null) ms.replaceChildren(msText(next.rttMs), h("small", {}, "ms"));
      else ms.replaceChildren("—");
      routeSlot.replaceChildren(route(next));
      sparkSlot.replaceChildren(spark(state.history.get(next.id) || [], 200, 30));
    },
  };
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

views.overview = {
  mount(ov) {
    this.down = h("b");
    this.up = h("b");
    this.paths = h("b");
    this.graph = meshGraph();
    this.empty = h("div", { class: "empty-map" }, "还没有别人。朋友凭网络码加入后，就会出现在这里。");
    this.grid = h("div", { class: "friends" });
    this.cards = new Map();
    this.hostChip = h("span", { class: "chip" });
    this.invite = h(
      "button",
      { class: "btn sm", glass: "tinted", "glass-tint": "#3d6bff", type: "button", onclick: () => state.overview.roster?.code && copy(state.overview.roster.code, "网络码") },
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
      h(
        "section",
        { class: "map", glass: "", "glass-corner-radius": "26" },
        h(
          "div",
          { class: "map-head" },
          h("div", {}, h("b", {}, "你的网"), h("span", {}, "点一个人复制他的地址"), this.extras),
          h("div", { class: "stats" }, h("div", {}, h("span", {}, "下行"), this.down), h("div", {}, h("span", {}, "上行"), this.up), h("div", {}, h("span", {}, "直连 / 中继"), this.paths)),
        ),
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
    const online = ov.peers.filter((p) => p.online);
    this.down.textContent = `${formatBytes(state.speed.rx)}/s`;
    this.up.textContent = `${formatBytes(state.speed.tx)}/s`;
    this.paths.textContent = `${online.filter((p) => p.route === "direct").length} / ${online.filter((p) => p.route === "relay").length}`;
    this.empty.hidden = ov.peers.length > 0;
    this.invite.hidden = !ov.roster?.code;
    const reach = ov.hosting?.reach;
    this.hostChip.hidden = !reach;
    this.hostChip.className = `chip ${reach === "open" ? "ok" : "warn"}`;
    this.hostChip.textContent = HOST_REACH[reach] || "";
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
    if (!row) rows.set(peer.id, (row = make(peer)));
    row.update(peer);
    return row.el;
  });
  for (const id of [...rows.keys()]) if (!seen.has(id)) rows.delete(id);
  const same = ordered.length === container.children.length && ordered.every((el, i) => container.children[i] === el);
  if (!same) container.replaceChildren(...ordered);
}

// ---------- 朋友 ----------

function friendRow(peer) {
  const avatar = h("div", { class: "ava sm" });
  const name = h("span");
  const cells = Array.from({ length: 7 }, () => h("td"));
  cells[3].className = "spark-cell";
  let current = peer;
  const el = h("tr", { title: "点一下复制地址", onclick: () => copy(current.ip, "地址") }, h("td", {}, h("div", { class: "who" }, avatar, name)), ...cells);
  return {
    el,
    update(next) {
      current = next;
      avatar.className = `ava sm ${colorClass(next.id)}`;
      avatar.textContent = initialOf(next);
      name.textContent = nameOf(next);
      cells[0].textContent = next.ip;
      cells[0].className = "mono";
      cells[1].replaceChildren(route(next));
      cells[2].className = `t-${tone(next)}`;
      cells[2].textContent = next.online && next.rttMs != null ? `${msText(next.rttMs)} ms` : "—";
      cells[3].replaceChildren(spark(state.history.get(next.id) || [], 140, 24));
      cells[4].textContent = next.online && next.jitterMs != null ? `${next.jitterMs} ms` : "—";
      cells[5].textContent = next.online && next.lossPercent != null ? `${next.lossPercent}%` : "—";
      cells[6].className = "t-none";
      cells[6].textContent = `${formatBytes(next.rx)} / ${formatBytes(next.tx)}`;
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
      h("section", { class: "card", glass: "", "glass-corner-radius": "22" }, h("div", { class: "card-head" }, h("b", {}, "朋友"), this.count), this.table, this.empty),
    );
    this.update(ov);
    return el;
  },
  update(ov) {
    const online = ov.peers.filter((p) => p.online).length;
    this.count.textContent = `${ov.peers.length} 人 · ${online} 在线 · 名字是对方自己起的，认人以地址为准`;
    this.table.hidden = !ov.peers.length;
    this.empty.hidden = ov.peers.length > 0;
    syncList(this.body, this.rows, ov.peers, friendRow);
  },
};

// ---------- 设置 ----------

function settingRow(title, desc, ...controls) {
  return h("div", { class: "set" }, h("div", { class: "t" }, h("b", {}, title), desc ? h("span", {}, desc) : null), ...controls);
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
    this.name = h("input", { class: "field", maxlength: "32", spellcheck: "false", "aria-label": "你的名字" });
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

    this.code = h("span", { class: "code" });
    this.codeNote = h("span");
    this.network = h("div", {}, h("div", { class: "group" }, "网络"), settingRow("网络码", null, h("button", { class: "btn sm", glass: "clear", type: "button", onclick: () => copy(state.overview.network, "网络码") }, "复制"), h("button", { class: "btn sm danger", glass: "clear", type: "button", onclick: () => act("forget") }, "离开这个网络")));
    this.network.querySelector(".t").append(this.code, this.codeNote);

    this.serverList = h("div");
    const newServer = h("input", { class: "field mono-field", spellcheck: "false", placeholder: "公钥@地址:端口", "aria-label": "服务器地址" });
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
    this.serverGroup = h(
      "div",
      {},
      h("div", { class: "group" }, "我的服务器"),
      this.serverList,
      settingRow("添加一台", "自己架的 meshora-coord（--hub）：公钥@地址:端口。建网络时可以选它", newServer, h("button", { class: "btn sm", glass: "clear", type: "button", onclick: addServer }, "添加")),
    );

    this.logs = h("pre", { class: "logs" }, "…");
    this.logs.hidden = true;
    let timer = 0;
    const loadLogs = async () => {
      const lines = await invoke("logs");
      const atBottom = this.logs.scrollTop + this.logs.clientHeight >= this.logs.scrollHeight - 8;
      this.logs.textContent = lines.length ? lines.join("\n") : "还没有日志";
      if (atBottom) this.logs.scrollTop = this.logs.scrollHeight;
    };
    const showLogs = h(
      "button",
      {
        class: "btn sm",
        glass: "clear",
        type: "button",
        onclick: () => {
          const show = this.logs.hidden;
          this.logs.hidden = !show;
          showLogs.textContent = show ? "收起" : "查看";
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

    const el = h(
      "div",
      { class: "view" },
      h(
        "section",
        { class: "card", glass: "", "glass-corner-radius": "22" },
        h("div", { class: "group" }, "你"),
        settingRow("你的名字", "网里的人看到的就是它。改了马上生效", this.name),
        settingRow("你的 ID", null, h("button", { class: "btn sm", glass: "clear", type: "button", onclick: () => copy(state.overview.id, "ID") }, "复制")),
        h("div", { class: "group" }, "联机"),
        settingRow("让游戏的广播走 Meshora（推荐）", "朋友的房间才会出现在局域网列表里。改了会重新连接", this.broadcast),
        settingRow("把 Meshora 设为专用网络", "朋友连不进你开的房间时再打开。代价：你对专用网络共享的东西（比如共享文件夹），网里的人也能访问", this.private),
        settingRow("打开时自动连接", "启动客户端时自动连上次的网络", this.auto),
        this.network,
        this.serverGroup,
        h("div", { class: "group" }, "排查"),
        settingRow("日志", "出问题时复制下来，发给帮你排查的人。里面有 IP 地址，没有密钥", showLogs, h("button", { class: "btn sm", glass: "clear", type: "button", onclick: async () => copy((await invoke("logs")).join("\n"), "日志") }, "复制")),
        this.logs,
      ),
    );
    el.querySelectorAll(".set .t")[1].append(h("span", { class: "code" }, ov.id));
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
          settingRow(hostOf(server), null, h("button", { class: "btn sm danger", glass: "clear", type: "button", onclick: async () => { await invoke("remove_server", { code: server }); refresh(); } }, "忘掉")),
        ),
      );
    }
    this.network.hidden = !ov.network;
    this.code.textContent = ov.network ? ov.network.replace(/#.*$/, "#••••••") : "";
    this.codeNote.textContent = ov.network && ov.network.includes("#") ? "带着邀请码：发给谁，谁就能加入这个网络。只发给要一起玩的人" : "";
  },
};

// ---------- 加入、连接中、出错 ----------

// ---------- 确认框 ----------

/** 问一句"确定吗"。返回用户点没点确定 */
function confirmBox(title, text, okLabel) {
  return new Promise((resolve) => {
    const close = (answer) => {
      scrim.remove();
      document.removeEventListener("keydown", onKey);
      resolve(answer);
    };
    const onKey = (event) => event.key === "Escape" && close(false);
    const ok = h("button", { class: "btn wide danger", glass: "clear", type: "button", onclick: () => close(true) }, okLabel);
    const scrim = h(
      "div",
      { class: "modal-scrim", onclick: (event) => event.target === scrim && close(false) },
      h(
        "section",
        { class: "dialog narrow", glass: "", "glass-corner-radius": "26", role: "alertdialog", "aria-modal": "true" },
        h("h2", {}, title),
        h("p", {}, text),
        h("div", { class: "row center-row" }, h("button", { class: "btn", glass: "clear", type: "button", onclick: () => close(false) }, "取消"), ok),
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
    const el = h("div", { class: "view center" }, h("section", { class: "dialog welcome-card", glass: "", "glass-corner-radius": "28" }, this.body));
    this.draw(ov);
    return el;
  },
  draw(ov) {
    if (this.step === 0) {
      const name = h("input", { class: "field wide-field", maxlength: "32", spellcheck: "false", "aria-label": "你的名字" });
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
        h("div", { class: "row center-row" }, h("button", { class: "btn wide", glass: "tinted", "glass-tint": "#3d6bff", type: "button", onclick: next }, "下一步")),
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
  const pick = (value) => {
    buttons.forEach((button, i) => button.setAttribute("aria-checked", String(options[i].value === value)));
    onPick(value);
  };
  return h("div", { class: "choices", role: "radiogroup" }, buttons);
}

views.start = {
  mount(ov) {
    // ---- 建网络 ----
    this.netName = h("input", { class: "field wide-field", maxlength: "32", spellcheck: "false", "aria-label": "网络名" });
    this.netName.value = `${ov.name || "我"}的网络`;
    this.where = ov.officialServer ? "official" : "thisPc";
    this.server = h("input", { class: "field wide-field mono-field", spellcheck: "false", placeholder: "公钥@地址:端口", "aria-label": "服务器地址" });
    this.server.value = ov.servers[0] || "";
    const serverRow = h("div", { class: "server-row" }, this.server);
    serverRow.hidden = this.where !== "server";
    const createError = h("div", { class: "field-error", role: "alert" });
    const createButton = h("button", { class: "btn wide", glass: "tinted", "glass-tint": "#3d6bff", type: "button" }, "建网络");
    const options = [
      { value: "official", label: "官方服务器", hint: ov.officialServer ? "最省事：朋友在哪都能连进来" : "还没上线", disabled: !ov.officialServer },
      { value: "thisPc", label: "本机当主机", hint: "不用服务器。路由器要支持 UPnP，这台电脑开着网络才在" },
      { value: "server", label: "我的服务器", hint: "自己架的 meshora-coord（--hub）" },
    ];
    const picker = choices(options, this.where, (value) => {
      this.where = value;
      serverRow.hidden = value !== "server";
      createError.textContent = "";
    });
    createButton.addEventListener("click", async () => {
      createError.textContent = "";
      const name = this.netName.value.trim();
      if (!name) {
        createError.textContent = "给网络起个名字";
        return;
      }
      let at = { kind: this.where };
      if (this.where === "server") {
        try {
          at = { kind: "server", code: await invoke("add_server", { code: this.server.value }) };
        } catch (err) {
          createError.textContent = String(err);
          return;
        }
      }
      createButton.disabled = true;
      try {
        await invoke("create", { at, name });
        state.page = "home";
        refresh();
      } catch (err) {
        createError.textContent = String(err);
      } finally {
        createButton.disabled = false;
      }
    });
    this.create = h(
      "section",
      { class: "dialog", glass: "", "glass-corner-radius": "26" },
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
    const area = h("textarea", { placeholder: "公钥@地址:端口/网络ID#邀请码", spellcheck: "false", "aria-label": "网络码" });
    const error = h("div", { class: "field-error", role: "alert" });
    const button = h("button", { class: "btn wide", glass: "tinted", "glass-tint": "#3d6bff", type: "button" }, "加入");
    const submit = async () => {
      const code = area.value.trim();
      if (!code) {
        error.textContent = "先粘贴一个网络码";
        area.focus();
        return;
      }
      error.textContent = "";
      button.disabled = true;
      try {
        await invoke("connect", { code });
        refresh();
      } catch (err) {
        error.textContent = String(err);
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
      h("button", { class: "btn sm", glass: "clear", type: "button", onclick: () => act("connect", {}) }, "重新连接"),
      h("button", { class: "btn sm danger", glass: "clear", type: "button", onclick: () => act("forget") }, "忘掉"),
    );
    this.join = h(
      "section",
      { class: "dialog", glass: "", "glass-corner-radius": "26" },
      h("h2", {}, "加入朋友的网络"),
      h("p", {}, "把朋友发给你的网络码贴进来。"),
      area,
      error,
      h("div", { class: "row" }, h("span", { class: "hint" }, "网络码里没有邀请码？", h("button", { class: "link", type: "button", onclick: () => copy(state.overview.id, "ID") }, "复制你的 ID"), " 发给建网络的人"), button),
      this.saved,
    );
    const el = h("div", { class: "view center" }, h("div", { class: "start" }, this.create, this.join));
    this.update(ov);
    // 引导里选了哪个，就先把光标放在哪
    const tab = state.startTab;
    state.startTab = null;
    setTimeout(() => (tab === "create" ? this.netName : area).focus(), 50);
    return el;
  },
  update(ov) {
    this.saved.hidden = !ov.network;
    this.savedHost.textContent = hostOf(ov.network);
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
    this.title = h("input", { class: "field", maxlength: "32", spellcheck: "false", "aria-label": "网络名" });
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
      h(
        "section",
        { class: "card", glass: "", "glass-corner-radius": "22" },
        h("div", { class: "group" }, "网络"),
        settingRow("网络名", "网里的人在你的网络码旁边看到的名字", this.title),
        h("div", { class: "group" }, "邀请朋友"),
        settingRow(
          "网络码",
          "发给谁，谁就能加入。泄露了就换一个：旧的立刻作废，已经在网里的人不受影响",
          h("button", { class: "btn sm", glass: "clear", type: "button", onclick: copyCode }, "复制"),
          h(
            "button",
            {
              class: "btn sm",
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
          h("button", { class: "btn sm", glass: "clear", type: "button", onclick: () => admin({ kind: "newInvite", uses: 1, hours: null }, (code) => code && copy(code, "一次性网络码")) }, "一次性"),
          h("button", { class: "btn sm", glass: "clear", type: "button", onclick: () => admin({ kind: "newInvite", uses: null, hours: 24 }, (code) => code && copy(code, "24 小时网络码")) }, "24 小时"),
        ),
        h("div", { class: "group" }, "成员 ", this.memberCount),
        this.members,
        h("div", { class: "group" }, "解散"),
        settingRow(
          "解散这个网络",
          "所有人立刻断开，网络码作废，网络删掉。不能撤销",
          h(
            "button",
            {
              class: "btn sm danger",
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
    el.querySelectorAll(".set .t")[1].append(this.code);
    this.update(ov);
    return el;
  },
  update(ov) {
    const roster = ov.roster;
    if (!roster) return;
    if (document.activeElement !== this.title && this.title.value !== roster.name) this.title.value = roster.name;
    this.code.textContent = roster.code ? roster.code.replace(/#.*$/, "#••••••") : "";
    const inviteKey = roster.invites.map((i) => i.invite + i.usesLeft + i.expires).join();
    if (inviteKey !== this.inviteKey) {
      this.inviteKey = inviteKey;
      this.invites.replaceChildren(
        ...roster.invites.map((invite) =>
          h(
            "div",
            { class: "set" },
            h("div", { class: "t" }, h("b", {}, inviteText(invite)), h("span", { class: "code" }, `#${invite.invite.slice(0, 6)}…`)),
            h("button", { class: "btn sm", glass: "clear", type: "button", onclick: () => copy(invite.code, "网络码") }, "复制"),
            h("button", { class: "btn sm danger", glass: "clear", type: "button", onclick: () => admin({ kind: "revokeInvite", invite: invite.invite }, () => toast("作废了")) }, "作废"),
          ),
        ),
      );
    }
    this.memberCount.textContent = `${roster.members.length} 人 · ${roster.members.filter((m) => m.online).length} 在线`;
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
        { class: "dialog narrow", glass: "", "glass-corner-radius": "26" },
        h("div", { class: "pulse", role: "progressbar", "aria-label": "正在连接" }, h("i"), h("i"), h("b")),
        h("h2", {}, "正在连接"),
        this.host,
        h("button", { class: "btn", glass: "clear", type: "button", onclick: () => act("disconnect") }, "取消"),
      ),
    );
    this.update(ov);
    return el;
  },
  update(ov) {
    this.host.textContent = hostOf(ov.network);
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
  if (kind === "tun" && /wintun\.dll/i.test(message)) return "tunDriver";
  if (kind === "tun" && /拒绝访问|access is denied|\(os error 5\)/i.test(message)) return "tunAdmin";
  return kind;
}

views.failed = {
  mount(ov) {
    this.title = h("h2", { class: "err" });
    this.hint = h("p");
    this.detail = h("div", { class: "detail" });
    this.copyId = h("button", { class: "link", type: "button", onclick: () => copy(state.overview.id, "ID") }, "复制你的 ID");
    const el = h(
      "div",
      { class: "view center" },
      h(
        "section",
        { class: "dialog", glass: "", "glass-corner-radius": "26" },
        this.title,
        this.hint,
        this.detail,
        h(
          "div",
          { class: "row" },
          h("button", { class: "btn wide", glass: "tinted", "glass-tint": "#3d6bff", type: "button", onclick: () => act("connect", {}) }, "重试"),
          h("button", { class: "btn", glass: "clear", type: "button", onclick: () => act("forget") }, "换一个网络"),
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
    this.title.textContent = info.title;
    this.hint.textContent = info.hint;
    this.detail.textContent = ov.error.message;
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
  document.getElementById("app").replaceChildren(pass.build(), h("div", { class: "column" }, titlebar(), main));
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

function render(ov) {
  state.overview = ov;
  document.getElementById("app").classList.toggle("solo", !ov.onboarded);
  pass.update(ov);
  const name = viewFor(ov);
  if (name !== state.view) {
    state.current?.unmount?.();
    state.view = name;
    state.current = Object.create(views[name]);
    main.replaceChildren(state.current.mount(ov));
    main.scrollTop = 0;
  } else {
    state.current.update(ov);
  }
}

function go(page) {
  state.page = page;
  if (state.overview) render(state.overview);
}

async function refresh() {
  try {
    const ov = await invoke("overview");
    track(ov);
    render(ov);
  } catch (err) {
    console.error(err);
  }
}

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
refresh();
setInterval(refresh, POLL_MS);
