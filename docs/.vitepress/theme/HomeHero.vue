<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import { withBase } from 'vitepress'
import DemoFrame from './DemoFrame.vue'

/**
 * 首页首屏：左边一句话说清楚是什么、下载；右边是客户端的录屏 —— 电脑窗口里"一起联机"，
 * 右下角压一部手机。
 *
 * 窄屏上画面排到文字下面，主次反过来：手机在前、居中，电脑窗口缩在后面只放截图 ——
 * 手机上看电脑窗口的录屏字太小，还白白多下一段视频。
 *
 * 版本号和状态写在这里（发版后跟着改），说法和 README、路线图一致：刚发布，还没在真实环境里验证过。
 */
const VERSION = '1.1.1'

/** 和下面 CSS 里窄屏的断点一致 */
const NARROW = '(max-width: 640px)'

const stage = ref<HTMLElement | null>(null)
const narrow = ref(false)
let cleanup: (() => void) | null = null

onMounted(() => {
  const mq = window.matchMedia(NARROW)
  narrow.value = mq.matches
  const onChange = (e: MediaQueryListEvent) => (narrow.value = e.matches)
  mq.addEventListener('change', onChange)

  // 指针视差：只在有鼠标、没要求减少动画时。画面跟着指针微微转一点，幅度很小
  const fine = window.matchMedia('(hover: hover) and (pointer: fine)').matches
  const reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches
  const el = stage.value
  let frame = 0
  const onMove = (e: PointerEvent) => {
    if (frame || !el) return
    frame = requestAnimationFrame(() => {
      frame = 0
      const x = e.clientX / window.innerWidth - 0.5
      const y = e.clientY / window.innerHeight - 0.5
      el.style.setProperty('--px', x.toFixed(3))
      el.style.setProperty('--py', y.toFixed(3))
    })
  }
  if (fine && !reduce && el) window.addEventListener('pointermove', onMove, { passive: true })

  cleanup = () => {
    mq.removeEventListener('change', onChange)
    window.removeEventListener('pointermove', onMove)
    cancelAnimationFrame(frame)
  }
})

onUnmounted(() => cleanup?.())
</script>

<template>
  <section class="hero">
    <div class="copy">
      <p class="status">
        <span class="pulse" />
        <span><b>{{ VERSION }} 已发布</b> · Windows 和安卓</span>
        <span class="status-note">刚发布，还没在真实环境里验证过</span>
      </p>
      <h1>
        <span class="name">Meshora</span>
        <span class="line">异地好友，同一个局域网</span>
      </h1>
      <p class="tagline">
        点一下建网络，把网络码发给朋友，贴进来就加入。只支持局域网联机的游戏，隔着城市也能开房间 ——
        能直连就直连，打不通走中继，全程加密。
      </p>
      <div class="actions">
        <a class="m-btn brand download" href="https://github.com/Kerxs/meshora/releases/latest">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M12 4v11m0 0l-4.5-4.5M12 15l4.5-4.5M5 19.5h14" />
          </svg>
          免费下载
        </a>
        <a class="m-btn" :href="withBase('/guide/desktop')">怎么用</a>
        <a class="m-btn ghost" href="https://github.com/Kerxs/meshora">
          <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
            <path d="M12 .5a11.5 11.5 0 0 0-3.64 22.41c.58.1.79-.25.79-.56v-2c-3.2.7-3.88-1.37-3.88-1.37-.52-1.33-1.28-1.69-1.28-1.69-1.04-.71.08-.7.08-.7 1.15.08 1.76 1.19 1.76 1.19 1.03 1.76 2.69 1.25 3.35.96.1-.75.4-1.25.73-1.54-2.55-.29-5.24-1.28-5.24-5.68 0-1.26.45-2.28 1.19-3.09-.12-.29-.52-1.46.11-3.05 0 0 .97-.31 3.17 1.18a11 11 0 0 1 5.77 0c2.2-1.49 3.17-1.18 3.17-1.18.63 1.59.23 2.76.11 3.05.74.81 1.19 1.83 1.19 3.09 0 4.41-2.69 5.38-5.26 5.67.41.36.78 1.06.78 2.14v3.17c0 .31.21.67.8.56A11.5 11.5 0 0 0 12 .5z" />
          </svg>
          GitHub
        </a>
      </div>
      <p class="fine">免费、开源（MIT）。Windows 安装程序没有代码签名，SmartScreen 会提示"未知发布者"。</p>
    </div>

    <div ref="stage" class="stage">
      <div class="glow" aria-hidden="true" />
      <DemoFrame
        class="desk"
        image="network-desktop"
        :video="narrow ? undefined : 'connect'"
        alt="Meshora 桌面客户端：贴上网络码，连上之后朋友一个个出现在网状图上"
        eager
      />
      <DemoFrame class="phone" kind="phone" image="network-phone" video="phone" alt="Meshora 安卓客户端：同一个网络，同一套界面" eager />
    </div>
  </section>
