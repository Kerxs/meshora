<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import { withBase } from 'vitepress'

/**
 * 首页"三步联机"：每步一张客户端的截图（scripts/capture 录的）。
 *
 * 宽屏三栏并排，序号之间一条线连起来；窄屏改成横向滑动的卡片（露出下一张的边，一看就知道能滑），
 * 下面三个点指示看到第几步，点一下跳过去。
 */
const steps = [
  {
    title: '建一个网络',
    text: '起个名字点"建网络"。默认放在官方服务器上，朋友在哪都连得进来；也能不用服务器，和朋友直连。',
    image: 'step-create',
    alt: '建一个网络：起名字，选官方服务器、本机、自己的服务器，或者不用服务器'
  },
  {
    title: '把网络码发给朋友',
    text: '网主页上一键复制网络码，发到聊天里。也能发只能用一次、或者 24 小时后作废的网络码。',
    image: 'step-invite',
    alt: '网主页上的网络码：复制、换一个，还能发一次性和 24 小时的'
  },
  {
    title: '朋友贴进来，就在一个局域网里了',
    text: '每个人多一个局域网地址，游戏里的房间列表直接看得见对方。',
    image: 'step-joined',
    alt: '网状图：每个人的地址、走直连还是中继、延迟多少'
  }
]

const list = ref<HTMLOListElement | null>(null)
const current = ref(0)
let observer: IntersectionObserver | null = null

onMounted(() => {
  const root = list.value
  if (!root || !('IntersectionObserver' in window)) return
  // 横向滑动时哪张卡露出得最多就是当前这步。宽屏三张都在，没有"当前"，指示点也是藏着的
  observer = new IntersectionObserver(
    entries => {
      for (const e of entries) {
        if (e.isIntersecting && e.intersectionRatio > 0.6) current.value = Number((e.target as HTMLElement).dataset.i)
      }
    },
    { root, threshold: [0.6] }
  )
  root.querySelectorAll('.step').forEach(el => observer!.observe(el))
})

onUnmounted(() => observer?.disconnect())

function go(i: number) {
  const card = list.value?.children[i] as HTMLElement | undefined
  if (!card || !list.value) return
  list.value.scrollTo({ left: card.offsetLeft - list.value.offsetLeft, behavior: 'smooth' })
}
</script>

<template>
  <section class="steps">
    <header class="head">
      <p class="m-eyebrow">三步联机</p>
      <h2 class="m-h2">不用配端口，不用开服务器</h2>
    </header>
    <ol ref="list" class="track">
      <li v-for="(step, i) in steps" :key="step.title" class="step" :data-i="i">
        <span class="n">{{ i + 1 }}</span>
        <h3>{{ step.title }}</h3>
        <p>{{ step.text }}</p>
        <div class="shot">
          <img :src="withBase(`/demo/${step.image}.webp`)" :alt="step.alt" loading="lazy" decoding="async" />
        </div>
      </li>
    </ol>
    <div class="dots" role="tablist" aria-label="三步联机">
      <button
        v-for="(step, i) in steps"
        :key="step.title"
        type="button"
        role="tab"
        :aria-selected="current === i"
        :aria-label="`第 ${i + 1} 步：${step.title}`"
        :class="{ on: current === i }"
        @click="go(i)"
      />
    </div>
  </section>
</template>

<style scoped>
.steps {
  padding: var(--m-section-y) 0;
}

.track {
  position: relative;
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 20px;
  margin: 32px 0 0;
  padding: 0;
  list-style: none;
}

/*
  三个序号之间的连线：从第一个序号的圆心连到第三个的，入场时从左往右画出来。
  序号的圆心在卡片里 22px（内边距）+ 15px（半径）处；最后一栏宽 (100% - 2 × 20px 间距) / 3
*/
.track::before {
  content: '';
  position: absolute;
  z-index: 1;
  top: 37px;
  left: 37px;
  right: calc((100% - 40px) / 3 - 37px);
  height: 1px;
  background: linear-gradient(90deg, rgba(61, 107, 255, 0.9), rgba(127, 231, 220, 0.7));
  transform-origin: left center;
  transition: transform 1200ms var(--meshora-ease-out) 300ms;
  pointer-events: none;
}

