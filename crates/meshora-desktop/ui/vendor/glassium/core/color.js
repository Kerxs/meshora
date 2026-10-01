/**
 * sRGB 编码 ↔ 线性光。
 *
 * IEC 61966-2-1 的分段曲线，与着色器里的 srgbToLinear / linearToSrgb 是同一条公式（WGSL 在
 * shaders/srgb.wgsl.ts，GLSL 在 webgl2/shaders.ts）。CSS 颜色是 sRGB 编码的；线性光模式下交给 GPU
 * 之前先换成线性值。
 */
/** sRGB 编码值 → 线性光。负数按 0 算。 */
export function srgbToLinear(c) {
    const s = Math.max(0, c);
    return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
}
/** 线性光 → sRGB 编码值。负数按 0 算；大于 1 的照曲线外推（加性的高光可以超过 1，之后才钳）。 */
export function linearToSrgb(c) {
    const l = Math.max(0, c);
    return l <= 0.0031308 ? l * 12.92 : 1.055 * l ** (1 / 2.4) - 0.055;
}
/** 调用处校验：写错的值在这里就抛，而不是悄悄当成默认值。 */
export function assertBlendSpace(space) {
    if (space !== 'srgb' && space !== 'linear') {
        throw new TypeError(`[Glassium] blendSpace 只能是 'srgb' 或 'linear'，收到 ${JSON.stringify(space)}`);
    }
}
