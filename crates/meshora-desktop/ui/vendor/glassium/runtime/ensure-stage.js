/**
 * runtime 用到的 stage：有就沿用（自己调 createGlassStage 的页面、组件），没有就按配置建一个。
 * 建不出来（没有 GPU）时是 null —— 玻璃留在 CSS 兜底（`data-glassium-active` 挂不上）。
 */
import { createGlassStage, stageOrPending } from "../renderer/stage.js";
import { getConfig } from "./config.js";
import { attachQuality } from "./quality-link.js";
let stagePromise = null;
export function ensureStage() {
    if (typeof document === 'undefined')
        return Promise.resolve(null);
    const existing = stageOrPending();
    if (existing)
        return existing;
    if (!stagePromise) {
        stagePromise = createGlassStage({ backend: getConfig().backend }).then((stage) => {
            attachQuality(stage);
            return stage;
        }).catch((err) => {
            console.warn('[Glassium] 建不出 GPU stage，玻璃改用 CSS 画：', err);
            stagePromise = null;
            return null;
        });
    }
    return stagePromise;
}
