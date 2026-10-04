<script setup lang="ts">
import DemoFrame from './DemoFrame.vue'

/**
 * 首页的一段功能：一边文字、一边客户端画面。`reverse` 时画面在左。
 * 窄屏上下排，画面在上：先看见界面，再读字。
 *
 * 用法（标签和 Markdown 之间留空行，中间的内容才按 Markdown 编译）：
 *
 *   <FeatureRow eyebrow="广播转发" title="房间列表里直接看见" image="network-desktop">
 *
 *   正文……
 *
 *   </FeatureRow>
 */
defineProps<{
  title: string
  eyebrow?: string
  image: string
  video?: string
  /** 录屏从第几秒开始放，见 DemoFrame */
  start?: number
  kind?: 'desktop' | 'phone'
  alt?: string
  reverse?: boolean
}>()
</script>

<template>
  <section :class="['feature', { reverse, phone: kind === 'phone' }]">
    <div class="copy">
      <p v-if="eyebrow" class="m-eyebrow">{{ eyebrow }}</p>
      <h2 class="m-h2">{{ title }}</h2>
      <div class="body"><slot /></div>
    </div>
    <DemoFrame class="media" :kind="kind" :image="image" :video="video" :start="start" :alt="alt || title" />
  </section>
</template>

<style scoped>
.feature {
  position: relative;
  display: grid;
  grid-template-columns: minmax(0, 0.85fr) minmax(0, 1.15fr);
  align-items: center;
  gap: clamp(28px, 5vw, 72px);
  padding: var(--m-section-y) 0;
}

/* 两段之间一条两头渐隐的细线 */
.feature::before {
  content: '';
  position: absolute;
  top: 0;
  left: 10%;
  right: 10%;
  height: 1px;
  background: linear-gradient(90deg, transparent, rgba(255, 255, 255, 0.1), transparent);
}

/* 手机画面窄：给它窄一点的一栏，文字宽一点 */
.feature.phone {
  grid-template-columns: minmax(0, 1.3fr) minmax(0, 0.7fr);
}

.feature.phone .media {
  width: 100%;
  max-width: 300px;
  justify-self: center;
}

.feature.reverse .copy {
  order: 2;
}

.body {
  margin-top: 16px;
  font-size: var(--m-fs-body);
  line-height: var(--m-lh-body);
  color: var(--m-c-body);
  text-wrap: pretty;
}

.body :deep(p) {
  margin: 0 0 12px;
}

.body :deep(strong) {
  color: #fff;
  font-weight: 600;
}

.body :deep(ul) {
  margin: 14px 0 0;
  padding: 0;
  list-style: none;
}

/* 列表项前面换成小勾，和正文拉开层次 */
.body :deep(li) {
  position: relative;
  margin: 0 0 8px;
  padding-left: 26px;
}

.body :deep(li)::before {
  content: '';
  position: absolute;
  left: 2px;
  top: 0.55em;
  width: 14px;
  height: 14px;
  border-radius: 50%;
  background:
    url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16'%3E%3Cpath d='M4.5 8.2l2.3 2.3 4.7-4.9' fill='none' stroke='%237fe7dc' stroke-width='1.8' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E") center / 12px no-repeat,
    rgba(127, 231, 220, 0.14);
}

.body :deep(a) {
  color: var(--m-c-eyebrow);
  font-weight: 600;
  /* 常驻一条半透明下划线，悬停时变亮 */
  text-decoration: underline;
  text-decoration-color: color-mix(in srgb, currentColor 40%, transparent);
  text-underline-offset: 4px;
  transition: color 200ms, text-decoration-color 200ms;
}

@media (hover: hover) and (pointer: fine) {
  .body :deep(a:hover) {
    color: #c3d3ff;
    text-decoration-color: currentColor;
  }
}

/*
  画面的入场（reveal.ts 加 reveal / reveal--media）：从画面那一侧滑进来、带一点缩放，比文字晚一拍。
  只在"还没入场"时写位移 —— 入场之后要交给 custom.css 的 .reveal-in 收回到原位
*/
.media.reveal--media:not(.reveal-in) {
  transform: translate3d(24px, 16px, 0) scale(0.97);
}

.reverse .media.reveal--media:not(.reveal-in) {
  transform: translate3d(-24px, 16px, 0) scale(0.97);
}

.media.reveal--media.reveal-in {
  transition-duration: 900ms;
  transition-delay: 120ms;
}

@media (max-width: 900px) {
  .feature,
  .feature.phone {
    grid-template-columns: minmax(0, 1fr);
    gap: 24px;
  }

  /* 窄屏：画面在上，文字在下 */
  .feature .media {
    order: -1;
  }

  .feature.reverse .copy {
    order: 0;
  }

  .feature.phone .media {
    max-width: 240px;
  }

  .media.reveal--media:not(.reveal-in),
  .reverse .media.reveal--media:not(.reveal-in) {
    transform: translate3d(0, 20px, 0) scale(0.97);
  }
}
</style>
