<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { withBase } from 'vitepress'

/**
 * 客户端画面的画框：电脑窗口（圆角、细边）或者手机（厚边框、刘海）。
 *
 * 画面是 scripts/capture 录的截图（WebP）和录屏（WebM），放在 docs/public/demo/。
 * 有录屏时播录屏，截图当 poster：加载前、系统要求减少动画、开了省流量时都显示截图。
 * 录屏要等画框快滚进屏幕才开始下（src 那时才填上），不在屏幕里时暂停，省电也省流量；回到屏幕里接着播。
 *
 * 画框本身是 CSS 毛玻璃，不标 Glassium：画面里本来就是客户端的玻璃，再叠一层会糊。
 */
const props = defineProps<{
  kind?: 'desktop' | 'phone'
  /** 截图文件名（不带扩展名），在 /demo/ 下 */
  image: string
  /** 录屏文件名（不带扩展名），没有就只放截图 */
  video?: string
  /** 从第几秒开始放（同一段录屏在不同地方想从不同的地方看起）。不给就用下面 START 里的 */
  start?: number
  alt: string
  /** 首屏里的画面：截图不懒加载 */
  eager?: boolean
}>()

/**
 * 每段录屏从哪一秒开始放：录屏是从打开空白页那一刻起录的，开头那一小截是页面还没画出来的黑屏（还闪一下白）。
 * 数字由抽帧量出来（看每一帧的平均亮度），scripts/capture 重录之后要重新量
 */
const START: Record<string, number> = { connect: 0.9, punch: 1.0, boot: 0.3, phone: 0.5 }

const el = ref<HTMLVideoElement | null>(null)
const still = ref(false)
/** 已经滚到附近、可以开始下录屏了 */
const near = ref(false)
let observer: IntersectionObserver | null = null

function wire() {
  observer?.disconnect()
  observer = null
  const video = el.value
  if (!video) return
  const start = props.start ?? START[props.video ?? ''] ?? 0
  // 跳过开头；放完了回到开头接着放（原生的 loop 会回到 0，又露出那截黑屏）
  const rewind = () => {
    video.currentTime = start
  }
  video.addEventListener('loadedmetadata', rewind, { once: true })
  video.addEventListener('ended', () => {
    rewind()
    video.play().catch(() => {})
  })
  observer = new IntersectionObserver(
    ([entry]) => {
      if (entry.isIntersecting) {
        near.value = true
        // src 刚填上时 play() 会等数据，不用等 loadedmetadata
        nextTick(() => video.play().catch(() => {}))
      } else video.pause()
    },
    // 提前一点开始下，滚到眼前时已经能播
    { rootMargin: '200px 0px', threshold: 0 }
  )
  observer.observe(video)
}

onMounted(() => {
  const saveData = (navigator as Navigator & { connection?: { saveData?: boolean } }).connection?.saveData
  still.value = window.matchMedia('(prefers-reduced-motion: reduce)').matches || !!saveData
  if (still.value) return
  wire()
})

// 首屏在窄屏上会把电脑窗口的录屏换成截图：换了之后重新接线
watch(
  () => props.video,
  () => nextTick(wire)
)

onUnmounted(() => observer?.disconnect())

const src = (name: string, ext: string) => withBase(`/demo/${name}.${ext}`)
</script>

<template>
  <figure :class="['demo-frame', props.kind === 'phone' ? 'is-phone' : 'is-desktop']">
    <div class="screen">
      <video
        v-if="props.video && !still"
        ref="el"
        :src="near ? src(props.video, 'webm') : undefined"
        :poster="src(props.image, 'webp')"
        muted
        playsinline
        preload="none"
        :aria-label="props.alt"
      />
      <img
        v-else
        :src="src(props.image, 'webp')"
        :alt="props.alt"
        :loading="props.eager ? 'eager' : 'lazy'"
        decoding="async"
      />
    </div>
  </figure>
</template>

<style scoped>
.demo-frame {
  margin: 0;
  position: relative;
}

.screen {
  overflow: hidden;
  background: #07080c;
}

.screen video,
.screen img {
  display: block;
  width: 100%;
  height: auto;
}

/* 电脑窗口：客户端自己画了标题栏，这里只要圆角、细亮边和投影 */
.is-desktop .screen {
  aspect-ratio: 1100 / 720;
  border-radius: 14px;
  border: 1px solid rgba(255, 255, 255, 0.14);
  box-shadow:
    0 1px 0 rgba(255, 255, 255, 0.08) inset,
    0 30px 80px -20px rgba(0, 0, 0, 0.7),
    0 0 0 1px rgba(0, 0, 0, 0.4);
}

/* 手机：厚边框、圆角大、顶上一个刘海 */
.is-phone {
  padding: 9px;
  border-radius: 38px;
  background: linear-gradient(160deg, #2a2e3a, #12141b);
  border: 1px solid rgba(255, 255, 255, 0.16);
  box-shadow: 0 30px 70px -18px rgba(0, 0, 0, 0.75);
}

.is-phone .screen {
  aspect-ratio: 390 / 844;
  border-radius: 30px;
}

/* 画框上沿一道细高光，像玻璃边 */
.is-desktop::after {
  content: '';
  position: absolute;
  top: 0;
  left: 12%;
  right: 12%;
  height: 1px;
  background: linear-gradient(90deg, transparent, rgba(255, 255, 255, 0.45), transparent);
  pointer-events: none;
}

.is-phone::before {
  content: '';
  position: absolute;
  z-index: 1;
  top: 17px;
  left: 50%;
  width: 22%;
  height: 16px;
  transform: translateX(-50%);
  border-radius: 999px;
  background: #050507;
}
</style>
