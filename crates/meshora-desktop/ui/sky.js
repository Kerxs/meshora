// 背景：深色底上几团柔和的品牌色光（蓝、紫，角落一点暖色和青色），上面一层细点阵（"网"）。
// 指针附近的点朝指针微微聚拢、变亮，指针走了慢慢散回原位。样式在 app.css（"背景：点阵"）。
//
// 客户端（index.html）、安卓（同一份）、安装程序（setup.html）都用它：页面里放一个空的 <div class="sky">，
// 在 Glassium 之前引入这个模块。光和点都画在同一张画布上。
// 电脑上这张画布交给 Glassium 当场景（useSkyAsGlassScene）：玻璃把后面的光晕模糊、折射成带颜色的面板；
// 手机上玻璃用 CSS 画，backdrop-filter 直接透过页面上的这张画布，一样带颜色。
// 规矩和别处一样：不写内联样式，尺寸走 CSSOM。

/** 安卓客户端：玻璃用 CSS 画（和 app.js 的 PHONE 同一个判断） */
const PHONE_UA = /Android/i.test(navigator.userAgent);

/** 点的间距（CSS 像素） */
const GAP = 24;
/** 指针影响的范围：高斯分布的标准差 */
const SIGMA = 90;
/** 最近处的点朝指针挪过去的比例 */
const PULL = 0.22;
/** 平常的点有多亮 */
const BASE_ALPHA = 0.14;
/** 指针跟前的点再亮多少 */
const LIT_ALPHA = 0.7;
/** 打开时点阵从中心一圈圈亮起：离中心每多一像素晚这么多毫秒 */
const RIPPLE_MS_PER_PX = 1;
/** 每个点从暗到亮用多久 */
const RIPPLE_FADE = 520;


/**
 * 背景上的几团光：位置是相对窗口的比例，半径是窗口长边的比例。
 * 玻璃（磨砂）把它们化成一片片带颜色的面板 —— 光就是材质的颜色来源，没有它玻璃只是一片灰
 */
const GLOWS = [
  { x: 0.08, y: 0.02, r: 0.78, rgb: "61, 107, 255", a: 0.46 },
  { x: 0.82, y: 0.5, r: 0.6, rgb: "124, 77, 255", a: 0.34 },
  { x: 0.02, y: 1.02, r: 0.38, rgb: "230, 73, 128", a: 0.24 },
  { x: 1.0, y: 1.04, r: 0.34, rgb: "18, 184, 134", a: 0.18 },
];

/**
 * 页面上的卡片、面板用的液态玻璃：不模糊，后面的点阵透过来是清楚的；边缘一圈折射把点阵往里拉弯，
 * 带一点色散（红蓝分开）—— 玻璃的曲率就是靠拉弯的点阵看出来的，模糊一大点阵就没了、边缘也看不出弯。
 * 只在电脑上（GPU 画）有折射和色散；手机上玻璃用 CSS 画，没有折射，还是用磨砂。
 * 弹窗、提示条（overlay）不用它：它们压在别的内容上，要磨砂才看得清字
 */
export const LIQUID_GLASS = PHONE_UA ? { glass: "frosted" } : {
  glass: "regular",
  "glass-blur": "0",
  "glass-refraction": "0.22",
  "glass-distortion": "1",
  "glass-dispersion": "0.6",
  "glass-tint": "rgba(255,255,255,0.08)",
};

/**
 * 按钮的液态玻璃边：卡片上的按钮也把后面的点阵在边上拉弯、带色散（材质照旧是 clear / tinted，只加强边缘）。
 * 手机上玻璃用 CSS 画，没有折射，给空的
 */
export const LIQUID_EDGE = PHONE_UA
  ? {}
  : { "glass-refraction": "0.5", "glass-distortion": "1", "glass-dispersion": "0.45" };

/**
 * 输入框、文本框：深色玻璃（颜色和原来的实底 --well 一样），电脑上边缘也有折射、色散
 */
export const WELL_GLASS = { glass: "tinted", "glass-tint": "rgba(0, 0, 0, 0.2)", ...LIQUID_EDGE };

/**
 * 网状图连线上的延迟胶囊：深色的玻璃，边上折射、色散、一圈亮边。手机上返回空的，照旧是 SVG 画的深色胶囊
 */
export function liquidPill(tint = "rgba(10, 16, 48, 0.5)", radius = "1frac") {
  if (PHONE_UA) return {};
  return {
    glass: "regular",
    "glass-blur": "0",
    "glass-refraction": "0.5",
    "glass-distortion": "1",
    "glass-dispersion": "0.45",
    "glass-highlight": "1.2",
    "glass-tint": tint,
    "glass-corner-radius": radius,
  };
}

/**
 * 头像、网状图节点的彩色玻璃泡：颜色是着色，边上折射、色散、一圈亮边。`dim` 是不在线的（颜色淡一些）。
 * `lens`：网状图里的"我" —— 无色的透明玻璃（不要蓝色），后面的点阵透过来，在边上被拉弯、分成红绿蓝
 *（和卡片边上的一样；鼠标靠近时点阵变亮，看得最清楚）。
 * 手机上不用（列表里一堆 CSS 玻璃拖慢滚动），返回空的，照旧是纯色圆
 */
