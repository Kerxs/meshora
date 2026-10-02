// 界面上的连贯动画：启动、切页、弹层退场、列表平滑重排、数字滚动。样式在 app.css 的"动画"一节。
//
// 规矩和别处一样：只动 transform 和 opacity，位置走 CSSOM（style.setProperty），不写内联 style 属性。
// 系统要求减少动画时一律瞬间完成。窗口看不见时 requestAnimationFrame 不触发：凡是靠它的，都另有定时器兜底

const SVG_NS = "http://www.w3.org/2000/svg";

/** 系统要求减少动画 */
export const still = () => matchMedia("(prefers-reduced-motion: reduce)").matches;

function svg(tag, attrs, ...children) {
  const el = document.createElementNS(SVG_NS, tag);
  for (const [key, value] of Object.entries(attrs)) el.setAttribute(key, value);
  el.append(...children);
  return el;
}

/** 会一笔一笔画出来的 Logo：四个节点依次弹出，连线随后画出（.logo-draw，样式在 app.css）。安装程序的欢迎页也用它 */
export function drawnLogo(className = "logo-draw") {
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
  const nth = (el, i) => {
    el.style.setProperty("--i", String(i));
    return el;
  };
  return svg(
    "svg",
    { class: className, viewBox: "0 0 64 64", "aria-hidden": "true" },
    svg("g", { stroke: "rgba(255,255,255,.6)", "stroke-width": "3.2", "stroke-linecap": "round" }, ...lines.map(([x1, y1, x2, y2], i) => nth(svg("line", { class: "l", x1, y1, x2, y2, pathLength: "1" }), i))),
    ...nodes.map(([cx, cy, fill], i) => nth(svg("circle", { class: "n", cx, cy, r: "6.5", fill }), i)),
  );
}

/** 等一段时间；看不见时也照样到点（用定时器，不用 rAF） */
const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/**
 * 启动动画：点阵从中心扩散（sky.js 自己做），中间画 Logo；第一份状态到了以后，Logo 缩小飞到左上角的品牌位置，
 * 整个界面浮上来。`ready` 是第一份状态到了的 Promise；最多等 1.8 秒，不让启动变慢。
 * 返回动画走完的 Promise（走完之后界面才收到 enter 动画）
 */
export async function intro(ready) {
  const root = document.documentElement;
  if (still() || document.hidden) return;
  root.classList.add("booting");
  const mark = drawnLogo("logo-draw intro-logo");
  const layer = document.createElement("div");
  layer.className = "intro";
  layer.append(mark);
  document.body.append(layer);
  // Logo 画完至少要这么久；状态最多等到 1.5 秒
  await Promise.all([wait(1350), Promise.race([ready, wait(1800)])]);
  const target = document.querySelector(".brand svg");
  const to = target?.getBoundingClientRect();
  const from = mark.getBoundingClientRect();
  if (to && to.width > 0) {
    // FLIP：从中间的大 Logo 变到品牌那个小 Logo 的位置和大小
    const dx = to.left + to.width / 2 - (from.left + from.width / 2);
    const dy = to.top + to.height / 2 - (from.top + from.height / 2);
    mark.style.setProperty("--fly", `translate(${dx}px, ${dy}px) scale(${to.width / from.width})`);
    layer.classList.add("fly");
  } else {
    layer.classList.add("fade");
  }
  root.classList.remove("booting");
  root.classList.add("booted");
  await wait(780);
  layer.remove();
  setTimeout(() => root.classList.remove("booted"), 1400);
}

/**
 * 换页：旧页往一边淡出，新页从另一边滑进来、里面的卡片依次浮上。`dir` 是 1（往后翻）或 -1（往回翻）。
 * 连着换好几次时只认最后一次
 */
let swapping = 0;
export function swapView(container, next, dir, enter) {
  const old = container.firstElementChild;
  const token = ++swapping;
  next.style.setProperty("--dir", String(dir));
  if (!old || still()) {
    container.replaceChildren(enter(next));
    return;
  }
  old.classList.add("leaving");
  old.style.setProperty("--dir", String(dir));
  setTimeout(() => {
    if (token !== swapping) return;
    container.replaceChildren(enter(next));
    container.scrollTop = 0;
  }, 220);
}

/** 关一个弹层（确认框、连接码弹窗）：缩小、淡出，播完再移除 */
export function closeLayer(scrim) {
  if (still()) {
    scrim.remove();
    return;
  }
  scrim.classList.add("closing");
  setTimeout(() => scrim.remove(), 260);
}

/**
 * 列表重排时平滑过渡（FLIP）：调 `before()` 量下现在的位置，改完 DOM 再调它返回的函数，
 * 挪了位置的元素从旧位置滑到新位置
 */
export function flip(container) {
  if (still()) return () => {};
  const rects = new Map([...container.children].map((el) => [el, el.getBoundingClientRect()]));
  return () => {
    for (const el of container.children) {
      const was = rects.get(el);
      if (!was) continue;
      const now = el.getBoundingClientRect();
      const dx = was.left - now.left;
      const dy = was.top - now.top;
      if (Math.abs(dx) < 1 && Math.abs(dy) < 1) continue;
      el.classList.remove("flip");
      el.style.setProperty("--flip", `translate(${dx}px, ${dy}px)`);
      // 先瞬间放回旧位置，下一帧（定时器，看不见时也走）再放开让它滑过去
      el.classList.add("flip-from");
      setTimeout(() => {
        el.classList.remove("flip-from");
        el.classList.add("flip");
        setTimeout(() => el.classList.remove("flip"), 600);
      }, 20);
    }
  };
}

/**
 * 数字从旧值滚到新值（网速、延迟），约 0.6 秒。`format` 把数字变成文字。
 * 看不见时、要求减少动画时直接写最终值
 */
export function tweenText(el, value, format) {
  const from = el._tweenValue;
  el._tweenValue = value;
  cancelAnimationFrame(el._tweenFrame || 0);
  if (from === undefined || from === null || value === null || document.hidden || still() || from === value) {
    el.textContent = format(value);
    return;
  }
  const start = performance.now();
  const step = (now) => {
    const t = Math.min(1, (now - start) / 600);
    const eased = 1 - (1 - t) ** 4;
    el.textContent = format(from + (value - from) * eased);
    if (t < 1) el._tweenFrame = requestAnimationFrame(step);
  };
  el._tweenFrame = requestAnimationFrame(step);
  // rAF 万一不来（窗口刚被藏起来），也要落到最终值
  setTimeout(() => {
    if (el._tweenValue === value) el.textContent = format(value);
  }, 650);
}

/** 按钮上闪一下"✓ 已复制"，1.4 秒后变回原来的字 */
export function flashCopied(button) {
  if (!button || button._flashing) return;
  const original = [...button.childNodes];
  button._flashing = true;
  button.classList.add("copied");
  button.replaceChildren("✓ 已复制");
  setTimeout(() => {
    button.replaceChildren(...original);
    button.classList.remove("copied");
    button._flashing = false;
  }, 1400);
}
