<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import { withBase } from 'vitepress'

/**
 * 客户端画面的画框：电脑窗口（圆角、细边）或者手机（厚边框、刘海）。
 *
 * 画面是 scripts/capture 录的截图（WebP）和录屏（WebM），放在 docs/public/demo/。
 * 有录屏时播录屏，截图当 poster：加载前、系统要求减少动画时都显示截图。
 * 不在屏幕里时暂停，省电；回到屏幕里接着播。
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
}>()

/**
 * 每段录屏从哪一秒开始放：录屏是从打开空白页那一刻起录的，开头那一小截是页面还没画出来的黑屏（还闪一下白）。
 * 数字由抽帧量出来（看每一帧的平均亮度），scripts/capture 重录之后要重新量
 */
const START: Record<string, number> = { connect: 0.9, punch: 1.0, boot: 0.3, phone: 0.5 }

const el = ref<HTMLVideoElement | null>(null)
const still = ref(false)
let observer: IntersectionObserver | null = null

onMounted(() => {
  still.value = window.matchMedia('(prefers-reduced-motion: reduce)').matches
  const video = el.value
  if (!video || still.value) return
  const start = props.start ?? START[props.video ?? ''] ?? 0
  // 跳过开头；放完了回到开头接着放（原生的 loop 会回到 0，又露出那截黑屏）
  const rewind = () => {
    video.currentTime = start
  }
  if (video.readyState >= 1) rewind()
  else video.addEventListener('loadedmetadata', rewind, { once: true })
  video.addEventListener('ended', () => {
    rewind()
    video.play().catch(() => {})
  })
  observer = new IntersectionObserver(
    ([entry]) => {
      if (entry.isIntersecting) video.play().catch(() => {})
      else video.pause()
    },
    { threshold: 0.25 }
  )
  observer.observe(video)
})

onUnmounted(() => observer?.disconnect())

const src = (name: string, ext: string) => withBase(`/demo/${name}.${ext}`)
</script>

<template>
  <figure :class="['demo-frame', props.kind === 'phone' ? 'is-phone' : 'is-desktop']">
    <div class="screen">
      <video
        v-if="props.video && !still"
        ref="el"
        :src="src(props.video, 'webm')"
        :poster="src(props.image, 'webp')"
        muted
        playsinline
        preload="metadata"
        :aria-label="props.alt"
      />
      <img v-else :src="src(props.image, 'webp')" :alt="props.alt" loading="lazy" decoding="async" />
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
