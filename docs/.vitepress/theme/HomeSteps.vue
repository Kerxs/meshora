<script setup lang="ts">
import { withBase } from 'vitepress'

/** 首页"三步联机"：每步一张客户端的截图（scripts/capture 录的） */
const steps = [
  {
    title: '建一个网络',
    text: '起个名字点"建网络"。默认放在官方服务器上，朋友在哪都连得进来；也能不用服务器，和朋友直连。',
    image: 'start-desktop',
    alt: '开始页：建一个网络、加入朋友的网络'
  },
  {
    title: '把网络码发给朋友',
    text: '网主页上一键复制网络码，发到聊天里。也能发只能用一次、或者 24 小时后作废的网络码。',
    image: 'admin-desktop',
    alt: '网主管理页：网络码、成员、邀请码'
  },
  {
    title: '朋友贴进来，就在一个局域网里了',
    text: '每个人多一个局域网地址，游戏里的房间列表直接看得见对方。',
    image: 'network-desktop',
    alt: '网络页：网状图上看得见每个人、走哪条路、延迟多少'
  }
]
</script>

<template>
  <section class="steps">
    <p class="eyebrow">三步联机</p>
    <h2>不用配端口，不用开服务器</h2>
    <ol>
      <li v-for="(step, i) in steps" :key="step.title" class="step">
        <span class="n">{{ i + 1 }}</span>
        <h3>{{ step.title }}</h3>
        <p>{{ step.text }}</p>
        <img :src="withBase(`/demo/${step.image}.webp`)" :alt="step.alt" loading="lazy" decoding="async" />
      </li>
    </ol>
  </section>
</template>

<style scoped>
.steps {
  padding: clamp(32px, 5vw, 64px) 0;
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
  color: #fff;
}

ol {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 20px;
  margin: 28px 0 0;
  padding: 0;
  list-style: none;
}

.step {
  display: flex;
  flex-direction: column;
  margin: 0;
  padding: 22px 22px 0;
  border-radius: 22px;
  overflow: hidden;
}

.n {
  display: grid;
  place-items: center;
  width: 30px;
  height: 30px;
  border-radius: 50%;
  background: #3d6bff;
  color: #fff;
  font-weight: 700;
  font-size: 14px;
}

h3 {
  margin: 14px 0 0;
  font-size: 18px;
  font-weight: 700;
  color: #fff;
}

.step p {
  margin: 8px 0 18px;
  font-size: 14.5px;
  line-height: 1.7;
  color: rgba(255, 255, 255, 0.72);
  flex: 1;
}

/* 截图只露出上半截，像从卡片底下探出来 */
.step img {
  display: block;
  width: 100%;
  aspect-ratio: 16 / 10;
  object-fit: cover;
  object-position: top left;
  border-radius: 12px 12px 0 0;
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-bottom: 0;
}

@media (max-width: 900px) {
  ol {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
