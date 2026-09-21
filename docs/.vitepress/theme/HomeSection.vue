<script setup lang="ts">
import { withBase } from 'vitepress'

/**
 * 首页的毛玻璃区块容器。
 *
 * 用法（组件标签和 Markdown 之间要留空行，VitePress 才会把中间内容按 Markdown 编译）：
 *
 *   <HomeSection title="使用场景" more="/guide/what-is-meshora">
 *
 *   普通 Markdown…
 *
 *   </HomeSection>
 */
defineProps<{
  title: string
  eyebrow?: string
  /** 指向完整版页面的链接，留空则不显示 */
  more?: string
  moreText?: string
}>()
</script>

<template>
  <section class="home-section">
    <p v-if="eyebrow" class="home-section__eyebrow">{{ eyebrow }}</p>
    <h2 class="home-section__title">{{ title }}</h2>

    <div class="home-section__body">
      <slot />
    </div>

    <!--
      必须包 withBase：Markdown 里的 ](/guide/x) 会被 VitePress 自动加上 base，
      但组件里动态传进来的路径不会，站点部署在 /meshora/ 子路径下会直接 404。
    -->
    <a v-if="more" class="home-section__more" :href="withBase(more)">
      {{ moreText || '读完整版' }}
      <span aria-hidden="true">→</span>
    </a>
  </section>
</template>

<style scoped>
.home-section {
  margin: 28px 0;
  padding: clamp(22px, 3.4vw, 38px);
  border-radius: 16px;
  border: 1px solid var(--meshora-glass-border);
  background: var(--meshora-glass-bg);
  backdrop-filter: blur(var(--meshora-glass-blur)) saturate(140%);
  -webkit-backdrop-filter: blur(var(--meshora-glass-blur)) saturate(140%);
}

.home-section__eyebrow {
  margin: 0 0 6px;
  font-size: 11.5px;
  font-weight: 600;
  letter-spacing: 0.12em;
  text-transform: uppercase;
  color: var(--meshora-on-glass-dim);
}

.home-section__title {
  margin: 0 0 18px;
  font-size: clamp(21px, 2.6vw, 27px);
  font-weight: 600;
  line-height: 1.3;
  letter-spacing: -0.01em;
  color: var(--meshora-on-glass);
  border: 0;
  padding: 0;
}

/* 区块内的 Markdown 首尾不要再撑出额外空白 */
.home-section__body :deep(> :first-child) {
  margin-top: 0;
}
.home-section__body :deep(> :last-child) {
  margin-bottom: 0;
}

/*
  同样用白字而不是品牌蓝（原因见 ScenarioGrid 的注释）。
  白字会和正文同色，所以加一条常驻下划线来保住「这是链接」的可辨识性。
*/
.home-section__more {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  margin-top: 20px;
  font-size: 14px;
  font-weight: 500;
  color: var(--meshora-on-glass);
  text-decoration: underline;
  text-decoration-color: color-mix(in srgb, currentColor 45%, transparent);
  text-underline-offset: 4px;
  transition: gap 0.2s var(--meshora-ease, ease), text-decoration-color 0.2s;
}
.home-section__more:hover {
  gap: 10px;
  text-decoration-color: currentColor;
}

/*
  不支持 backdrop-filter 时退回更不透明的底色。
  否则会变成「半透明面板直接压在流动的流体上」，文字完全没法读。
*/
@supports not ((backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px))) {
  .home-section {
    background: var(--meshora-glass-bg-solid);
  }
}

@media (prefers-reduced-motion: reduce) {
  .home-section__more {
    transition: none;
  }
}
</style>
