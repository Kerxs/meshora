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

const sky = document.querySelector(".sky");
if (sky && !sky.firstElementChild) {
  build(sky);
  follow(sky);
  pauseWhenHidden(sky);
}
