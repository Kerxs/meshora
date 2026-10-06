/**
 * `glassium` 命名空间：runtime 的入口对象。`import glassium from 'glassium'`，浏览器里也挂在 `window.glassium`。
 *
 * ```js
 * import glassium from 'glassium'
 * glassium.configure({ quality: 'auto' })
 * glassium.glass(el, { preset: 'tinted' })
 * await glassium.ready
 * console.log(glassium.capabilities)   // { webgpu, webgl2, backdropFilter, tier, renderer, … }
 * glassium.debug.enable()
 * ```
 *
 * 具名导出（createGlassStage、组件类……）照旧都在，这里只是把 runtime 的几样收成一个对象。
 */
import { morphGlass } from "../interaction/morph.js";
import { currentStage, onStageChange, stageOrPending } from "../renderer/stage.js";
import { absorbedElements } from "./absorb.js";
import { startRuntime, whenScanned } from "./auto.js";
import { contentBlocks, contentStats } from "./content.js";
import { detectSync, detectWebGpu, tierOf } from "./capabilities.js";
import { configure, getConfig } from "./config.js";
import { glass, glassOf } from "./glass.js";
let syncCaps = null;
let webgpu = null;
let webgpuProbe = null;
function rendererOf(stage) {
    if (!stage)
        return getConfig().backend === 'css' ? 'css' : 'none';
    if (!stage.active)
        return 'css';
    return stage.backend === 'webgpu' ? 'webgpu' : stage.backend === 'webgl2' ? 'webgl2' : 'css';
}
function capabilities() {
    syncCaps ??= detectSync();
    const renderer = rendererOf(currentStage());
    const base = { ...syncCaps, webgpu, renderer };
    return { ...base, tier: tierOf(base) };
}
function probeWebGpu() {
    webgpuProbe ??= detectWebGpu().then((ok) => (webgpu = ok));
    return webgpuProbe;
}
/** WebGPU 查完、runtime 正在建的 stage 建完（或者失败）之后 resolve，给出完整的能力。 */
function ready() {
    // 先等 runtime 第一次扫描（import 之后马上读 ready 时它还没开始：那时还没有在建的 stage，renderer 会是 none）
    return whenScanned().then(() => {
        const pending = stageOrPending();
        // backend: 'css' 用不上 WebGPU：不去要适配器（手机上要一次也费时费电），webgpu 留 null
        const probe = getConfig().backend === 'css' ? Promise.resolve(false) : probeWebGpu();
        return Promise.all([probe, pending ? pending.catch(() => null) : Promise.resolve(null)]).then(() => capabilities());
    });
}
const debug = {
    /** 右下角的调试面板：后端、质量、帧时间、面板数……（只在调用时加载）。 */
    async enable() {
        const { enableDebugPanel } = await import("../debug/panel.js");
        enableDebugPanel();
    },
    async disable() {
        const { disableDebugPanel } = await import("../debug/panel.js");
        disableDebugPanel();
    },
    /** 收进场景的背景与内容块（内容块带着视频出了几帧、画了几次）。 */
    info() {
        return {
            backgrounds: absorbedElements(),
            content: contentBlocks().map((element) => ({ element, ...(contentStats(element) ?? { videoFrames: 0, paints: 0 }) }))
        };
    }
};
let readyPromise = null;
if (typeof window !== 'undefined') {
    // stage 换了（建好、重建）：ready 重新算
    onStageChange(() => {
        readyPromise = null;
    });
}
export const glassium = {
    glass,
    glassOf,
    morph: morphGlass,
    configure,
    get config() {
        return getConfig();
    },
    get capabilities() {
        return capabilities();
    },
    get ready() {
        readyPromise ??= ready();
        return readyPromise;
    },
    start: startRuntime,
    get stage() {
        return currentStage();
    },
    debug
};