</template>

<style scoped>
.hero {
  display: grid;
  grid-template-columns: minmax(0, 0.9fr) minmax(0, 1.25fr);
  align-items: center;
  gap: clamp(28px, 5vw, 64px);
  padding: clamp(40px, 7vw, 96px) 0 clamp(32px, 5vw, 64px);
}

.status {
  display: inline-flex;
  flex-wrap: wrap;
  align-items: center;
  column-gap: 8px;
  row-gap: 2px;
  margin: 0 0 22px;
  padding: 6px 14px 6px 12px;
  /* 胶囊是 Glassium 的玻璃（glass.ts） */
  border-radius: 999px;
  font-size: 12.5px;
  line-height: 1.5;
  color: rgba(255, 255, 255, 0.86);
}

.status b {
  font-weight: 600;
  color: #fff;
}

.status-note {
  color: rgba(255, 255, 255, 0.6);
}

.status-note::before {
  content: '·';
  margin-right: 8px;
}

.pulse {
  flex: none;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--m-teal);
  animation: pulse 2.6s ease-out infinite;
}

@keyframes pulse {
  0% { box-shadow: 0 0 0 0 rgba(127, 231, 220, 0.55); }
  70% { box-shadow: 0 0 0 7px rgba(127, 231, 220, 0); }
  100% { box-shadow: 0 0 0 0 rgba(127, 231, 220, 0); }
}

h1 {
  margin: 0;
  display: grid;
  gap: 8px;
}

