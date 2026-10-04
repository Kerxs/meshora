<script setup lang="ts">
import { withBase } from 'vitepress'

/** 首页收尾：读到底的人，最后再给一次下载的入口 */
</script>

<template>
  <section class="home-cta">
    <svg class="mark" viewBox="0 0 64 64" aria-hidden="true">
      <g stroke="rgba(255,255,255,.5)" stroke-width="3" stroke-linecap="round">
        <line x1="18" y1="20" x2="46" y2="17" />
        <line x1="18" y1="20" x2="20" y2="46" />
        <line x1="18" y1="20" x2="45" y2="44" />
        <line x1="46" y1="17" x2="20" y2="46" />
        <line x1="46" y1="17" x2="45" y2="44" />
        <line x1="20" y1="46" x2="45" y2="44" />
      </g>
      <circle cx="18" cy="20" r="6.5" fill="#fff" />
      <circle cx="46" cy="17" r="6.5" fill="#fff" />
      <circle cx="20" cy="46" r="6.5" fill="#fff" />
      <circle class="live" cx="45" cy="44" r="6.5" fill="#5ff0c0" />
    </svg>
    <h2 class="m-h2">叫上朋友，开一局</h2>
    <p>免费、开源，Windows 和安卓都有。装好、建网络、把网络码发出去。</p>
    <div class="actions">
      <a class="m-btn brand" href="https://github.com/Kerxs/meshora/releases/latest">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M12 4v11m0 0l-4.5-4.5M12 15l4.5-4.5M5 19.5h14" />
        </svg>
        下载 Meshora
      </a>
      <a class="m-btn" :href="withBase('/guide/what-is-meshora')">先看看文档</a>
    </div>
  </section>
</template>

<style scoped>
.home-cta {
  position: relative;
  display: grid;
  justify-items: center;
  margin: 56px 0 24px;
  padding: clamp(40px, 7vw, 72px) 20px;
  text-align: center;
  border-radius: 28px;
  overflow: hidden;
  border: 1px solid rgba(255, 255, 255, 0.1);
  background:
    radial-gradient(60% 80% at 50% 0%, rgba(61, 107, 255, 0.32), transparent 70%),
    radial-gradient(40% 60% at 80% 100%, rgba(127, 231, 220, 0.12), transparent 70%),
    rgba(14, 16, 24, 0.5);
}

/* 上沿一道细高光 */
.home-cta::before {
  content: '';
  position: absolute;
  top: 0;
  left: 20%;
  right: 20%;
  height: 1px;
  background: linear-gradient(90deg, transparent, rgba(255, 255, 255, 0.5), transparent);
}

.mark {
  width: 56px;
  height: 56px;
  margin-bottom: 18px;
  filter: drop-shadow(0 6px 20px rgba(61, 107, 255, 0.6));
}

.live {
  transform-box: fill-box;
  transform-origin: center;
  animation: live 2.6s ease-in-out infinite;
}

@keyframes live {
  50% { transform: scale(1.25); opacity: 0.75; }
}

p {
  max-width: 28em;
  margin: 12px 0 0;
  font-size: var(--m-fs-body);
  line-height: var(--m-lh-body);
  color: var(--m-c-body);
}

.actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: 12px;
  margin-top: 28px;
}

@media (max-width: 640px) {
  .home-cta {
    margin-top: 36px;
    border-radius: var(--m-radius-card);
  }

  .actions {
    display: grid;
    /* 显式一栏 1fr：不然宽屏那条 justify-content: center 会把隐式的栏缩成按钮内容宽 */
    grid-template-columns: minmax(0, 1fr);
    width: 100%;
    max-width: 340px;
  }

  .actions .m-btn {
    height: 50px;
  }
}

@media (prefers-reduced-motion: reduce) {
  .live {
    animation: none;
  }
}
</style>
