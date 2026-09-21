<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { withBase } from 'vitepress'

/**
 * 建连流水线的六步演示。
 *
 * 高亮进入视口后依次走过；悬停/聚焦某一步会把高亮钉在那里，移开恢复行走。
 * 每一步链到 connection-flow 对应的小节，所以它不只是装饰，还是导航。
 *
 * 静止状态（active = -1）是「六步同等呈现」—— 减少动态效果命中时就停在这里，
 * 信息一点不少。
 */
const STEPS = [
  { label: '节点发现', anchor: '_1-节点发现' },
  { label: '身份认证', anchor: '_2-身份认证' },
  { label: 'NAT 穿透', anchor: '_3-nat-穿透' },
  { label: 'P2P 建连', anchor: '_4-p2p-建连' },
  { label: 'Relay 回退', anchor: '_5-relay-回退' },
  { label: '路径选择', anchor: '_6-路径选择' }
]

const root = ref<HTMLElement | null>(null)
const active = ref(-1)
const pinned = ref<number | null>(null)

let timer: number | undefined
let io: IntersectionObserver | undefined

function start() {
  if (timer !== undefined) return
  timer = window.setInterval(() => {
    if (pinned.value !== null) return // 钉住时不推进
    active.value = (active.value + 1) % STEPS.length
  }, 900)
}

function stop() {
  if (timer === undefined) return
  clearInterval(timer)
  timer = undefined
}

onMounted(() => {
  // 减少动态效果：保持静止状态，不启动行走
  if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) return

  if (!('IntersectionObserver' in window) || !root.value) {
    active.value = 0
    start()
    return
  }
  // 只在可见时行走，滚出视口就停，别空转
  io = new IntersectionObserver(
    entries => { for (const e of entries) e.isIntersecting ? start() : stop() },
    { threshold: 0.35 }
  )
  io.observe(root.value)
})

onUnmounted(() => {
  stop()
  io?.disconnect()
})

const current = computed(() => (pinned.value !== null ? pinned.value : active.value))
</script>

<template>
  <div class="pipeline" ref="root">
    <ol class="steps">
      <li v-for="(s, i) in STEPS" :key="s.anchor" class="step">
        <a
          class="chip"
          :class="{ on: current === i }"
          :href="withBase(`/guide/connection-flow#${s.anchor}`)"
          @mouseenter="pinned = i"
          @mouseleave="pinned = null"
          @focus="pinned = i"
          @blur="pinned = null"
        >
          <span class="idx">{{ i + 1 }}</span>
          <span class="label">{{ s.label }}</span>
        </a>
        <span v-if="i < STEPS.length - 1" class="arrow" aria-hidden="true">→</span>
      </li>
    </ol>
  </div>
</template>

<style scoped>
.pipeline {
  margin: 28px 0;
}

.steps {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px 2px;
  margin: 0;
  padding: 0;
  list-style: none;
}
.steps li::marker {
  content: none;
}

.step {
  display: flex;
  align-items: center;
  gap: 2px;
}

/*
  尺寸是照着「文档内容区最宽 688px 时六步排成一行」量出来的。
  再放大一点就会折行，行尾会留一个悬空的箭头。
*/
.chip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 6px 10px;
  border: 1px solid var(--meshora-glass-border);
  border-radius: 999px;
  background: var(--meshora-glass-bg-soft);
  backdrop-filter: blur(10px) saturate(140%);
  -webkit-backdrop-filter: blur(10px) saturate(140%);
  color: var(--meshora-on-glass-dim);
  font-size: 13px;
  font-weight: 500;
  line-height: 1.4;
  text-decoration: none;
  white-space: nowrap;
  transition: border-color 0.24s, background-color 0.24s, color 0.24s, transform 0.24s;
}

.chip:hover {
  border-color: var(--vp-c-brand-1);
}

.chip.on {
  border-color: var(--vp-c-brand-1);
  background: var(--vp-c-brand-soft);
  color: var(--vp-c-brand-1);
  transform: translateY(-2px);
}

@supports not ((backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px))) {
  .chip {
    background: var(--meshora-glass-bg-solid);
  }
}

.idx {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: none;
  width: 15px;
  height: 15px;
  border-radius: 50%;
  border: 1px solid currentColor;
  font-family: var(--vp-font-family-mono);
  font-size: 10px;
  opacity: 0.7;
}

.arrow {
  color: var(--vp-c-text-3);
  font-size: 12px;
  padding: 0 1px;
  user-select: none;
}

/* 窄屏改为纵向，箭头跟着转 90 度 */
@media (max-width: 720px) {
  .steps {
    flex-direction: column;
    align-items: flex-start;
    gap: 0;
  }
  .step {
    flex-direction: column;
    align-items: flex-start;
    gap: 0;
  }
  .chip {
    width: 100%;
    border-radius: 8px;
  }
  .arrow {
    display: block;
    transform: rotate(90deg);
    padding: 3px 0 3px 16px;
  }
}

@media (prefers-reduced-motion: reduce) {
  .chip {
    transition: none;
  }
  .chip.on {
    transform: none;
  }
}
</style>