export function liquidBubble(hex, dim = false, lens = false) {
  if (PHONE_UA) return {};
  const n = Number.parseInt(hex.slice(1), 16);
  const rgb = `${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}`;
  if (lens) {
    return {
      glass: "regular",
      "glass-blur": "0",
      "glass-refraction": "0.45",
      "glass-distortion": "1.6",
      "glass-dispersion": "1.5",
      "glass-magnify": "0",
      "glass-highlight": "1.2",
      "glass-tint": "rgba(255, 255, 255, 0.04)",
      "glass-corner-radius": "1frac",
    };
  }
  return {
    glass: "regular",
    "glass-blur": "0",
    "glass-refraction": "0.6",
    "glass-distortion": "1",
    "glass-dispersion": "0.4",
    "glass-highlight": "1.2",
    "glass-tint": `rgba(${rgb}, ${dim ? 0.4 : 0.78})`,
    "glass-corner-radius": "1frac",
  };
}

/** 画布每重画一次派发它：交给 Glassium 当场景时，靠它通知重新上传（不每帧都传） */
const DRAW_EVENT = "sky:draw";

/**
 * 电脑上把背景画布交给 Glassium 当场景：玻璃后面真的是这片光和点，模糊、折射、带颜色都对。
 * 页面自己的背景换成透明、画布藏起来（不然点会出现两遍），底色交给场景。
 * 没有 GPU、Glassium 用 CSS 画的时候不用（backdrop-filter 本来就透过页面上的画布）
 */
export function useSkyAsGlassScene(glassium) {
  const canvas = document.querySelector(".sky canvas.dots");
  if (!canvas) return;
  glassium.ready
    .then(() => {
      const stage = glassium.stage;
      if (!stage || !stage.active) return;
      return stage.setScene(canvas, { fit: "fill", background: "#07080c" }).then(() => {
        canvas.addEventListener(DRAW_EVENT, () => stage.refreshScene());
        document.documentElement.classList.add("sky-in-scene");
      });
    })
    .catch(() => {});
}

