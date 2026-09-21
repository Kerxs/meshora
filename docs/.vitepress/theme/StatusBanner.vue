<template>
  <p class="status-chip">
    <span class="pulse" />
    Pre-alpha · 设计阶段，暂无可用版本
  </p>
</template>

<style scoped>
.status-chip {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  margin: 0 0 24px;
  padding: 5px 13px 5px 11px;
  border: 1px solid rgba(255, 255, 255, 0.28);
  border-radius: 999px;
  font-size: 12.5px;
  font-weight: 500;
  letter-spacing: 0.01em;
  color: rgba(255, 255, 255, 0.88);
  background: rgba(255, 255, 255, 0.07);
}

.pulse {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: #7fe7dc;
  box-shadow: 0 0 0 0 rgba(127, 231, 220, 0.6);
  animation: pulse 2.4s ease-out infinite;
}

@keyframes pulse {
  0%   { box-shadow: 0 0 0 0 rgba(127, 231, 220, 0.55); }
  70%  { box-shadow: 0 0 0 7px rgba(127, 231, 220, 0); }
  100% { box-shadow: 0 0 0 0 rgba(127, 231, 220, 0); }
}

@media (prefers-reduced-motion: reduce) {
  .pulse { animation: none; }
}

@media (max-width: 639px) {
  .status-chip { margin-bottom: 16px; font-size: 11.5px; }
}
</style>
