// 流体背景：建好光团，让它们跟着指针轻轻流动，窗口看不见时停下。样式和流动的路径在 app.css（"背景：流体"）。
//
// 客户端（index.html）、安卓（同一份）、安装程序（setup.html）都用它：页面里放一个空的 <div class="sky">，
// 在 Glassium 之前引入这个模块。规矩和别处一样：不写内联样式，位置走 CSSOM（style.setProperty）。

/** 每团光：class 和它跟着指针偏多少（近大远小，有层次） */
const ORBS = [
  ["orb-a", 0.6],
  ["orb-b", -0.8],
  ["orb-c", 1],
  ["orb-d", -1.3],
  ["orb-e", 1.5],
  ["orb-f", -1.1],
];

/** 指针在窗口边上时，光团最多偏多少像素 */
const REACH = 36;

function build(sky) {
  for (const [name, k] of ORBS) {
    const flow = document.createElement("span");
    flow.className = "flow";
    flow.style.setProperty("--k", String(k));
    const orb = document.createElement("i");
    orb.className = `orb ${name}`;
    flow.append(orb);
    sky.append(flow);
  }
}

function follow(sky) {
  let pending = null;
  const apply = () => {
    const { x, y } = pending;
    pending = null;
    sky.style.setProperty("--px", `${(x * REACH).toFixed(1)}px`);
    sky.style.setProperty("--py", `${(y * REACH).toFixed(1)}px`);
  };
  // 指针位置换成 -1 ~ 1；一帧最多写一次（缓动交给 CSS 的 transition）
  const move = (event) => {
    const x = (event.clientX / window.innerWidth) * 2 - 1;
    const y = (event.clientY / window.innerHeight) * 2 - 1;
    const first = pending === null;
    pending = { x, y };
    if (first) setTimeout(apply, 50);
  };
  window.addEventListener("pointermove", move, { passive: true });
  window.addEventListener("pointerdown", move, { passive: true });
}

function pauseWhenHidden(sky) {
  const sync = () => sky.classList.toggle("paused", document.hidden);
  document.addEventListener("visibilitychange", sync);
  sync();
}

// ---------- 备选背景（挑选中）：<html data-bg="aurora|grid|mono|nodes">，没写就是上面的流体 ----------

function layer(sky, className) {
  const el = document.createElement("i");
  el.className = className;
  sky.append(el);
  return el;
}

/** 节点网：稀疏的节点和细连线（Logo 的语言），用 SVG 画，整张图很慢地漂 */
function buildNodes(sky) {
  const NS = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(NS, "svg");
  svg.setAttribute("class", "net");
  svg.setAttribute("viewBox", "0 0 1600 1000");
  svg.setAttribute("preserveAspectRatio", "xMidYMid slice");
  // 固定的点位：每次打开都一样，不随机
  const pts = [
    [120, 140], [420, 90], [760, 210], [1080, 120], [1440, 230],
    [240, 460], [560, 400], [900, 520], [1260, 440], [1520, 600],
    [90, 800], [430, 760], [780, 880], [1120, 780], [1400, 900],
  ];
  const near = (a, b) => Math.hypot(a[0] - b[0], a[1] - b[1]) < 400;
  for (let i = 0; i < pts.length; i++) {
    for (let j = i + 1; j < pts.length; j++) {
      if (!near(pts[i], pts[j])) continue;
      const line = document.createElementNS(NS, "line");
      line.setAttribute("x1", pts[i][0]);
      line.setAttribute("y1", pts[i][1]);
      line.setAttribute("x2", pts[j][0]);
      line.setAttribute("y2", pts[j][1]);
      svg.append(line);
    }
  }
  for (const [x, y] of pts) {
    const dot = document.createElementNS(NS, "circle");
    dot.setAttribute("cx", x);
    dot.setAttribute("cy", y);
    dot.setAttribute("r", "3");
    svg.append(dot);
  }
  sky.append(svg);
}

const VARIANTS = {
  aurora(sky) {
    layer(sky, "aurora-band band-1");
    layer(sky, "aurora-band band-2");
  },
  grid(sky) {
    layer(sky, "grid-glow");
    layer(sky, "grid-dots");
    sky.append(spot(sky));
  },
  mono(sky) {
    layer(sky, "mono-wash");
    layer(sky, "mono-grain");
  },
  nodes(sky) {
    layer(sky, "nodes-glow");
    buildNodes(sky);
  },
};

/** 点阵：指针附近一圈微亮的光 */
function spot(sky) {
  const el = document.createElement("i");
  el.className = "grid-spot";
  window.addEventListener(
    "pointermove",
    (event) => {
      sky.style.setProperty("--sx", `${event.clientX}px`);
      sky.style.setProperty("--sy", `${event.clientY}px`);
    },
    { passive: true },
  );
  return el;
}

const sky = document.querySelector(".sky");
if (sky && !sky.firstElementChild) {
  const variant = VARIANTS[document.documentElement.dataset.bg];
  if (variant) {
    sky.classList.add(`bg-${document.documentElement.dataset.bg}`);
    variant(sky);
  } else {
    build(sky);
    follow(sky);
  }
  pauseWhenHidden(sky);
}
