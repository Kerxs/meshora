<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'

/**
 * 全站的流体背景：和 Meshora 客户端（crates/meshora-desktop/ui/sky.js）同一套光团。
 *
 * 几团彩色光各自沿一条闭合路径漂移、转动、拉伸，跟着指针轻轻偏一点。每团只是一层 radial-gradient，
 * 只动 transform —— Glassium 每帧把它们收进自己的场景，玻璃（导航、卡片、按钮）折射的就是这些在流动的光。
 * 以前用的是整屏 WebGL 着色器：那是一块画布，Glassium 收不进场景，玻璃后面就什么都没有。
 *
 * 常驻在 Layout 的 layout-top 插槽，换页不重建。窗口看不见时停下。
 */

const sky = ref<HTMLElement | null>(null)

/** 每团光：名字、跟着指针偏多少（近大远小，有层次） */
const ORBS: [string, number][] = [
  ['a', 0.6],
  ['b', -0.8],
  ['c', 1],
  ['d', -1.3],
  ['e', 1.5],
  ['f', -1.1]
]

/** 指针在窗口边上时，光团最多偏多少像素 */
const REACH = 36

let pending: { x: number; y: number } | null = null

function onMove(event: PointerEvent) {
  const first = pending === null
  pending = {
    x: (event.clientX / window.innerWidth) * 2 - 1,
    y: (event.clientY / window.innerHeight) * 2 - 1
  }
  if (first) setTimeout(apply, 50)
}

function apply() {
  if (!pending || !sky.value) return
  sky.value.style.setProperty('--px', `${(pending.x * REACH).toFixed(1)}px`)
  sky.value.style.setProperty('--py', `${(pending.y * REACH).toFixed(1)}px`)
  pending = null
}

function onVisibility() {
  sky.value?.classList.toggle('paused', document.hidden)
}

onMounted(() => {
  window.addEventListener('pointermove', onMove, { passive: true })
  document.addEventListener('visibilitychange', onVisibility)
})

onUnmounted(() => {
  window.removeEventListener('pointermove', onMove)
  document.removeEventListener('visibilitychange', onVisibility)
})
</script>

<template>
  <div ref="sky" class="mesh-sky" aria-hidden="true">
    <span v-for="[name, k] in ORBS" :key="name" class="flow" :style="{ '--k': k }">
      <i :class="['orb', `orb-${name}`]" />
    </span>
  </div>
</template>

<style>
.mesh-sky {
  position: fixed;
  inset: 0;
  z-index: -1;
  overflow: hidden;
  pointer-events: none;
  --px: 0px;
  --py: 0px;
}

.mesh-sky .flow {
  position: absolute;
  inset: 0;
  translate: calc(var(--px) * var(--k, 1)) calc(var(--py) * var(--k, 1));
  transition: translate 1.8s cubic-bezier(0.22, 0.8, 0.3, 1);
}

.mesh-sky .orb {
  position: absolute;
  border-radius: 50%;
  animation: var(--path) var(--dur) ease-in-out infinite alternate;
  animation-delay: var(--delay, 0s);
}

.mesh-sky.paused .orb {
  animation-play-state: paused;
}

