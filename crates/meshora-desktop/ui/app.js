// Meshora 桌面客户端的界面。所有状态来自 Rust 那一侧的 overview 命令，每秒拉一次；
// 按钮只是把意图转过去。数据一律用 textContent 放进页面，不拼 HTML。
"use strict";

const invoke = (command, args) => window.__TAURI__.core.invoke(command, args);

const POLL_MS = 1000;

// ---------- 小工具 ----------

function h(tag, props, ...children) {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(props || {})) {
    if (value === undefined || value === null || value === false) continue;
    if (key === "class") el.className = value;
    else if (key.startsWith("on")) el.addEventListener(key.slice(2), value);
    else if (key === "vars") for (const [name, v] of Object.entries(value)) el.style.setProperty(name, v);
    else el.setAttribute(key, value === true ? "" : value);
  }
  for (const child of children.flat()) {
    if (child === null || child === undefined || child === false) continue;
    el.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return el;
}

function svg(markup) {
  const template = document.createElement("template");
  template.innerHTML = markup.trim(); // 只用于下面写死的图标，不含任何外部数据
  return template.content.firstChild;
}

const ICONS = {
  logo: `<svg viewBox="0 0 64 64" aria-hidden="true"><defs><linearGradient id="lg" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#3c6bb5"/><stop offset="1" stop-color="#122b55"/></linearGradient></defs><rect x="2.5" y="2.5" width="59" height="59" rx="14" fill="url(#lg)"/><g stroke="rgba(255,255,255,.6)" stroke-width="2.9" stroke-linecap="round"><line x1="19.2" y1="21.1" x2="45.4" y2="18.6"/><line x1="19.2" y1="21.1" x2="21.1" y2="45.4"/><line x1="19.2" y1="21.1" x2="44.2" y2="43.5"/><line x1="45.4" y1="18.6" x2="21.1" y2="45.4"/><line x1="45.4" y1="18.6" x2="44.2" y2="43.5"/><line x1="21.1" y1="45.4" x2="44.2" y2="43.5"/></g><circle cx="19.2" cy="21.1" r="5.4" fill="#fff"/><circle cx="45.4" cy="18.6" r="5.4" fill="#fff"/><circle cx="21.1" cy="45.4" r="5.4" fill="#fff"/><circle cx="44.2" cy="43.5" r="5.4" fill="#9fd0ff"/></svg>`,
  gear: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/></svg>`,
  alert: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="10"/><line x1="12" y1="7.5" x2="12" y2="13"/><circle cx="12" cy="16.5" r=".6" fill="currentColor"/></svg>`,
};

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

function copyButton(getText, what) {
  return h("button", { class: "copy", type: "button", onclick: () => copy(getText(), what) }, "复制");
}

