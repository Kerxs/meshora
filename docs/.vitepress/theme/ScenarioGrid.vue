<script setup lang="ts">
/**
 * 使用场景。1.0.0 只做局域网联机，所以四条都是联机时真会遇到的情况，
 * 各自标注靠的是哪一关 —— 让访客能对号入座，而不是读一堆抽象名词。
 *
 * 游戏例子只用查实过「靠广播 / 组播找房间」的，端口号只写查得最实的。
 */
const SCENARIOS = [
  {
    title: '和朋友打只支持局域网的老游戏',
    body: '魔兽争霸 III 的局域网游戏靠 UDP 6112 上的广播找房间。广播过不去，就会互相 ping 得通，房间列表里却看不见对方。',
    caps: ['广播转发', '虚拟局域网']
  },
  {
    title: '把 Minecraft 的局域网世界开给异地朋友',
    body: 'Java 版「对局域网开放」靠往组播地址 224.0.2.60:4445 发公告让别人看见。朋友不在同一个网里，就收不到这条公告。',
    caps: ['组播转发', '虚拟局域网']
  },
  {
    title: '宽带没有公网 IP',
    body: '运营商级 NAT 下端口转发根本配不了，开不了房间。只能靠打洞直连；两边都打不通时，由中继兜底。',
    caps: ['打洞直连', '中继']
  },
  {
    title: '打到一半换了网络',
    body: '笔记本从 Wi-Fi 切到手机热点，底层地址全变了。节点会重新探测、重新打洞，不用退出重新加入 —— 但中间会卡几秒，1.0.0 要把这段缩短。',
    caps: ['自动重连', '延迟优先']
  }
]
</script>

<template>
  <ul class="scenarios">
    <li v-for="s in SCENARIOS" :key="s.title" class="scenario">
      <h3>{{ s.title }}</h3>
      <p>{{ s.body }}</p>
      <p class="caps">
        <span v-for="c in s.caps" :key="c" class="cap">{{ c }}</span>
      </p>
    </li>
  </ul>
</template>

<style scoped>
.scenarios {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 16px;
  margin: 0;
  padding: 0;
  list-style: none;
}

@media (max-width: 720px) {
  .scenarios {
    grid-template-columns: minmax(0, 1fr);
  }
}

.scenario {
  display: flex;
  flex-direction: column;
  padding: 20px;
  /* 卡片是 Glassium 的玻璃（glass.ts） */
  border-radius: 12px;
  transition: transform 0.18s;
}
.scenario:hover {
  transform: translateY(-2px);
}

.scenario h3 {
  margin: 0 0 8px;
  font-size: 15.5px;
  font-weight: 600;
  line-height: 1.45;
  color: var(--meshora-on-glass);
  border: 0;
  padding: 0;
}

.scenario p {
  margin: 0;
  font-size: 14px;
  line-height: 25px;
  color: var(--meshora-on-glass-dim);
}

.caps {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 14px !important;
}

/*
  标签用白字不用品牌蓝：背景有明有暗（当初是流体背景，实测任何浅蓝在它的亮区
  都到不了小字 AA 要求的 4.5，最浅的 #d0e5fb 也只有 4.16）。
  白字对比度 8.12，辨识度靠边框和圆角维持。
*/
.cap {
  padding: 2px 9px;
  /* 标签是 Glassium 的玻璃（glass.ts） */
  border-radius: 999px;
  font-size: 11.5px;
  font-weight: 500;
  line-height: 1.7;
  white-space: nowrap;
  color: var(--meshora-on-glass);
}

@media (prefers-reduced-motion: reduce) {
  .scenario {
    transition: none;
  }
  .scenario:hover {
    transform: none;
  }
}
</style>