/* 项目名：白到淡蓝再到 Logo 那颗青色节点，一层很淡的渐变 */
.name {
  font-size: clamp(48px, 6.4vw, 76px);
  font-weight: 800;
  line-height: 1.02;
  letter-spacing: -0.03em; /* 拉丁字母收紧；中文那行不收 */
  color: #fff;
  background: linear-gradient(100deg, #fff 30%, #cdd8ff 62%, #8ff0e4 100%);
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
  padding-bottom: 0.04em;
}

.line {
  font-size: clamp(26px, 3.4vw, 40px);
  font-weight: 700;
  line-height: 1.25;
  color: #cdd8ff;
  text-wrap: balance;
}

.tagline {
  margin: 22px 0 0;
  max-width: 34em;
  font-size: 16.5px;
  line-height: 1.8;
  color: rgba(255, 255, 255, 0.8);
  text-wrap: pretty;
}

.actions {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
  margin-top: 30px;
}

.fine {
  margin: 18px 0 0;
  font-size: 12.5px;
  line-height: 1.7;
  color: rgba(255, 255, 255, 0.5);
}

/* 画面：电脑窗口在后，手机压在右下角 */
.stage {
  --px: 0;
  --py: 0;
  position: relative;
  padding: 0 7% 9% 0;
  perspective: 1400px;
}

/* 画面后面一团淡淡的品牌蓝，让画框从点阵里"浮"起来 */
.glow {
  position: absolute;
  inset: 10% 5% 0 0;
  z-index: -1;
  border-radius: 50%;
  background: radial-gradient(closest-side, rgba(61, 107, 255, 0.35), rgba(127, 231, 220, 0.08) 60%, transparent);
  filter: blur(30px);
  pointer-events: none;
}

.desk,
.phone {
  transition: transform 600ms var(--meshora-ease-out);
}

.desk {
  transform: rotateY(calc(var(--px) * -5deg)) rotateX(calc(var(--py) * 4deg)) translate3d(calc(var(--px) * -6px), calc(var(--py) * -6px), 0);
}

.phone {
  position: absolute;
  right: 0;
  bottom: 0;
  width: 26%;
  transform: translate3d(calc(var(--px) * 12px), calc(var(--py) * 10px), 0);
}

/* ---------- 入场：intro 放完后 reveal.ts 给 .hero 加 hero-in ---------- */
/* 整条写进 :global()：Vue 会把 ":global(.a) .b" 编译成只剩 ".a"，那就把整个 <html> 藏了 */
:global(.meshora-animate .hero:not(.hero-in) .copy > *),
:global(.meshora-animate .hero:not(.hero-in) .stage) {
  opacity: 0;
}

.hero.hero-in .copy > * {
  animation: hero-rise 760ms var(--meshora-ease-out) both;
}

.hero.hero-in .copy > :nth-child(1) { animation-delay: 40ms; }
.hero.hero-in .copy > :nth-child(2) { animation-delay: 120ms; }
.hero.hero-in .copy > :nth-child(3) { animation-delay: 200ms; }
.hero.hero-in .copy > :nth-child(4) { animation-delay: 280ms; }
.hero.hero-in .copy > :nth-child(5) { animation-delay: 360ms; }

.hero.hero-in .stage {
  animation: hero-stage 1000ms var(--meshora-ease-out) 220ms both;
}

.hero.hero-in .stage .phone {
  animation: hero-phone 900ms var(--meshora-ease-out) 520ms both;
}

@keyframes hero-rise {
  from { opacity: 0; transform: translateY(16px); }
}

@keyframes hero-stage {
  from { opacity: 0; transform: translateY(28px) scale(0.97); filter: blur(6px); }
}

@keyframes hero-phone {
  from { opacity: 0; translate: 0 36px; }
}

/* ---------- 平板：上下排，手机框大一点 ---------- */
@media (max-width: 900px) {
  .hero {
    grid-template-columns: minmax(0, 1fr);
    padding-top: 40px;
  }

  .stage {
    max-width: 680px;
    width: 100%;
    justify-self: center;
  }

  .phone {
    width: 30%;
  }
}

/* ---------- 手机：文字居中，手机框在前、电脑窗口在后 ---------- */
@media (max-width: 640px) {
  .hero {
    gap: 36px;
    padding: 28px 0 20px;
    text-align: center;
  }

  .status {
    justify-content: center;
    border-radius: 16px;
    margin-bottom: 20px;
  }

  .status-note {
    flex-basis: 100%;
    font-size: 12px;
  }

  .status-note::before {
    content: none;
  }

  h1 {
    gap: 6px;
  }

  .name {
    font-size: clamp(46px, 14vw, 58px);
  }

  .line {
    font-size: clamp(24px, 7vw, 30px);
  }

  .tagline {
    margin: 18px auto 0;
    font-size: 15.5px;
    line-height: 1.85;
  }

  .actions {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
    margin-top: 26px;
  }

  .actions .download {
    grid-column: 1 / -1;
    height: 52px;
    font-size: 16px;
  }

  .actions .m-btn:not(.download) {
    height: 44px;
    padding: 0 12px;
  }

  .fine {
    max-width: 30em;
    margin: 16px auto 0;
  }

  .stage {
    padding: 0;
    height: min(118vw, 520px);
    perspective: none;
  }

  .glow {
    inset: 15% 0 5%;
  }

  /* 电脑窗口退到后面，压暗一点 */
  .desk {
    position: absolute;
    top: 7%;
    left: 0;
    width: 88%;
    opacity: 0.55;
    transform: none;
  }

  .phone {
    left: 50%;
    right: auto;
    bottom: 0;
    width: 52%;
    max-width: 240px;
    transform: translateX(-34%);
  }
}

@media (prefers-reduced-motion: reduce) {
  .pulse {
    animation: none;
  }
}
</style>
