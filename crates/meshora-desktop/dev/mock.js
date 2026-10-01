// 在普通浏览器里看界面用的假后端：不需要管理员权限、不建网卡。
// 用法：在 crates/meshora-desktop 下起一个静态服务器，打开 dev/index.html?s=<场景>
// 场景：join、saved、connecting、connected、empty、rejected、invite、tun、unreachable
"use strict";

(() => {
  const scenario = new URLSearchParams(location.search).get("s") || "join";
  const id = "mTe0q8vN3kRZp1u5yXcW7bLdF2gH9jK4sA6eQoIiUtY=";
  const code = "Bq7Zt4mN0xR2c8vL5kP1wY9sD3fG6hJ8aE2uQ4iO7tU=@play.example.com:7443#3q2-7wEYkQ6n0Cf8Hs5VYA";
  const peers = [
    { id: "Kx81ZrT0pQv3Yb7Nc2Lw5Df8Gh1Jk4Ms6Aa9Ee0Ii2U=", name: "小明", ip: "100.64.0.1", route: "direct", rttMs: 14, jitterMs: 3, lossPercent: 2, online: true, rx: 18_734_112, tx: 9_201_554 },
    { id: "Pm42VcX9sB1nQ7rT3yH5jK8lZ0wE2dF4gA6uI9oO1eU=", name: "老王的笔记本", ip: "100.64.0.2", route: "relay", rttMs: 48, jitterMs: 4, online: true, rx: 2_048_331, tx: 1_530_227 },
    { id: "Wq7Hd3Fk9Lz1Xc5Vb8Nm2As4Df6Gh0Jk3Lq5We7Rt9Y=", name: "", ip: "100.64.0.4", route: "pending", rttMs: null, jitterMs: null, online: false, rx: 0, tx: 0 },
  ];
  const base = {
    version: "0.0.0",
    id,
    name: "阿杰的台式机",
    network: null,
    preferBroadcast: true,
    autoConnect: true,
    privateNetwork: false,
    phase: "idle",
    error: null,
    me: null,
    coordConnected: false,
    peers: [],
  };
  const me = { ip: "100.64.0.3", prefix: 10, tun: "Meshora" };
  const scenarios = {
    join: {},
    saved: { network: code },
    connecting: { network: code, phase: "connecting" },
    connected: { network: code, phase: "connected", me, coordConnected: true, peers },
    empty: { network: code, phase: "connected", me, coordConnected: false, peers: [] },
    rejected: {
      network: code,
      phase: "failed",
      error: { kind: "rejected", message: "注册失败：协调服务拒绝了本机：不在名单里" },
    },
    invite: {
      network: code,
      phase: "failed",
      error: { kind: "rejected", message: "注册失败：协调服务拒绝了本机：邀请码不对，可能已经换过了：向建网络的人要一个新的网络码" },
    },
    tun: {
      network: code,
      phase: "failed",
      error: { kind: "tun", message: "创建虚拟网卡 Meshora 失败：加载 C:\\Program Files\\Meshora\\wintun.dll 失败：找不到指定的模块。" },
    },
    unreachable: {
      network: code,
      phase: "failed",
      error: { kind: "unreachable", message: "注册失败：连接协调服务超时" },
    },
  };
  let ov = { ...base, ...(scenarios[scenario] || {}) };
  let tick = 0;

  const handlers = {
    overview() {
      tick += 1;
      if (ov.phase === "connected") {
        ov = {
          ...ov,
          // 延迟有点起伏，偶尔一个毛刺：历史曲线上才看得出东西
          peers: ov.peers.map((p, i) => {
            if (!p.online) return p;
            const base = peers[i].rttMs;
            const wobble = Math.round(Math.sin(tick / 3 + i) * base * 0.15 + (tick % 23 === 0 ? base * 0.8 : 0));
            return { ...p, rx: p.rx + 60_000 * (i + 1), tx: p.tx + 20_000 * (i + 1), rttMs: Math.max(1, base + wobble) };
          }),
        };
      }
      return ov;
    },
    connect({ code: next }) {
      if (next !== undefined && !next.includes("@")) throw "网络码应为 公钥@地址:端口，没找到 @";
      ov = { ...ov, network: next ?? ov.network, phase: "connecting", error: null };
      setTimeout(() => {
        ov = { ...ov, phase: "connected", me, coordConnected: true, peers };
      }, 1500);
    },
    disconnect() {
      ov = { ...ov, phase: "idle", me: null, peers: [], error: null };
    },
    forget() {
      ov = { ...ov, phase: "idle", me: null, peers: [], error: null, network: null };
    },
    set_prefer_broadcast({ on }) {
      ov = { ...ov, preferBroadcast: on };
    },
    set_auto_connect({ on }) {
      ov = { ...ov, autoConnect: on };
    },
    set_private_network({ on }) {
      ov = { ...ov, privateNetwork: on };
    },
    set_name({ name }) {
      const cleaned = name.trim().slice(0, 32);
      ov = { ...ov, name: cleaned };
      return cleaned;
    },
    logs() {
      return [
        "2026-09-30T10:00:00.000Z  INFO meshorad: 本机身份 key=" + id,
        "2026-09-30T10:00:00.120Z  INFO meshora_control: 已注册到协调服务 ip=100.64.0.3",
        "2026-09-30T10:00:00.340Z  INFO meshorad: 虚拟网卡已就绪 tun=Meshora ip=100.64.0.3 prefix=10",
        "2026-09-30T10:00:01.002Z  INFO meshora_control: 切换路径 peer=Kx81…",
      ];
    },
  };

  window.__TAURI__ = {
    core: {
      async invoke(command, args) {
        await new Promise((resolve) => setTimeout(resolve, 60));
        const handler = handlers[command];
        if (!handler) throw `没有这个命令：${command}`;
        return handler(args || {});
      },
    },
  };
})();
