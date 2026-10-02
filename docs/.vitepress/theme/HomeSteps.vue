<script setup lang="ts">
import { withBase } from 'vitepress'

/** 首页"三步联机"：每步一张客户端的截图（scripts/capture 录的） */
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
  justify-content: space-between;
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

/* 界面里相关的那一块（局部截图，原尺寸看得清），太高的露出上半截 */
.step img {
  display: block;
  width: 100%;
  max-height: 300px;
  object-fit: cover;
  object-position: top center;
  margin-bottom: 22px;
  border-radius: 14px;
  border: 1px solid rgba(255, 255, 255, 0.12);
  box-shadow: 0 16px 40px -16px rgba(0, 0, 0, 0.6);
}

@media (max-width: 900px) {
  ol {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
