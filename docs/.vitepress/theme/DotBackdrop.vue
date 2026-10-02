<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'

/**
 * 全站背景：和 Meshora 客户端（crates/meshora-desktop/ui/sky.js）同一套点阵。
 *
 * 深色底一层细点（"网"），左上一团极淡的品牌蓝。指针附近的点朝它微微聚拢、变亮，指针走了慢慢散回原位。
 * 点画在一张画布上：Glassium 发现画布内容变了会重新收进场景，磨砂玻璃后面的点也跟着动。
 * 只在动的时候出帧；窗口看不见时停；系统要求减少动画时只画静止的点。
 *
 * 常驻在 Layout 的 layout-top 插槽，换页不重建。
 */

/** 点的间距（CSS 像素） */
const GAP = 24
/** 指针影响的范围：高斯分布的标准差 */
const SIGMA = 90
/** 最近处的点朝指针挪过去的比例 */
const PULL = 0.22
const BASE_ALPHA = 0.14
const LIT_ALPHA = 0.7

const canvas = ref<HTMLCanvasElement | null>(null)

let ctx: CanvasRenderingContext2D | null = null
let width = 0
let height = 0
let ratio = 1
let frame = 0
let still: MediaQueryList | null = null
const target = { x: 0, y: 0, on: 0 }
const at = { x: 0, y: 0, on: 0 }

function resize() {
  const el = canvas.value
  if (!el) return
  ratio = Math.min(window.devicePixelRatio || 1, 2)
  width = window.innerWidth
  height = window.innerHeight
  el.width = Math.round(width * ratio)
  el.height = Math.round(height * ratio)
  el.style.width = `${width}px`
  el.style.height = `${height}px`
  draw()
}

function draw() {
  if (!ctx) return
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0)
  ctx.clearRect(0, 0, width, height)
  const strength = still?.matches ? 0 : at.on
  const reach = SIGMA * 3
  ctx.fillStyle = `rgba(255, 255, 255, ${BASE_ALPHA})`
  const lit: [number, number, number][] = []
  for (let y = GAP / 2; y < height; y += GAP) {
    for (let x = GAP / 2; x < width; x += GAP) {
      const dx = at.x - x
      const dy = at.y - y
      if (strength > 0.01 && Math.abs(dx) < reach && Math.abs(dy) < reach) {
        const w = Math.exp(-(dx * dx + dy * dy) / (2 * SIGMA * SIGMA)) * strength
        if (w > 0.02) {
          lit.push([x + dx * PULL * w, y + dy * PULL * w, w])
          continue
        }
      }
      ctx.fillRect(x - 0.75, y - 0.75, 1.5, 1.5)
    }
  }
  for (const [x, y, w] of lit) {
    ctx.fillStyle = `rgba(${Math.round(255 - 85 * w)}, ${Math.round(255 - 60 * w)}, 255, ${BASE_ALPHA + LIT_ALPHA * w})`
    ctx.beginPath()
    ctx.arc(x, y, 0.9 + 1.1 * w, 0, Math.PI * 2)
    ctx.fill()
  }
}

function step() {
  frame = 0
  at.x += (target.x - at.x) * 0.16
  at.y += (target.y - at.y) * 0.16
  at.on += (target.on - at.on) * 0.1
  draw()
  const moving =
    Math.abs(target.x - at.x) > 0.3 || Math.abs(target.y - at.y) > 0.3 || Math.abs(target.on - at.on) > 0.005
  if (moving && !document.hidden) frame = requestAnimationFrame(step)
}

function kick() {
  if (!frame && !document.hidden && !still?.matches) frame = requestAnimationFrame(step)
}

function aim(x: number, y: number) {
  target.x = x
  target.y = y
  if (at.on < 0.01) {
    at.x = target.x
    at.y = target.y
  }
  target.on = 1
  kick()
}

function onMove(event: PointerEvent) {
  aim(event.clientX, event.clientY)
}

/** 手机上一滑就开始滚动，浏览器随即停发 pointermove：跟着触摸事件走（passive，不挡滚动） */
function onTouch(event: TouchEvent) {
  const finger = event.touches[0]
  if (finger) aim(finger.clientX, finger.clientY)
}

function onTouchEnd(event: TouchEvent) {
  if (event.touches.length === 0) onLeave()
}

function onLeave() {
  target.on = 0
  kick()
}

function onVisibility() {
  if (document.hidden && frame) {
    cancelAnimationFrame(frame)
    frame = 0
  } else {
    kick()
  }
}

onMounted(() => {
  ctx = canvas.value?.getContext('2d') ?? null
  still = window.matchMedia('(prefers-reduced-motion: reduce)')
  window.addEventListener('pointermove', onMove, { passive: true })
  window.addEventListener('pointerdown', onMove, { passive: true })
  window.addEventListener('touchstart', onTouch, { passive: true })
  window.addEventListener('touchmove', onTouch, { passive: true })
  window.addEventListener('touchend', onTouchEnd, { passive: true })
  window.addEventListener('touchcancel', onLeave, { passive: true })
  document.documentElement.addEventListener('pointerleave', onLeave)
  window.addEventListener('blur', onLeave)
  window.addEventListener('resize', resize)
  document.addEventListener('visibilitychange', onVisibility)
  resize()
})

onUnmounted(() => {
  if (frame) cancelAnimationFrame(frame)
  window.removeEventListener('pointermove', onMove)
  window.removeEventListener('pointerdown', onMove)
  window.removeEventListener('touchstart', onTouch)
  window.removeEventListener('touchmove', onTouch)
  window.removeEventListener('touchend', onTouchEnd)
  window.removeEventListener('touchcancel', onLeave)
  document.documentElement.removeEventListener('pointerleave', onLeave)
  window.removeEventListener('blur', onLeave)
  window.removeEventListener('resize', resize)
  document.removeEventListener('visibilitychange', onVisibility)
})
</script>

<template>
  <div class="mesh-sky" aria-hidden="true">
    <i class="sky-glow" />
    <canvas ref="canvas" class="dots" />
  </div>
</template>

<style>
.mesh-sky {
  position: fixed;
  inset: 0;
  z-index: -1;
  overflow: hidden;
  pointer-events: none;
}

.mesh-sky .sky-glow {
  position: absolute;
  left: -25vmax;
  top: -30vmax;
  width: 80vmax;
  height: 80vmax;
  background: radial-gradient(circle, rgba(61, 107, 255, 0.3), rgba(61, 107, 255, 0) 66%);
}

.mesh-sky .dots {
  position: absolute;
  left: 0;
  top: 0;
}
</style>