function formatBytes(n) {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = n / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${units[unit]}`;
}

// 同一个 ID 永远是同一个颜色，好认人
function avatarColor(id) {
  let hash = 0;
  for (const ch of id) hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
  return `hsl(${hash % 360} 55% 48%)`;
}

// 网络码里给人看的部分：地址。邀请码是秘密，不往界面上摆
function hostOf(code) {
  const text = (code || "").split("#")[0];
  const at = text.lastIndexOf("@");
  return at < 0 ? text : text.slice(at + 1);
}

// ---------- 各个画面 ----------
// 每个画面有 mount(ov) 建出元素，update(ov) 就地更新。同一个画面不重建，输入框里打到一半的字不会丢

const views = {};

views.join = {
  mount(ov) {
    const area = h("textarea", {
      placeholder: "粘贴网络码，形如  公钥@地址:端口#邀请码",
      spellcheck: "false",
      "aria-label": "网络码",
    });
    const error = h("div", { class: "field-error", role: "alert" });
    const button = h("button", { class: "primary block", type: "button" }, "加入");
    const title = h("h2", {}, "加入一个联机网络");
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

    const saved = h("div");
    const el = h(
      "div",
      { class: "view" },
      saved,
      h(
        "section",
        { class: "card stack" },
        h("div", {}, title, h("p", {}, "建网络的人会给你一个网络码。贴进来，就和网里的朋友在同一个局域网了。")),
        area,
        error,
        button,
      ),
      idCard(ov),
    );
    this.saved = saved;
    this.title = title;
    this.button = button;
    this.update(ov);
    if (!ov.network) setTimeout(() => area.focus(), 50);
    return el;
  },
  update(ov) {
    // 断开之后回到这里：保存着的网络一键重连
    const key = ov.network || "";
    if (this.savedKey === key) return;
    this.savedKey = key;
    // 存着上次的网络时，"连接"是主角，这里退成次要的
    this.title.textContent = ov.network ? "加入另一个网络" : "加入一个联机网络";
    this.button.className = `${ov.network ? "secondary" : "primary"} block`;
    this.saved.replaceChildren();
    if (!ov.network) return;
    this.saved.append(
      h(
        "section",
        { class: "card stack" },
        h("div", {}, h("div", { class: "label" }, "上次的网络"), h("div", { class: "mono" }, hostOf(ov.network))),
        h(
          "div",
          { class: "row" },
          h("button", { class: "primary", type: "button", onclick: () => act("connect", {}) }, "连接"),
          h("span", { class: "spacer" }),
          h("button", { class: "ghost danger", type: "button", onclick: () => act("forget") }, "忘掉它"),
        ),
      ),
    );
  },
};

function idCard(ov) {
  return h(
    "section",
    { class: "card stack" },
    h(
      "div",
      {},
      h("h2", {}, "你的 ID"),
      h("p", {}, "网络码里带着邀请码的话用不着它。不带的话，把它发给建网络的人，由对方加进名单。它只代表这台电脑，不是密码，可以放心发。"),
    ),
    h("div", { class: "id-box" }, h("span", { class: "mono" }, ov.id), copyButton(() => ov.id, "ID")),
  );
}

views.connecting = {
  mount(ov) {
    this.host = h("p", {});
    const el = h(
      "div",
      { class: "view" },
      h(
        "section",
        { class: "card center" },
        h("div", { class: "spinner", role: "progressbar", "aria-label": "正在连接" }),
        h("h2", {}, "正在连接"),
        this.host,
        h("button", { class: "secondary", type: "button", onclick: () => act("disconnect") }, "取消"),
      ),
    );
    this.update(ov);
    return el;
  },
  update(ov) {
    this.host.textContent = hostOf(ov.network);
  },
};

views.connected = {
  mount(ov) {
    this.address = h("div", { class: "address" });
    this.chips = h("div", { class: "chips" });
    this.count = h("span", { class: "label" });
    this.list = h("div", { class: "peers" });
    this.rows = new Map();
    const el = h(
      "div",
      { class: "view" },
      h(
        "section",
        { class: "card hero" },
        h("div", { class: "label" }, "你的局域网地址"),
        h("div", { class: "row" }, this.address, copyButton(() => state.overview.me.ip, "地址")),
        h("p", {}, "朋友在游戏里输这个地址就能连到你。房间开在局域网模式下，朋友那边的房间列表里应该也能直接看到。"),
        this.chips,
      ),
      h("div", { class: "section-title" }, h("h3", {}, "网里的人"), this.count),
      h("section", { class: "card peers-card" }, this.list),
      h(
        "button",
        { class: "secondary block", type: "button", onclick: () => act("disconnect") },
        "断开",
      ),
      h("p", { class: "footnote" }, "关掉窗口不会断线，Meshora 会留在托盘里。要彻底退出，右键托盘图标。"),
    );
    this.update(ov);
    return el;
  },
  update(ov) {
    this.address.textContent = ov.me.ip;
    this.chips.replaceChildren(
      h("span", { class: `chip ${ov.coordConnected ? "ok" : "warn"}` }, ov.coordConnected ? "协调服务已连接" : "协调服务重连中…"),
      h("span", { class: "chip" }, `网卡 ${ov.me.tun}`),
      h("span", { class: "chip" }, hostOf(ov.network)),
    );
    const online = ov.peers.filter((peer) => peer.online).length;
    this.count.textContent = ov.peers.length ? `${online} / ${ov.peers.length} 在线` : "";

    if (!ov.peers.length) {
      this.rows.clear();
      this.list.replaceChildren(h("div", { class: "empty" }, "还没有别人。朋友装好 Meshora、被加进名单后，就会出现在这里。"));
      return;
    }
    const seen = new Set();
    const ordered = [];
    for (const peer of ov.peers) {
      seen.add(peer.id);
      let row = this.rows.get(peer.id);
      if (!row) {
        row = peerRow(peer);
        this.rows.set(peer.id, row);
      }
      row.update(peer);
      ordered.push(row.el);
    }
    for (const id of [...this.rows.keys()]) if (!seen.has(id)) this.rows.delete(id);
    this.list.replaceChildren(...ordered);
  },
};

function peerRow(peer) {
  const presence = h("span", { class: "presence" });
  const ip = h("div", { class: "peer-ip", title: "点一下复制地址" });
  const sub = h("div", { class: "peer-sub" });
  const badge = h("span", { class: "badge" });
  const el = h(
    "div",
    { class: "peer" },
    h("div", { class: "avatar", vars: { "--avatar": avatarColor(peer.id) } }, peer.ip.split(".").pop(), presence),
    h("div", { class: "peer-main" }, ip, sub),
    badge,
  );
  ip.addEventListener("click", () => copy(ip.textContent, "地址"));
  return {
    el,
    update(peer) {
      ip.textContent = peer.ip;
      presence.classList.toggle("on", peer.online);
      presence.title = peer.online ? "在线" : "不在线";
      sub.textContent = `${peer.id.slice(0, 8)}… · ↓ ${formatBytes(peer.rx)} · ↑ ${formatBytes(peer.tx)}`;
      sub.title = peer.id;
      let text;
      let kind;
      if (!peer.online) {
        text = peer.route === "pending" ? "等待中" : "连接中";
        kind = "";
      } else if (peer.route === "direct") {
        text = peer.rttMs === null ? "直连" : `直连 · ${peer.rttMs < 1 ? "<1" : peer.rttMs} ms`;
        if (peer.lossPercent >= 1) text += ` · 丢包 ${peer.lossPercent}%`;
        kind = "direct";
      } else {
        text = "经中继";
        kind = "relay";
      }
      badge.textContent = text;
      badge.className = `badge ${kind}`;
      badge.title =
        kind === "direct"
          ? "两台电脑之间直接连通，游戏流量不经过第三方" +
            (peer.jitterMs === null || peer.jitterMs === undefined ? "" : `。延迟抖动约 ${peer.jitterMs} ms`)
          : kind === "relay"
            ? "打不通直连，经中继服务器转发（全程加密，中继看不到内容）"
            : "还在建立连接";
    },
  };
}

const FAILURES = {
  rejected: {
    title: "还没被加进这个网络",
    hint: "这个网络码里没有邀请码，网络只认名单。向建网络的人要一个带邀请码的网络码；或者把下面的 ID 发给对方，加进名单后再点「重试」。",
    showId: true,
  },
  // 下面三种是 rejected 的细分，按协调服务给的原因认
  rejectedInvite: {
    title: "邀请码不对",
    hint: "邀请码可能已经换过了。向建网络的人要一个最新的网络码，点「换一个网络」贴进去。",
  },
  rejectedClosed: {
    title: "这个网络不接受邀请",
    hint: "网络码里带着邀请码，但这个网络只认名单。把下面的 ID 发给建网络的人，加进名单后再点「重试」。",
    showId: true,
  },
  rejectedKicked: {
    title: "你已被移出这个网络",
    hint: "建网络的人把你移出了。想回来的话，向对方要一个新的网络码。",
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

views.failed = {
  mount(ov) {
    this.title = h("h2", {});
    this.hint = h("p", {});
    this.detail = h("div", { class: "detail" });
    this.extra = h("div");
    const el = h(
      "div",
      { class: "view" },
      h(
        "section",
        { class: "card error-card" },
        h("div", { class: "error-title" }, svg(ICONS.alert), this.title),
        this.hint,
        this.detail,
        h(
          "div",
          { class: "actions" },
          h("button", { class: "primary", type: "button", onclick: () => act("connect", {}) }, "重试"),
          h("button", { class: "secondary", type: "button", onclick: () => act("forget") }, "换一个网络"),
        ),
      ),
      this.extra,
    );
    this.update(ov);
    return el;
  },
  update(ov) {
    let kind = ov.error.kind;
    if (kind === "rejected" && ov.error.message.includes("移出")) kind = "rejectedKicked";
    else if (kind === "rejected" && ov.error.message.includes("邀请码不对")) kind = "rejectedInvite";
    else if (kind === "rejected" && ov.error.message.includes("不接受凭邀请码")) kind = "rejectedClosed";
    else if (kind === "rejected" && ov.error.message.includes("网络已满")) kind = "rejectedFull";
    if (kind === "tun" && /wintun\.dll/i.test(ov.error.message)) kind = "tunDriver";
    else if (kind === "tun" && /拒绝访问|access is denied|\(os error 5\)/i.test(ov.error.message)) kind = "tunAdmin";
    const info = FAILURES[kind] || FAILURES.other;
    this.title.textContent = info.title;
    this.hint.textContent = info.hint;
    this.detail.textContent = ov.error.message;
    if (info.showId && !this.extra.childElementCount) this.extra.append(idCard(ov));
    if (!info.showId) this.extra.replaceChildren();
  },
};

// ---------- 设置面板 ----------

let sheet = null;

function openSettings() {
  if (sheet) return;
  const ov = state.overview;
  const toggle = (checked, onChange) => {
    const el = h("button", { class: "switch", type: "button", role: "switch", "aria-checked": String(checked) });
    el.addEventListener("click", async () => {
      const next = el.getAttribute("aria-checked") !== "true";
      el.setAttribute("aria-checked", String(next));
      el.disabled = true;
      try {
        await onChange(next);
      } catch (err) {
        el.setAttribute("aria-checked", String(!next));
        toast(String(err));
      } finally {
        el.disabled = false;
        refresh();
      }
    });
    return el;
  };

  const logs = h("pre", { class: "logs" }, "…");
  let logTimer = 0;
  const loadLogs = async () => {
    const lines = await invoke("logs");
    const atBottom = logs.scrollTop + logs.clientHeight >= logs.scrollHeight - 8;
    logs.textContent = lines.length ? lines.join("\n") : "还没有日志";
    if (atBottom) logs.scrollTop = logs.scrollHeight;
  };
  const logsBlock = h(
    "div",
    { class: "stack", hidden: true },
    logs,
    h("button", { class: "secondary", type: "button", onclick: () => copy(logs.textContent, "日志") }, "复制全部日志"),
  );

  const scrim = h("div", { class: "sheet-scrim" });
  const panel = h(
    "div",
    { class: "sheet", role: "dialog", "aria-modal": "true", "aria-label": "设置" },
    h("div", { class: "grabber" }),
    h("h2", {}, "设置"),
    h(
      "div",
      { class: "setting" },
      h(
        "div",
        { class: "setting-text" },
        h("strong", {}, "让游戏的广播走 Meshora（推荐）"),
        h(
          "span",
          {},
          "局域网游戏靠广播找房间，而 Windows 只把广播交给一张网卡。打开后交给 Meshora 的虚拟网卡，朋友的房间才会出现在列表里。改了会重新连接。",
        ),
      ),
      toggle(ov.preferBroadcast, (on) => invoke("set_prefer_broadcast", { on })),
    ),
    h(
      "div",
      { class: "setting" },
      h("div", { class: "setting-text" }, h("strong", {}, "打开时自动连接"), h("span", {}, "启动客户端时自动连上次的网络。")),
      toggle(ov.autoConnect, (on) => invoke("set_auto_connect", { on })),
    ),
    ov.network
      ? h(
          "div",
          { class: "setting" },
          h(
            "div",
            { class: "setting-text" },
            h("strong", {}, "网络码"),
            h("span", { class: "mono" }, ov.network),
            ov.network.includes("#")
              ? h("span", {}, "带着邀请码：发给谁，谁就能加入这个网络。只发给要一起玩的人。")
              : null,
            h(
              "div",
              { class: "row" },
              h("button", { class: "ghost", type: "button", onclick: () => copy(ov.network, "网络码") }, "复制"),
              h(
                "button",
                {
                  class: "ghost danger",
                  type: "button",
                  onclick: async () => {
                    closeSettings();
                    await act("forget");
                  },
                },
                "离开这个网络",
              ),
            ),
          ),
        )
      : null,
    h(
      "div",
      { class: "setting" },
      h("div", { class: "setting-text" }, h("strong", {}, "你的 ID"), h("span", { class: "mono" }, ov.id)),
      copyButton(() => ov.id, "ID"),
    ),
    h(
      "div",
      { class: "setting" },
      h(
        "div",
        { class: "setting-text" },
        h("strong", {}, "日志"),
        h("span", {}, "出问题时复制下来，发给帮你排查的人。里面有 IP 地址，没有密钥。"),
      ),
      h(
        "button",
        {
          class: "copy",
          type: "button",
          onclick: (event) => {
            const show = logsBlock.hidden;
            logsBlock.hidden = !show;
            event.currentTarget.textContent = show ? "收起" : "查看";
            clearInterval(logTimer);
            if (show) {
              loadLogs();
              logTimer = setInterval(loadLogs, POLL_MS);
            }
          },
        },
        "查看",
      ),
    ),
    logsBlock,
    h("div", { class: "version" }, `Meshora ${ov.version} · Pre-alpha`),
  );
  scrim.addEventListener("click", closeSettings);
  document.body.append(scrim, panel);
  sheet = { scrim, panel, stop: () => clearInterval(logTimer) };
}

function closeSettings() {
  if (!sheet) return;
  const { scrim, panel, stop } = sheet;
  sheet = null;
  stop();
  scrim.classList.add("closing");
  panel.classList.add("closing");
  setTimeout(() => {
    scrim.remove();
    panel.remove();
  }, 230);
}

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") closeSettings();
});

// ---------- 主循环 ----------

const state = { overview: null, view: null, current: null };

const PILLS = {
  idle: ["", "未连接"],
  connecting: ["busy", "连接中"],
  connected: ["ok", "已连接"],
  failed: ["bad", "未连上"],
};

let pill;
let main;

function shell() {
  pill = h("span", { class: "pill" }, h("span", { class: "dot" }), h("span", {}));
  main = h("main");
  const app = document.getElementById("app");
  app.replaceChildren(
    h(
      "header",
      { class: "topbar" },
      h("div", { class: "brand" }, svg(ICONS.logo), "Meshora"),
      h("span", { class: "spacer" }),
      pill,
      h("button", { class: "icon-button", type: "button", title: "设置", "aria-label": "设置", onclick: openSettings }, svg(ICONS.gear)),
    ),
    main,
  );
}

function viewFor(ov) {
  if (ov.phase === "connected") return "connected";
  if (ov.phase === "connecting") return "connecting";
  if (ov.phase === "failed") return "failed";
  return "join";
}

function render(ov) {
  state.overview = ov;
  const [kind, text] = PILLS[ov.phase] || PILLS.idle;
  pill.className = `pill ${kind}`;
  pill.lastChild.textContent = text;

  const name = viewFor(ov);
  if (name !== state.view) {
    state.view = name;
    state.current = Object.create(views[name]);
    main.replaceChildren(state.current.mount(ov));
  } else {
    state.current.update(ov);
  }
}

async function refresh() {
  try {
    render(await invoke("overview"));
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
  refresh();
}

shell();
refresh();
setInterval(refresh, POLL_MS);
