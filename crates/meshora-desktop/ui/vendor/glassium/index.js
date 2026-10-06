/// <reference types="@webgpu/types" preserve="true" />
/**
 * Glassium 公开入口。
 *
 * 第一期按 T1→T12 逐步填充。每个 export 在其对应任务完成时加上，不提前占位 ——
 * 空壳 export 会让 playground 编译通过却在运行时崩，比缺失更难查。
 *
 * 现在有的：光学核心、有序效果管线与材质立面（T1–T4），WebGPU 渲染器（T5–T8），
 * `<glass-card>` / `<glass-button>` 组件与层级诊断（T9），`<glass-container>` 的合并（T10），
 * WebGL2 后端（T11）。后端阶梯：WebGPU → WebGL2 → CSS 兜底。第一期之后：用户场景（setScene）、
 * 裁剪、静止时不画、按压处的光、表单、自适应、四个系统设置。逐项说明见 docs/api.md。
 *
 * 在 Node 里 import 整个包是安全的（SSR）：模块顶层不碰任何浏览器全局，
 * defineGlassElements() 在没有 customElements 时什么都不做。
 */
export { VERSION } from "./version.js";
// —— Runtime 入口：`import 'glassium'` 之后页面上的 `<div glass>` 就是玻璃（runtime/auto.ts） ——
import { scheduleAutoStart, setElementRegistrar } from "./runtime/auto.js";
import { defineGlassElements } from "./components/register.js";
import { glassium } from "./runtime/glassium.js";
export { glassium };
export default glassium;
export { glass, glassOf } from "./runtime/glass.js";
export { configure } from "./runtime/config.js";
export { tierOf } from "./runtime/capabilities.js";
export { RUNTIME_PRESETS, cornerRadiusFromCss, isInteractiveElement, parseGlassAttributes, runtimePreset } from "./runtime/presets.js";
export { GlassBinding } from "./runtime/binding.js";
export { PressInteraction } from "./interaction/press.js";
export { ElementMotion, ELEMENT_FLY_SCALE, JUMP_PX, jellyScale2, noteScroll } from "./interaction/element-motion.js";
// —— 统一的时间轴：自己的动画也可以排在这里，与玻璃同一帧 ——
export { cancelFrame, everyFrame, flushFrame, nextFrame, pendingFrames, REDUCED_MOTION_SKIP } from "./animation/timeline.js";
export { startRuntime, stopRuntime } from "./runtime/auto.js";
export { absorbedElements } from "./runtime/absorb.js";
export { contentBlocks, contentStats } from "./runtime/content.js";
export { hitStacksBehind } from "./renderer/layering.js";
export { canvasIsClean, objectFitRect, videoIsClean } from "./components/scene-label.js";
export { AdaptiveQuality } from "./performance/adaptive.js";
export { QualityController, factorsFor, jellyFactor, QUALITY_MIN, allocateQuality, HEAVY_SHARE, LOCAL_SPAN } from "./performance/quality.js";
export { FrameMonitor } from "./performance/monitor.js";
export { FULL_QUALITY, combineQuality } from "./renderer/quality.js";
export { formatBytes, textureBytes } from "./renderer/resources.js";
export { passesFrom } from "./renderer/gpu-timer.js";
export { IDLE_LAYER_FRAMES } from "./renderer/backend.js";
export { inspectFrame } from "./renderer/inspect.js";
if (typeof window !== 'undefined') {
    ;
    window.glassium ??= glassium;
    // 完整入口：runtime 启动时顺带注册组件（`glassium/runtime` 入口不注册，见 runtime-entry.ts）
    setElementRegistrar(defineGlassElements);
    scheduleAutoStart();
}
// —— 光学核心（CPU 参考实现，与 WGSL 侧逐点一致）——
export { channelSampleOffsets, circleMap, clampRadii, gradRadiusOf, gradSdRoundedRect, bodyLight, magnifyFactor, radiusAt, refractionDirection, refractionProfile, rimLight, rimMask, safeNormalize, sdRoundedRect, smin, sminGradient, spectralWeights, squircleMap } from "./core/optics.js";
// —— 单位与分辨率 ——
export { MAX_PIXELS, MIN_SCENE_RATIO, cssToDevicePx, deviceToCssPx, describeViewport, dpToCssPx, resolveViewport, texelCenterUv, uvToTexelCoord } from "./core/units.js";
// —— 多块玻璃的合并（T10 起）——
export { MAX_GROUP_MEMBERS, evalMergedOptics, memberOptics, mergeBleed } from "./core/merge.js";
// —— 有序效果管线（内核）——
export { assertCanonicalOrder, resolveMargins, sampleMargin } from "./core/pipeline.js";
// —— 声明式材质立面 ——
export { GlassPresets, MATERIAL_DEFAULTS, lowerMaterial, parseTint, resolveCornerRadii } from "./core/material.js";
// —— 着色器源 ——
// 渲染器后端要用它们拼出完整着色器（绑定、入口点与 Y 翻转各后端手写）。
export { OPTICS_WGSL } from "./shaders/optics.wgsl.js";
export { OPTICS_GLSL } from "./shaders/generated/optics.glsl.js";
// —— 渲染器（T5 起）——
// stage.register() 把任意 DOM 元素注册成玻璃面板；组件（下面）就是在它之上的一层。
export { createGlassStage, currentStage, onStageChange, prefersReducedMotion, prefersMoreContrast, prefersReducedTransparency, simulateForcedColors, simulateMoreContrast, simulateReducedMotion, simulateReducedTransparency } from "./renderer/stage.js";
// 减少透明度时的材质变换（纯函数，别的渲染器也能用同一套规则）。
export { FROST, frostFor, frostForColor, REDUCED_TRANSPARENCY, reduceTransparency, relativeLuminance } from "./core/transparency.js";
// 填充的渐变：`--glass-fill` 的解析与按盒子解算的几何（纯函数，别的渲染器也能用同一套规则）。
export { MAX_GRADIENT_STOPS, parseFillPaint, resolvePaint, resolveStopOffsets } from "./core/gradient.js";
// 混合空间（createGlassStage 的 blendSpace）与 sRGB ↔ 线性光的转换（与着色器同一条公式）。
export { linearToSrgb, srgbToLinear } from "./core/color.js";
// 用户场景怎么铺进视口（object-fit 语义）。纯函数，别的渲染器也能用同一套算法。
export { sceneBitmapSize, sceneCssBackground, sceneUvTransform } from "./core/scene.js";
export { gl2CreationCounts } from "./webgl2/renderer.js";
export { deviceLossCount, gpuCreationCounts, simulateDeviceLoss, simulateNoWebGpu } from "./webgpu/device.js";
// —— 层级诊断（T9 起）——
// 面板与画布之间有东西挡着时点名警告。stage 自动触发，这里导出的是给调试读结果用的。
export { describeElement, describeProblem } from "./renderer/layering.js";
// —— 组件（T9 起）——
// 样式兜底在 src/components/glassium.css，要用 <link> 放进 <head>。
export { defineGlassElements } from "./components/register.js";
export { GlassElement } from "./components/base.js";
export { GlassCard } from "./components/glass-card.js";
export { GlassButton } from "./components/glass-button.js";
export { GlassContainer, MORPH_DROPLET, MORPH_EASING, MORPH_MS, dropletOffset } from "./components/glass-container.js";
// 两块不相干的玻璃之间的变形（glassEffectID 那种「这一块变成那一块」）
export { cubicBezier, morphGlass, MORPH_GLASS_EASE, MORPH_GLASS_FADE, MORPH_GLASS_MS } from "./interaction/morph.js";
export { GlassFill } from "./components/glass-fill.js";
export { GlassSwitch } from "./components/glass-switch.js";
export { GlassSegmented } from "./components/glass-segmented.js";
export { GlassTabBar, bubbleMaterial } from "./components/glass-tab-bar.js";
export { GlassBar, GlassNavBar, GlassToolbar, NAV_EDGE_RAMP, edgeProgress, inlineTitleOpacity, largeTitleProgress } from "./components/glass-nav-bar.js";
// 浮在正文上的玻璃：正文滚到底下时淡入的磨砂（scroll-edge 属性）
export { SCROLL_EDGE_RAMP, scrollEdgeProgress } from "./components/scroll-edge.js";
export { OVERLAY_HOST_CSS, overlayHostRule, overlayVars } from "./core/overlay.js";
export { Segments, segmentValue } from "./components/segments.js";
export { GlassSlider, defaultValue as sliderDefaultValue, parseRange as parseSliderRange, ratioOf as sliderRatio, snapValue as snapSliderValue } from "./components/glass-slider.js";
export { PressTween, SEGMENT_THUMB_PRESSED, THUMB_PRESSED, THUMB_REST, thumbMaterial } from "./components/thumb.js";
// 文字进场景：把元素里的文字、图标画进 2D 画布（位图填充的 painter），分段控件与标签栏按住时用
export { SceneLabels, paintContent } from "./components/scene-label.js";
export { MATERIAL_ATTRIBUTES, parseMaterialAttributes } from "./core/attributes.js";
// —— 玻璃面板（T7 起）与合并组（T10 起）——
export { DEFAULT_SMOOTHING_DP, LIGHT_GAIN, LIGHT_SIGMA_FRAC, MAX_GLASS_LAYER, OVERLAY_ATTRIBUTE, OVERLAY_OPT_IN, RIM_MIN_PX, RIM_WIDTH_DP, SHADOW_OPACITY, shadowShapeDp, presentRect } from "./renderer/panels.js";
// —— 填充：画进场景的纯色形状（`<glass-fill>`）——
export { FILL_PROPERTY, parseFillColor } from "./renderer/fills.js";
export { DEBUG_MODES } from "./shaders/glass.wgsl.js";
export { SECTORS, compareGroupOptics, compareOptics, joinProbeAndColors, sectorOf, summarizeBySector } from "./renderer/verify.js";
