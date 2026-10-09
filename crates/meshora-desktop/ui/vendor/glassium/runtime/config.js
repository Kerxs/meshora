/**
 * `glassium.configure()`：runtime 的全局选项。
 *
 * 默认值就是零配置：自动发现 `[glass]`、后端自动挑、质量自适应、玻璃后面的背景自动收进场景。
 * configure 要在 runtime 启动之前调才对「启动」这件事生效（import 之后同步调就行 —— 启动放在微任务里）；
 * 质量、收背景这些随时改随时生效。
 */
export const DEFAULT_CONFIG = Object.freeze({
    auto: true,
    backend: 'auto',
    quality: 'auto',
    absorbBackgrounds: true,
    absorbForComponents: false,
    absorbContent: true,
    rememberQuality: true,
    memoryBudget: null,
    cssRefraction: true
});
let current = DEFAULT_CONFIG;
const listeners = new Set();
export function getConfig() {
    return current;
}
/** 合并进当前配置；非法值报一次、忽略。返回合并后的配置。 */
export function configure(options) {
    const previous = current;
    const next = { ...current };
    for (const [key, value] of Object.entries(options)) {
        if (!(key in DEFAULT_CONFIG)) {
            console.warn(`[Glassium] configure 不认识 ${key}，已忽略`);
            continue;
        }
        if (key === 'quality' && !validQuality(value)) {
            console.warn(`[Glassium] quality 只能是 'auto' | 'high' | 'medium' | 'low' 或 0–1 的数，收到 ${String(value)}`);
            continue;
        }
        if (key === 'memoryBudget' && value !== null && !(typeof value === 'number' && Number.isFinite(value) && value > 0)) {
            console.warn(`[Glassium] memoryBudget 只能是正的字节数或 null，收到 ${String(value)}`);
            continue;
        }
        if (key === 'backend' && value !== 'auto' && value !== 'webgpu' && value !== 'webgl2' && value !== 'css') {
            console.warn(`[Glassium] backend 只能是 'auto' | 'webgpu' | 'webgl2' | 'css'，收到 ${String(value)}`);
            continue;
        }
        next[key] = value;
    }
    current = Object.freeze(next);
    for (const l of listeners)
        l(current, previous);
    return current;
}
export function onConfigChange(listener) {
    listeners.add(listener);
    return () => listeners.delete(listener);
}
function validQuality(q) {
    return q === 'auto' || q === 'high' || q === 'medium' || q === 'low' || (typeof q === 'number' && q >= 0 && q <= 1);
}
/** 固定档的质量值（auto 返回 null）。 */
export function fixedQuality(q) {
    if (q === 'auto')
        return null;
    if (q === 'high')
        return 1;
    if (q === 'medium')
        return 0.7;
    if (q === 'low')
        return 0.4;
    return q;
}
/** 测试用：回到默认配置。 */
export function resetConfig() {
    current = DEFAULT_CONFIG;
}
