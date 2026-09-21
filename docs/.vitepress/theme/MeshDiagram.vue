<script setup lang="ts">
// 架构示意图。颜色全部走 VitePress 自带的主题变量，明暗两套自动适配。
//
// 动画不是装饰，它在表达机制：
//   控制信令虚线持续流动  → 控制面一直在下发信令
//   直连线上跑快脉冲      → 这是活跃的首选路径
//   回退线慢漂移且更淡    → 这是备选，不是首选
</script>

<template>
  <figure class="mesh-diagram">
    <svg viewBox="0 0 760 400" role="img" aria-labelledby="mesh-diagram-title">
      <title id="mesh-diagram-title">
        控制面负责发现、认证与穿透协调；数据面由 WireGuard 承载，优先直连，失败时经中继转发已加密报文。
      </title>

      <!-- 控制面 -->
      <g class="g-plane">
        <rect x="180" y="16" width="400" height="62" rx="10" class="plane" />
        <text x="380" y="41" class="t-title">控制面（自研）</text>
        <text x="380" y="62" class="t-sub">身份 · 发现 · 穿透协调 · 选路 · ACL</text>
      </g>

      <!-- 控制信令 -->
      <g class="g-signal">
        <path d="M 250 78 L 150 150" class="signal" />
        <path d="M 510 78 L 610 150" class="signal" />
        <text x="168" y="118" class="t-edge">控制信令</text>
        <text x="592" y="118" class="t-edge" text-anchor="end">控制信令</text>
      </g>

      <!-- 两个节点 -->
      <g class="g-node">
        <rect x="40" y="150" width="220" height="104" rx="10" class="node" />
        <text x="150" y="176" class="t-node">Node A</text>
        <rect x="60" y="188" width="180" height="48" rx="7" class="inner" />
        <text x="150" y="207" class="t-inner">数据面 · boringtun</text>
        <text x="150" y="225" class="t-inner dim">wintun 虚拟网卡</text>
      </g>

      <g class="g-node">
        <rect x="500" y="150" width="220" height="104" rx="10" class="node" />
        <text x="610" y="176" class="t-node">Node B</text>
        <rect x="520" y="188" width="180" height="48" rx="7" class="inner" />
        <text x="610" y="207" class="t-inner">数据面 · boringtun</text>
        <text x="610" y="225" class="t-inner dim">wintun 虚拟网卡</text>
      </g>

      <!-- P2P 直连。路径全长 240，脉冲的 dasharray 按这个长度配 -->
      <g class="g-direct">
        <path d="M 260 202 L 500 202" class="direct" />
        <path d="M 260 202 L 500 202" class="direct-pulse" />
        <text x="380" y="188" class="t-edge strong">P2P 直连（打洞成功）</text>
      </g>

      <!-- Relay 回退 -->
      <g class="g-relay">
        <rect x="310" y="310" width="140" height="54" rx="10" class="relay" />
        <text x="380" y="333" class="t-title sm">Relay</text>
        <text x="380" y="352" class="t-sub sm">转发密文</text>

        <path d="M 150 254 L 150 292 Q 150 310 172 310 L 310 332" class="fallback" />
        <path d="M 610 254 L 610 292 Q 610 310 588 310 L 450 332" class="fallback" />
        <text x="380" y="292" class="t-edge">打洞失败时回退</text>
      </g>
    </svg>
    <figcaption>
      中继只转发已经加密的 WireGuard 报文，看不到明文 —— 它不是信任节点。
    </figcaption>
  </figure>
</template>

<style scoped>
.mesh-diagram {
  margin: 28px 0;
}
.mesh-diagram svg {
  display: block;
  width: 100%;
  max-width: 100%;
  height: auto;
}
figcaption {
  margin-top: 10px;
  font-size: 13.5px;
  color: var(--vp-c-text-2);
  text-align: center;
}

.plane {
  fill: var(--vp-c-bg-soft);
  stroke: var(--vp-c-brand-1);
  stroke-width: 1.5;
}
.node {
  fill: var(--vp-c-bg-soft);
  stroke: var(--vp-c-divider);
  stroke-width: 1.5;
}
.inner {
  fill: var(--vp-c-bg);
  stroke: var(--vp-c-divider);
  stroke-width: 1;
}
.relay {
  fill: var(--vp-c-bg-soft);
  stroke: var(--vp-c-divider);
  stroke-width: 1.5;
  stroke-dasharray: 5 4;
}

