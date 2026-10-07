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
  /** 常见问题：段落开头加粗的那句当问题排 */
  faq?: boolean
}>()
// 玻璃由 glass.ts 标上（.home-section 在它的选择器表里），别在模板里写 glass：Glassium 一 import 就接管页面上已有的
// [glass]，那时 glass.ts 还没来得及 configure（触屏上要 backend: 'css'）
</script>

<template>
  <section :class="['home-section', { faq }]">
    <p v-if="eyebrow" class="m-eyebrow">{{ eyebrow }}</p>
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
  padding: clamp(22px, 3.4vw, 40px);
  border-radius: 26px;
}

@media (max-width: 640px) {
  .home-section {
    margin: 20px 0;
    padding: 22px 18px;
    border-radius: var(--m-radius-card);
  }
}

/* 正文：和功能段同一套字号行高 */
.home-section__body {
  font-size: var(--m-fs-body);
  line-height: var(--m-lh-body);
}

/*
  常见问题（faq）：段落开头加粗的那句是问题，单独占一行、前面一个小标记，问答一眼分得开。
  只认"段落的第一个元素就是加粗"的情况，正文中间的加粗不受影响
*/
.faq .home-section__body :deep(p > strong:first-child) {
  display: block;
  margin-bottom: 4px;
  color: #fff;
  font-size: 1.04em;
}

.faq .home-section__body :deep(p > strong:first-child)::before {
  content: 'Q';
  display: inline-grid;
  place-items: center;
  width: 20px;
  height: 20px;
  margin-right: 8px;
  border-radius: 6px;
  background: rgba(122, 156, 255, 0.18);
  color: #b8caff;
  font-size: 12px;
  font-weight: 700;
  vertical-align: 2px;
}

.home-section__title {
  margin: 0 0 18px;
  font-size: clamp(22px, 2.6vw, 28px);
  font-weight: 700;
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
@media (hover: hover) and (pointer: fine) {
  .home-section__more:hover {
    gap: 10px;
    text-decoration-color: currentColor;
  }
}

@media (prefers-reduced-motion: reduce) {
  .home-section__more {
    transition: none;
  }
}
</style>