.mesh-sky .orb-a {
  --path: mesh-flow-a;
  --dur: 26s;
  width: 70vmax;
  height: 70vmax;
  left: -22vmax;
  top: -26vmax;
  background: radial-gradient(circle, #2f5bff 0%, rgba(47, 91, 255, 0) 68%);
}

.mesh-sky .orb-b {
  --path: mesh-flow-b;
  --dur: 33s;
  width: 62vmax;
  height: 62vmax;
  right: -16vmax;
  top: 4vmax;
  background: radial-gradient(circle, #8a4dff 0%, rgba(138, 77, 255, 0) 66%);
}

.mesh-sky .orb-c {
  --path: mesh-flow-c;
  --dur: 41s;
  width: 58vmax;
  height: 58vmax;
  left: 22vmax;
  bottom: -30vmax;
  background: radial-gradient(circle, #13c7a0 0%, rgba(19, 199, 160, 0) 64%);
}

.mesh-sky .orb-d {
  --path: mesh-flow-d;
  --dur: 29s;
  width: 34vmax;
  height: 34vmax;
  right: 20vmax;
  bottom: -10vmax;
  background: radial-gradient(circle, #ff6f61 0%, rgba(255, 111, 97, 0) 66%);
}

.mesh-sky .orb-e {
  --path: mesh-flow-e;
  --dur: 37s;
  --delay: -9s;
  width: 30vmax;
  height: 30vmax;
  left: 34vmax;
  top: 6vmax;
  background: radial-gradient(circle, #3fb8ff 0%, rgba(63, 184, 255, 0) 66%);
}

.mesh-sky .orb-f {
  --path: mesh-flow-f;
  --dur: 47s;
  --delay: -21s;
  width: 26vmax;
  height: 26vmax;
  left: -4vmax;
  bottom: 4vmax;
  background: radial-gradient(circle, #d65bff 0%, rgba(214, 91, 255, 0) 66%);
}

@keyframes mesh-flow-a {
  0% { transform: translate(0, 0) rotate(0deg) scale(1, 1); }
  35% { transform: translate(9vmax, 6vmax) rotate(40deg) scale(1.18, 0.86); }
  70% { transform: translate(3vmax, 12vmax) rotate(95deg) scale(0.9, 1.12); }
  100% { transform: translate(-4vmax, 4vmax) rotate(140deg) scale(1.08, 0.95); }
}

@keyframes mesh-flow-b {
  0% { transform: translate(0, 0) rotate(0deg) scale(1, 1); }
  40% { transform: translate(-10vmax, 8vmax) rotate(-50deg) scale(0.85, 1.2); }
  75% { transform: translate(-4vmax, 16vmax) rotate(-110deg) scale(1.15, 0.9); }
  100% { transform: translate(-12vmax, 6vmax) rotate(-160deg) scale(1, 1.05); }
}

@keyframes mesh-flow-c {
  0% { transform: translate(0, 0) rotate(0deg) scale(1, 1); }
  30% { transform: translate(12vmax, -6vmax) rotate(60deg) scale(1.2, 0.85); }
  65% { transform: translate(-6vmax, -12vmax) rotate(120deg) scale(0.88, 1.15); }
  100% { transform: translate(-14vmax, -4vmax) rotate(180deg) scale(1.1, 0.92); }
}

@keyframes mesh-flow-d {
  0% { transform: translate(0, 0) rotate(0deg) scale(1, 1); }
  45% { transform: translate(-14vmax, -10vmax) rotate(70deg) scale(1.35, 0.8); }
  100% { transform: translate(-6vmax, -20vmax) rotate(150deg) scale(0.9, 1.25); }
}

@keyframes mesh-flow-e {
  0% { transform: translate(0, 0) rotate(0deg) scale(1, 1); }
  50% { transform: translate(14vmax, 10vmax) rotate(-80deg) scale(1.3, 0.8); }
  100% { transform: translate(-8vmax, 18vmax) rotate(-170deg) scale(0.85, 1.2); }
}

@keyframes mesh-flow-f {
  0% { transform: translate(0, 0) rotate(0deg) scale(1, 1); }
  50% { transform: translate(18vmax, -8vmax) rotate(90deg) scale(1.25, 0.85); }
  100% { transform: translate(30vmax, 4vmax) rotate(200deg) scale(0.9, 1.2); }
}

/* 手机上少两团、慢一些 */
@media (max-width: 640px) {
  .mesh-sky .orb-e,
  .mesh-sky .orb-f {
    display: none;
  }

  .mesh-sky .orb {
    animation-duration: calc(var(--dur) * 1.6);
  }
}

@media (prefers-reduced-motion: reduce) {
  .mesh-sky .orb {
    animation: none;
  }

  .mesh-sky .flow {
    transition: none;
  }
}
</style>
