<script setup lang="ts">
/**
 * 使用场景。四条都是真实会遇到的情况，各自标注用到哪项能力 ——
 * 让访客能对号入座，而不是读一堆抽象名词。
 */
const SCENARIOS = [
  {
    title: '出差在外访问家里的 NAS',
    body: '笔记本和家里的 NAS 各自躲在 NAT 后面。打洞成功就直连，失败就走中继，换酒店 Wi-Fi 也不用重新配。',
    caps: ['P2P', 'Relay']
  },
  {
    title: '多地办公室组网',
    body: '几个城市的设备分到同一个 overlay 网段，像插在同一台交换机上。需要统一出口时，指定一台有公网的机器当网关。',
    caps: ['Virtual LAN', 'Gateway']
  },
  {
    title: '和朋友玩只支持局域网的老游戏',
    body: '那些靠 UDP 广播找房间的游戏，难点不在带宽而在广播帧要真的被转发过去 —— 否则互相 ping 得通，房间列表里却看不见对方。',
    caps: ['Game Node', 'Virtual LAN']
  },
  {
    title: 'CGNAT 宽带下让设备互访',
    body: '运营商级 NAT 意味着你连自己的公网 IP 都没有，端口转发彻底失效。只能靠打洞，双方都是对称型 NAT 时由中继兜底。',
    caps: ['NAT 穿透', 'Relay']
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
  border-radius: 12px;
  border: 1px solid var(--meshora-glass-border);
  background: var(--meshora-glass-bg-soft);
  transition: border-color 0.18s, transform 0.18s;
}
.scenario:hover {
  border-color: var(--vp-c-brand-1);
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
  标签用白字不用品牌蓝：流体背景明暗一直在变，实测任何浅蓝在它的亮区
  都到不了小字 AA 要求的 4.5（最浅的 #d0e5fb 也只有 4.16）。
  白字对比度 8.12，辨识度靠边框和圆角维持。
*/
.cap {
  padding: 2px 9px;
  border-radius: 999px;
  border: 1px solid var(--meshora-glass-border);
  background: var(--meshora-glass-bg-soft);
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
