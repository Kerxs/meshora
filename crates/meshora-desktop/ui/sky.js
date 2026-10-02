// 背景：深色底上一层细点阵（"网"），左上一团极淡的品牌蓝。指针附近的点朝指针微微聚拢、变亮，
// 指针走了慢慢散回原位。样式在 app.css（"背景：点阵"）。
//
// 客户端（index.html）、安卓（同一份）、安装程序（setup.html）都用它：页面里放一个空的 <div class="sky">，
// 在 Glassium 之前引入这个模块。点画在一张画布上：Glassium 发现画布内容变了会重新收进场景，玻璃后面的点也跟着动。
// 规矩和别处一样：不写内联样式，尺寸走 CSSOM。

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

const sky = document.querySelector(".sky");
if (sky && !sky.firstElementChild) start(sky);

function start(sky) {
  const glow = document.createElement("i");
  glow.className = "sky-glow";
  const canvas = document.createElement("canvas");
  canvas.className = "dots";
  sky.append(glow, canvas);
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
    draw();
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
    ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
    ctx.clearRect(0, 0, width, height);
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

  window.addEventListener("pointermove", move, { passive: true });
  window.addEventListener("pointerdown", move, { passive: true });
  document.documentElement.addEventListener("pointerleave", leave);
  window.addEventListener("touchstart", touch, { passive: true });
  window.addEventListener("touchmove", touch, { passive: true });
  // 手机上手指抬起就散开
  window.addEventListener("touchend", (event) => event.touches.length === 0 && leave(), { passive: true });
  window.addEventListener("touchcancel", leave, { passive: true });
  window.addEventListener("blur", leave);
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
