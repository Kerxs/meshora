/// <reference types="@webgpu/types" preserve="true" />
/**
 * `glassium/runtime`：只有 runtime —— `<div glass>`、`glass()`、`configure`、能力、自适应质量、内容进场景、果冻与飞行、
 * 变形、时间轴 —— **不注册 `<glass-*>` 组件**，也不带组件的代码。只用 `<div glass>` 的页面引它更小。
 *
 * ```js
 * import 'glassium/runtime'            // 零配置：页面上的 [glass] 自动变成玻璃
 * import { glass, configure } from 'glassium/runtime'
 * ```
 *
 * 要组件（`<glass-switch>`、`<glass-tab-bar>`……）就引完整的 `glassium`。两个入口可以同时用：它们共用同一套模块，
 * 引了完整入口之后组件照常注册。这里的导出是完整入口的子集（名字、行为都相同）。
 */
import { scheduleAutoStart } from "./runtime/auto.js";
import { glassium } from "./runtime/glassium.js";
export { VERSION } from "./version.js";
export { glassium };
export default glassium;
export { glass, glassOf } from "./runtime/glass.js";
export { configure } from "./runtime/config.js";
export { tierOf } from "./runtime/capabilities.js";
export { RUNTIME_PRESETS, runtimePreset } from "./runtime/presets.js";
export { startRuntime, stopRuntime } from "./runtime/auto.js";
export { absorbedElements } from "./runtime/absorb.js";
export { contentBlocks, contentStats } from "./runtime/content.js";
export { cancelFrame, everyFrame, nextFrame } from "./animation/timeline.js";
export { morphGlass } from "./interaction/morph.js";
export { createGlassStage, currentStage, onStageChange } from "./renderer/stage.js";
export { GlassPresets } from "./core/material.js";
if (typeof window !== 'undefined') {
    ;
    window.glassium ??= glassium;
    scheduleAutoStart();
}