/* 整条写进 :global()，原因见 HomeHero.vue */
:global(.meshora-animate .steps:not(.in-view) .track::before) {
  transform: scaleX(0);
}

.step {
  position: relative;
  display: flex;
  flex-direction: column;
  margin: 0;
  padding: 22px 22px 0;
  border-radius: var(--m-radius-card);
  overflow: hidden;
}

.n {
  position: relative;
  z-index: 2;
  display: grid;
  place-items: center;
  width: 30px;
  height: 30px;
  border-radius: 50%;
  background: var(--m-accent);
  box-shadow: 0 0 0 5px rgba(61, 107, 255, 0.18), 0 6px 18px -4px rgba(61, 107, 255, 0.8);
  color: #fff;
  font-weight: 700;
  font-size: 14px;
}

h3 {
  margin: 16px 0 0;
  font-size: 18px;
  font-weight: 700;
  line-height: 1.4;
  color: #fff;
}

.step p {
  margin: 8px 0 18px;
  font-size: 14.5px;
  line-height: 1.75;
  color: var(--m-c-body);
  flex: 1;
}

/* 界面里相关的那一块（局部截图，原尺寸看得清），太高的露出上半截，底下渐隐 */
.shot {
  position: relative;
  margin-bottom: 22px;
  border-radius: 14px;
  overflow: hidden;
  border: 1px solid rgba(255, 255, 255, 0.12);
  box-shadow: 0 16px 40px -16px rgba(0, 0, 0, 0.6);
}

.shot img {
  display: block;
  width: 100%;
  max-height: 300px;
  object-fit: cover;
  object-position: top center;
  transition: transform 700ms var(--meshora-ease-out);
}

@media (hover: hover) and (pointer: fine) {
  .step:hover .shot img {
    transform: scale(1.025);
  }
}

.dots {
  display: none;
}

/* ---------- 窄屏：横向滑动的卡片 ---------- */
@media (max-width: 900px) {
  .track {
    display: flex;
    gap: 14px;
    margin: 24px calc(-1 * var(--m-gutter, 24px)) 0;
    padding: 0 var(--m-gutter, 24px) 6px;
    overflow-x: auto;
    scroll-snap-type: x mandatory;
    scroll-padding-inline: var(--m-gutter, 24px);
    overscroll-behavior-x: contain;
    scrollbar-width: none;
    -webkit-overflow-scrolling: touch;
  }

  .track::-webkit-scrollbar {
    display: none;
  }

  .track::before {
    content: none;
  }

  .step {
    flex: 0 0 min(82%, 420px);
    scroll-snap-align: start;
    padding: 20px 18px 0;
    /* 触屏上没有 Glassium，卡片自己画一层 CSS 毛玻璃 */
    background: linear-gradient(180deg, rgba(40, 46, 64, 0.62), rgba(22, 26, 36, 0.62));
    border: 1px solid rgba(255, 255, 255, 0.1);
    backdrop-filter: blur(18px) saturate(140%);
    -webkit-backdrop-filter: blur(18px) saturate(140%);
  }

  /* 宽屏下 Glassium 管的卡片，窄屏下别和上面的底叠两层 */
  .step[glass] {
    background: transparent;
    border-color: transparent;
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
  }

  .shot {
    margin-bottom: 18px;
  }

  .shot img {
    max-height: 220px;
  }

  .dots {
    display: flex;
    justify-content: center;
    gap: 8px;
    margin-top: 16px;
  }

  .dots button {
    width: 8px;
    height: 8px;
    padding: 0;
    border: 0;
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.28);
    transition: width 320ms var(--meshora-ease-out), background-color 320ms;
    /* 点很小，手指点的范围放大 */
    position: relative;
  }

  .dots button::after {
    content: '';
    position: absolute;
    inset: -10px -6px;
  }

  .dots button.on {
    width: 22px;
    background: var(--m-accent-hi);
  }
}
</style>
