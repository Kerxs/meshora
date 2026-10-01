export const FULL_QUALITY = Object.freeze({
    resolution: 1,
    blur: 1,
    refraction: 1,
    depth: 1,
    dispersion: 1,
    shadow: 1
});
/**
 * 全局的系数 × 一块玻璃自己的系数（GlassPanel.setQuality）。分辨率只看全局的 —— 场景是整页共用的一张。
 */
export function combineQuality(global, local) {
    return {
        resolution: global.resolution,
        blur: global.blur * (local.blur ?? 1),
        refraction: global.refraction * (local.refraction ?? 1),
        depth: global.depth * (local.depth ?? 1),
        dispersion: global.dispersion * (local.dispersion ?? 1),
        shadow: global.shadow * (local.shadow ?? 1)
    };
}
export function sameQuality(a, b) {
    return (a.resolution === b.resolution &&
        a.blur === b.blur &&
        a.refraction === b.refraction &&
        a.depth === b.depth &&
        a.dispersion === b.dispersion &&
        a.shadow === b.shadow);
}
