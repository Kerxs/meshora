<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'

/**
 * 全站流体背景层。
 *
 * 这是整个会话里**唯一**一个 WebGL 上下文：渲染在 Layout 的 layout-top 插槽，
 * position: fixed 铺满视口、压在所有内容之下，路由切换时不销毁不重建。
 *
 * 之前它是挂在 home-hero-before 插槽上的 FluidHero，只存在于首页，
 * 每次进出首页都要创建/销毁一次上下文。改成常驻之后，既让文档页也有东西
 * 可以透过毛玻璃，又反而减少了上下文 churn。
 *
 * 首页与文档页的强弱差别由 CSS 覆盖层控制（见 custom.css 的 scrim），
 * 不是靠换着色器参数 —— 换参数就得重建 uniform，没必要。
 */

const host = ref<HTMLElement | null>(null)
// 着色器挂载完成后才淡入，否则会看到 CSS 兜底渐变被"啪"地换成画布
const live = ref(false)

/** 配色：换一套只改这一行 */
const COLORS = ['#AED5F3', '#2E58A4', '#04101F']

/*
  参数是照着 DeepSeek 线上实测值做起点、再实测微调出来的。三条经验：

  1) softness 偏高（>0.7）时，中间那个颜色会吞掉整个画面，结果是一片发白的雾。
     0.4 左右才看得出层次。
  2) 可见区域的混色偏向数组里**最后**一个颜色，所以深色要放在末位，
     否则压不住白字。
  3) 新版 API 的 scale 语义和 DeepSeek 用的旧版完全不同，不能照抄数值。
*/
const PARAMS = {
  proportion: 0.46,
  softness: 0.4,
  distortion: 0.06,
  swirl: 0.28,
  swirlIterations: 12,
  shape: 'checks' as 'checks' | 'stripes' | 'edge',
  shapeScale: 0.1,
  scale: 3.4,
  rotation: 12, // 新版 API 是角度制
  offsetX: 0.01,
  offsetY: 0.4,

  /*
    speed 别往下调。实测每秒画面平均变化（0~255 通道差，本组参数下）：

        0.6 → 10   1.0 → 17   2.0 → 35   3.0 → 49

    低于 1 在这种柔和渐变上根本看不出在动 —— 0.6 曾被当成"背景是静止的"。
    2.0 大约 7 秒走完一轮，缓慢但明确。库自带预设的取值范围是 1~20。
  */
  speed: 2
}

/*
  像素预算：它现在是**全站常驻**的，不再只在首页，所以比之前压得更低。
  220 万 → 130 万。配合 minPixelRatio: 1，4K 屏也不会按全分辨率渲染。
*/
const MAX_PIXELS = 1300000

let mount: { dispose: () => void } | null = null
let unmounted = false

onMounted(async () => {
  if (!host.value) return

  // SSG 阶段在 Node 里跑，没有 window / WebGL，所以必须动态引入，不能写成顶层 import
  let L: any
  try {
    L = await import('@paper-design/shaders')
  } catch (err) {
    // 加载失败就留着 CSS 兜底渐变，页面不会开天窗。
    // 但一定要留下线索 —— 静默失败会让人对着一张兜底渐变查半天。
    console.warn('[Meshora] 流体背景着色器未能加载，已回落到 CSS 渐变：', err)
    return
  }
  if (unmounted || !host.value) return

  const tex = L.getShaderNoiseTexture()
  // ShaderMount 会拒绝尚未解码完成的纹理，而 data: URI 同样是异步解码的，
  // 这个 await 是必需的，不是防御性代码
  if (!tex.complete) {
    await tex.decode().catch(
      () => new Promise(r => { tex.onload = r; tex.onerror = r })
    )
  }
  if (unmounted || !host.value) return

  const reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches
  const d = L.defaultPatternSizing

  mount = new L.ShaderMount(
    host.value,
    L.warpFragmentShader,
    {
      u_colors: COLORS.map(L.getShaderColorFromString),
      u_colorsCount: COLORS.length,
      u_proportion: PARAMS.proportion,
      u_softness: PARAMS.softness,
      u_distortion: PARAMS.distortion,
      u_swirl: PARAMS.swirl,
      u_swirlIterations: PARAMS.swirlIterations,
      u_shape: L.WarpPatterns[PARAMS.shape],
      u_shapeScale: PARAMS.shapeScale,
      u_noiseTexture: tex,
      u_scale: PARAMS.scale,
      u_rotation: PARAMS.rotation,
      u_fit: L.ShaderFitOptions[d.fit],
      u_offsetX: PARAMS.offsetX,
      u_offsetY: PARAMS.offsetY,
      u_originX: d.originX,
      u_originY: d.originY,
      u_worldWidth: d.worldWidth,
      u_worldHeight: d.worldHeight
    },
    undefined,
    // speed 传 0 时库会彻底停掉 rAF，得到一张静止渐变且零持续开销
    reduce ? 0 : PARAMS.speed,
    0,
    1, // minPixelRatio
    MAX_PIXELS
  )

  live.value = true
})

onUnmounted(() => {
  unmounted = true
  // 正常情况下这个组件跟着 Layout 活整个会话，不会走到这里。
  // 但热更新和卸载时仍然要释放，否则会漏 WebGL 上下文（浏览器上限约 16 个）。
  mount?.dispose()
  mount = null
})
</script>

<template>
  <div class="fluid-backdrop" aria-hidden="true">
    <div ref="host" class="fluid-backdrop__canvas" :class="{ 'is-live': live }" />
    <!-- 覆盖层：首页很淡、文档页很厚，强弱全靠它，见 custom.css -->
    <div class="fluid-backdrop__scrim" />
  </div>
</template>

<style scoped>
.fluid-backdrop {
  position: fixed;
  inset: 0;
  /*
    必须是 -1：固定定位 + z-index 0 会画在未定位的正文内容之上，把文档页糊住。
    -1 让它画在所有内容之下、但仍在根元素背景之上 ——
    custom.css 里 html 留了一层兜底底色，着色器挂了也不会白屏。
  */
  z-index: -1;
  pointer-events: none;
  overflow: hidden;
  /* 着色器没起来时的兜底，也是首帧底色 */
  background: radial-gradient(130% 150% at 16% 4%, #aed5f3 0%, #2e58a4 42%, #04101f 100%);
}

.fluid-backdrop__canvas {
  position: absolute;
  inset: 0;
}

/* canvas 是库创建的，不在模板里，scoped 样式必须用 :deep 才能命中 */
.fluid-backdrop__canvas :deep(canvas) {
  display: block;
  width: 100%;
  height: 100%;
  opacity: 0;
  transition: opacity 900ms ease;
}
.fluid-backdrop__canvas.is-live :deep(canvas) {
  opacity: 1;
}

.fluid-backdrop__scrim {
  position: absolute;
  inset: 0;
  transition: background-color 400ms ease;
  background-color: var(--meshora-scrim);
}

@media (prefers-reduced-motion: reduce) {
  .fluid-backdrop__canvas :deep(canvas),
  .fluid-backdrop__scrim {
    transition: none;
  }
}
</style>