function start(sky) {
  const canvas = document.createElement("canvas");
  canvas.className = "dots";
  sky.append(canvas);
  // 光晕预先画在一张离屏画布上（改尺寸时重画）：每帧画点之前贴一下，不必每帧算渐变
  const glow = document.createElement("canvas");
  const ctx = canvas.getContext("2d");
  const still = matchMedia("(prefers-reduced-motion: reduce)");

  let width = 0;
  let height = 0;
  let ratio = 1;
  // 指针：target 是真实位置，at 是缓动后的位置；strength 是"指针在不在"，进出窗口时渐变
  const target = { x: 0, y: 0, on: 0 };
  const at = { x: 0, y: 0, on: 0 };
  let frame = 0;
  // 打开时的扩散：从这一刻起算；窗口一开始就看不见（开机藏在托盘里）、或者要求减少动画，就不扩散
  let born = document.hidden || still.matches ? null : performance.now();

  function resize() {
    ratio = Math.min(window.devicePixelRatio || 1, 2);
    width = window.innerWidth;
    height = window.innerHeight;
    canvas.width = Math.round(width * ratio);
    canvas.height = Math.round(height * ratio);
    canvas.style.width = `${width}px`;
    canvas.style.height = `${height}px`;
    paintGlow();
    draw();
  }

  function paintGlow() {
    glow.width = canvas.width;
    glow.height = canvas.height;
    const g = glow.getContext("2d");
    const long = Math.max(glow.width, glow.height);
    for (const { x, y, r, rgb, a } of GLOWS) {
      const cx = x * glow.width;
      const cy = y * glow.height;
      const radius = r * long;
      const gradient = g.createRadialGradient(cx, cy, 0, cx, cy, radius);
      gradient.addColorStop(0, `rgba(${rgb}, ${a})`);
      gradient.addColorStop(0.45, `rgba(${rgb}, ${a * 0.45})`);
      gradient.addColorStop(1, `rgba(${rgb}, 0)`);
      g.fillStyle = gradient;
      g.fillRect(0, 0, glow.width, glow.height);
    }
  }

  /** 扩散还没走到的点有多亮（0 到 1）；扩散完了是 null */
  function rippleAt(now) {
    if (born === null) return null;
    const age = now - born;
    const far = Math.hypot(width, height) / 2;
    if (age > far * RIPPLE_MS_PER_PX + RIPPLE_FADE) {
      born = null;
      return null;
    }
    const cx = width / 2;
    const cy = height / 2;
    return (x, y) => Math.min(1, Math.max(0, (age - Math.hypot(x - cx, y - cy) * RIPPLE_MS_PER_PX) / RIPPLE_FADE));
  }

  function draw() {
    paintDots();
    canvas.dispatchEvent(new Event(DRAW_EVENT));
  }

  function paintDots() {
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    ctx.drawImage(glow, 0, 0);
    ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
    const reveal = rippleAt(performance.now());
    if (reveal) {
      // 扩散中：每个点按它离中心多远亮起来，正在亮的那一圈稍大一点
      for (let y = GAP / 2; y < height; y += GAP) {
        for (let x = GAP / 2; x < width; x += GAP) {
          const r = reveal(x, y);
          if (r <= 0) continue;
          const crest = r < 1 ? Math.sin(r * Math.PI) : 0;
          ctx.fillStyle = `rgba(${Math.round(255 - 80 * crest)}, ${Math.round(255 - 50 * crest)}, 255, ${BASE_ALPHA * r + 0.5 * crest})`;
          const size = 1.5 + 1.6 * crest;
          ctx.fillRect(x - size / 2, y - size / 2, size, size);
        }
      }
      return;
    }
    const strength = still.matches ? 0 : at.on;
    const reach = SIGMA * 3;
    // 离指针远的点都一样：一次画完，不逐个算
    ctx.fillStyle = `rgba(255, 255, 255, ${BASE_ALPHA})`;
    const lit = [];
    for (let y = GAP / 2; y < height; y += GAP) {
      for (let x = GAP / 2; x < width; x += GAP) {
        const dx = at.x - x;
        const dy = at.y - y;
        if (strength > 0.01 && Math.abs(dx) < reach && Math.abs(dy) < reach) {
          const w = Math.exp(-(dx * dx + dy * dy) / (2 * SIGMA * SIGMA)) * strength;
          if (w > 0.02) {
            lit.push([x + dx * PULL * w, y + dy * PULL * w, w]);
            continue;
          }
        }
        ctx.fillRect(x - 0.75, y - 0.75, 1.5, 1.5);
      }
    }
    // 指针跟前的点：挪过去一点、大一点、亮一点，带一点品牌蓝
    for (const [x, y, w] of lit) {
      const r = 0.9 + 1.1 * w;
      ctx.fillStyle = `rgba(${Math.round(255 - 85 * w)}, ${Math.round(255 - 60 * w)}, 255, ${BASE_ALPHA + LIT_ALPHA * w})`;
      ctx.beginPath();
      ctx.arc(x, y, r, 0, Math.PI * 2);
      ctx.fill();
    }
  }

  // 缓动到目标位置；到了就停，不空转
  // 扩散：一直出帧直到走完。rAF 在看不见时不触发，最后用一个定时器兜底，保证点阵一定亮全
  function ripple() {
    draw();
    if (born !== null && !document.hidden) requestAnimationFrame(ripple);
  }

  function step() {
    frame = 0;
    const ease = 0.12;
    at.x += (target.x - at.x) * ease;
    at.y += (target.y - at.y) * ease;
    at.on += (target.on - at.on) * 0.1;
    draw();
    const moving = Math.abs(target.x - at.x) > 0.3 || Math.abs(target.y - at.y) > 0.3 || Math.abs(target.on - at.on) > 0.005;
    if (moving && !document.hidden) frame = requestAnimationFrame(step);
  }

  function kick() {
    if (!frame && !document.hidden && !still.matches) frame = requestAnimationFrame(step);
  }

  function aim(x, y) {
    target.x = x;
    target.y = y;
    // 指针刚进来：从它所在的位置亮起，不是从角落飞过去
    if (at.on < 0.01) {
      at.x = target.x;
      at.y = target.y;
    }
    target.on = 1;
    kick();
  }

  const move = (event) => aim(event.clientX, event.clientY);

  // 手机上手指一滑页面就开始滚动，浏览器随即停发 pointermove（先来一个 pointercancel）：
  // 触摸事件照常一直来，跟着它走。passive，不挡滚动
  function touch(event) {
    const finger = event.touches[0];
    if (finger) aim(finger.clientX, finger.clientY);
  }

  function leave() {
    target.on = 0;
    kick();
  }

  // 安装程序的窗口小、整个被一块面板盖着，指针几乎总在字上面：亮起来的点会压着字，不跟指针
  const follow = !document.body.classList.contains("setup");
  if (follow) {
    window.addEventListener("pointermove", move, { passive: true });
    window.addEventListener("pointerdown", move, { passive: true });
    document.documentElement.addEventListener("pointerleave", leave);
    window.addEventListener("touchstart", touch, { passive: true });
    window.addEventListener("touchmove", touch, { passive: true });
    // 手机上手指抬起就散开
    window.addEventListener("touchend", (event) => event.touches.length === 0 && leave(), { passive: true });
    window.addEventListener("touchcancel", leave, { passive: true });
    window.addEventListener("blur", leave);
  }
  window.addEventListener("resize", resize);
  document.addEventListener("visibilitychange", () => {
    if (document.hidden && frame) {
      cancelAnimationFrame(frame);
      frame = 0;
    } else {
      kick();
    }
  });
  resize();
  if (born !== null) {
    requestAnimationFrame(ripple);
    setTimeout(() => {
      born = null;
      draw();
    }, 2500);
  }
}

// 放在最后：上面的常量（GLOWS 之类）都定义好了再开始画
const sky = document.querySelector(".sky");
if (sky && !sky.firstElementChild) start(sky);
