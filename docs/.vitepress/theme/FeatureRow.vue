<script setup lang="ts">
import DemoFrame from './DemoFrame.vue'

/**
 * 首页的一段功能：一边文字、一边客户端画面。`reverse` 时画面在左。窄屏上下排，画面在下。
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
      <p v-if="eyebrow" class="eyebrow">{{ eyebrow }}</p>
      <h2>{{ title }}</h2>
      <div class="body"><slot /></div>
    </div>
    <DemoFrame class="media" :kind="kind" :image="image" :video="video" :start="start" :alt="alt || title" />
  </section>
</template>

<style scoped>
.feature {
  display: grid;
  grid-template-columns: minmax(0, 0.85fr) minmax(0, 1.15fr);
  align-items: center;
  gap: clamp(28px, 5vw, 72px);
  padding: clamp(36px, 6vw, 80px) 0;
}

/* 手机画面窄：给它窄一点的一栏，文字宽一点 */
.feature.phone {
  grid-template-columns: minmax(0, 1.3fr) minmax(0, 0.7fr);
}

.feature.phone .media {
  max-width: 300px;
  justify-self: center;
}

.feature.reverse .copy {
  order: 2;
}

.eyebrow {
  margin: 0 0 8px;
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0.12em;
  color: #9cb6ff;
}

h2 {
  margin: 0;
  padding: 0;
  border: 0;
  font-size: clamp(24px, 3vw, 32px);
  font-weight: 750;
  line-height: 1.25;
  letter-spacing: -0.005em;
  color: #fff;
  text-wrap: balance;
}

.body {
  margin-top: 14px;
  font-size: 15.5px;
  line-height: 1.8;
  color: rgba(255, 255, 255, 0.76);
}

.body :deep(p) {
  margin: 0 0 12px;
}

.body :deep(strong) {
  color: #fff;
}

.body :deep(ul) {
  margin: 8px 0 0;
  padding-left: 1.2em;
}

.body :deep(a) {
  color: #9cb6ff;
  font-weight: 600;
}

@media (max-width: 900px) {
  .feature,
  .feature.phone {
    grid-template-columns: minmax(0, 1fr);
  }

  .feature.reverse .copy {
    order: 0;
  }
}
</style>
