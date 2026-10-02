<script setup lang="ts">
import { withBase } from 'vitepress'
import DemoFrame from './DemoFrame.vue'

/**
 * 首页首屏：左边一句话说清楚是什么、下载；右边是客户端的录屏 —— 电脑窗口里"一起联机"，
 * 右下角压一部手机。窄屏上画面排到文字下面。
 *
 * 版本号和状态写在这里（发版后跟着改），说法和 README、路线图一致：刚发布，还没在真实环境里验证过。
 */
const VERSION = '1.0.3'
</script>

<template>
  <section class="hero">
    <div class="copy">
      <p class="status">
        <span class="pulse" />
        {{ VERSION }} 已发布 · Windows 和安卓 · 刚发布，还没在真实环境里验证过
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
        <a class="btn brand" href="https://github.com/Kerxs/meshora/releases/latest">下载</a>
        <a class="btn" :href="withBase('/guide/desktop')">怎么用</a>
        <a class="btn ghost" href="https://github.com/Kerxs/meshora">GitHub</a>
      </div>
      <p class="fine">免费、开源（MIT）。Windows 安装程序没有代码签名，SmartScreen 会提示"未知发布者"。</p>
    </div>

    <div class="stage">
      <DemoFrame class="desk" image="network-desktop" video="connect" alt="Meshora 桌面客户端：贴上网络码，连上之后朋友一个个出现在网状图上" />
      <DemoFrame class="phone" kind="phone" image="network-phone" video="phone" alt="Meshora 安卓客户端：同一个网络，同一套界面" />
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
  align-items: center;
  gap: 8px;
  margin: 0 0 22px;
  padding: 5px 13px 5px 11px;
  border: 1px solid rgba(255, 255, 255, 0.22);
  border-radius: 999px;
  font-size: 12.5px;
  color: rgba(255, 255, 255, 0.86);
  background: rgba(255, 255, 255, 0.06);
}

.pulse {
  flex: none;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: #7fe7dc;
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
  gap: 6px;
  letter-spacing: -0.01em;
}

.name {
  font-size: clamp(44px, 6vw, 68px);
  font-weight: 800;
  line-height: 1.05;
  color: #fff;
}

.line {
  font-size: clamp(26px, 3.4vw, 40px);
  font-weight: 700;
  line-height: 1.25;
  color: #cdd8ff;
  text-wrap: balance;
}

.tagline {
  margin: 20px 0 0;
  max-width: 34em;
  font-size: 16.5px;
  line-height: 1.75;
  color: rgba(255, 255, 255, 0.78);
}

.actions {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
  margin-top: 28px;
}

.btn {
  display: inline-flex;
  align-items: center;
  height: 44px;
  padding: 0 22px;
  border-radius: 999px;
  border: 1px solid rgba(255, 255, 255, 0.2);
  background: rgba(255, 255, 255, 0.07);
  color: #fff !important;
  font-weight: 600;
  text-decoration: none !important;
  transition: transform 200ms cubic-bezier(0.16, 1, 0.3, 1), background-color 200ms, border-color 200ms;
}

.btn:hover {
  background: rgba(255, 255, 255, 0.13);
  transform: translateY(-1px);
}

.btn:active {
  transform: scale(0.97);
}

.btn.brand {
  border-color: transparent;
  background: #3d6bff;
}

.btn.brand:hover {
  background: #5580ff;
}

.btn.ghost {
  background: transparent;
}

.fine {
  margin: 18px 0 0;
  font-size: 12.5px;
  color: rgba(255, 255, 255, 0.5);
}

/* 画面：电脑窗口在后，手机压在右下角 */
.stage {
  position: relative;
  padding: 0 7% 9% 0;
}

.phone {
  position: absolute;
  right: 0;
  bottom: 0;
  width: 26%;
}

@media (max-width: 900px) {
  .hero {
    grid-template-columns: minmax(0, 1fr);
  }
}

@media (prefers-reduced-motion: reduce) {
  .pulse {
    animation: none;
  }
}
</style>