.signal {
  fill: none;
  stroke: var(--vp-c-brand-1);
  stroke-width: 1.4;
  stroke-dasharray: 4 4;
}
.direct {
  fill: none;
  stroke: var(--vp-c-brand-1);
  stroke-width: 2.4;
}
.fallback {
  fill: none;
  stroke: var(--vp-c-text-3);
  stroke-width: 1.6;
  stroke-dasharray: 6 5;
}

/* 直连线上来回跑的高亮脉冲：16 亮 + 224 空 = 240，正好是路径全长 */
.direct-pulse {
  fill: none;
  stroke: var(--vp-c-brand-1);
  stroke-width: 4;
  stroke-linecap: round;
  stroke-dasharray: 16 224;
  opacity: 0.85;
}

text {
  font-family: var(--vp-font-family-base);
  text-anchor: middle;
  fill: var(--vp-c-text-1);
}
.t-title { font-size: 15px; font-weight: 600; }
.t-title.sm { font-size: 13.5px; }
.t-sub { font-size: 12px; fill: var(--vp-c-text-2); }
.t-sub.sm { font-size: 11.5px; }
.t-node { font-size: 14px; font-weight: 600; }
.t-inner { font-size: 11.5px; fill: var(--vp-c-text-2); }
.t-inner.dim { fill: var(--vp-c-text-3); }
.t-edge { font-size: 11.5px; fill: var(--vp-c-text-3); }
.t-edge.strong { fill: var(--vp-c-brand-1); font-weight: 500; }

/* ---- 持续流动：表达"一直在发生" ---------------------------------- */

/* 一个完整的 dash 周期 = 4 + 4 = 8，跑满一周期才能无缝循环 */
@keyframes mesh-flow {
  to { stroke-dashoffset: -8; }
}
/* 回退线的周期 = 6 + 5 = 11 */
@keyframes mesh-flow-slow {
  to { stroke-dashoffset: -11; }
}
@keyframes mesh-pulse {
  from { stroke-dashoffset: 240; }
  to { stroke-dashoffset: 0; }
}

.signal {
  animation: mesh-flow 1.1s linear infinite;
}
.direct-pulse {
  animation: mesh-pulse 2.2s linear infinite;
}
/* 更慢、更淡 —— 这是回退路径，不该和直连抢注意力 */
.fallback {
  animation: mesh-flow-slow 3.4s linear infinite;
  opacity: 0.72;
}

/* ---- 进视口时按建连顺序依次浮现 ---------------------------------- */
/*
  由 reveal.ts 给 figure 加上 .reveal-in 触发。JS 没跑就不会触发，
  各部分保持自然的可见状态 —— 图仍然是完整的。
  只动 opacity：SVG 元素上的 transform 受 transform-origin 影响，容易出意外。
*/
@keyframes mesh-appear {
  from { opacity: 0; }
  to { opacity: 1; }
}

.mesh-diagram.reveal-in .g-plane,
.mesh-diagram.reveal-in .g-signal,
.mesh-diagram.reveal-in .g-node,
.mesh-diagram.reveal-in .g-direct,
.mesh-diagram.reveal-in .g-relay {
  animation: mesh-appear 420ms var(--meshora-ease, ease) both;
}

.mesh-diagram.reveal-in .g-plane { animation-delay: 60ms; }
.mesh-diagram.reveal-in .g-signal { animation-delay: 200ms; }
.mesh-diagram.reveal-in .g-node { animation-delay: 320ms; }
.mesh-diagram.reveal-in .g-direct { animation-delay: 460ms; }
.mesh-diagram.reveal-in .g-relay { animation-delay: 600ms; }

@media (prefers-reduced-motion: reduce) {
  .signal,
  .direct-pulse,
  .fallback {
    animation: none;
  }
  /* 脉冲停下来时别在线上留一截静止的粗线头 */
  .direct-pulse {
    display: none;
  }
}
</style>
