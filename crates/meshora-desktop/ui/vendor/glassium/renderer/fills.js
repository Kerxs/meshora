/**
 * 填充（`<glass-fill>`）：CSS 摆位、Glassium 画进场景的圆角矩形，纯色、渐变或位图。
 *
 * 玻璃只折射场景（R2），DOM 的背景它看不见。开关的轨道、滑块的进度条、页面上的彩色渐变块这类
 * 「玻璃底下的形状」写成填充就进了场景：玻璃折射它、模糊它、按它的亮度调自适应。怎么画见 shaders/fill.wgsl.ts。
 *
 * 样式全从 CSS 来：
 * - 盒子（位置、尺寸、变换、裁剪、不透明度）与面板一样每帧量（panels.ts 共用同一段测量）；
 * - 颜色是自定义属性 `--glass-fill`，注册成不继承的 `<color> | <image>`（register.ts 与 glassium.css
 *   各注册一次）：纯色可以过渡，也可以写 `linear-gradient()` / `radial-gradient()`（core/gradient.ts）
 *   —— 每帧读计算值；
 * - 圆角是 CSS 的 border-radius。
 *
 * 元素自己的 CSS 背景在 stage 生效时是透明的（glassium.css），否则它会挡住后面的玻璃（R1）。
 */
import { MAX_GRADIENT_STOPS } from "../core/gradient.js";
import { parseTint } from "../core/material.js";
import { FILL_STRIDE_FLOATS } from "../shaders/fill.wgsl.js";
import { packMask } from "./mask.js";
import { CLIP_UNBOUNDED_PX, packClipExtras, packRoundedBox, parseCornerRadius, scaleRadii } from "./clipping.js";
/** 填充颜色的 CSS 自定义属性。 */
export const FILL_PROPERTY = '--glass-fill';
/**
 * 注册成 `<color> | <image>`：不继承（开关里的轨道不会把颜色传给里面的东西）、初始透明；纯色可以过渡，
 * 渐变（`<image>`）是离散的。
 */
export const FILL_PROPERTY_DEFINITION = {
    name: FILL_PROPERTY,
    syntax: '<color> | <image>',
    inherits: false,
    initialValue: 'transparent'
};
const TRANSPARENT = [0, 0, 0, 0];
/** 从活的计算样式里读。每帧调：颜色的过渡要逐帧跟上。 */
export function readFillStyle(style) {
    return {
        color: style.getPropertyValue(FILL_PROPERTY),
        currentColor: style.color,
        radii: [
            style.borderTopLeftRadius,
            style.borderTopRightRadius,
            style.borderBottomRightRadius,
            style.borderBottomLeftRadius
        ]
    };
}
let converter;
/** 1×1 的 2D 画布，把任何 CSS 颜色换算成 sRGB。惰性创建；没有 DOM 时是 null。 */
function colorConverter() {
    if (converter !== undefined)
        return converter;
    converter = null;
    try {
        if (typeof OffscreenCanvas !== 'undefined') {
            converter = new OffscreenCanvas(1, 1).getContext('2d', { willReadFrequently: true });
        }
        else if (typeof document !== 'undefined') {
            converter = document.createElement('canvas').getContext('2d', { willReadFrequently: true });
        }
    }
    catch {
        converter = null;
    }
    return converter;
}
/**
 * CSS 颜色 → 未预乘的 [r, g, b, a]。
 *
 * hex 与 `rgb()` / `rgba()` 直接解析（注册过的 `<color>` 属性，sRGB 颜色的计算值都是这种写法）；
 * 其余（`oklch()`、`color(display-p3 …)`、`color-mix()` 的结果、没注册时的具名颜色）交给浏览器：
 * 在 1×1 的画布上画一个点再读回来，浏览器负责换算与色域映射，结果是 8 位的。实测约 4µs 一次，
 * 而且只在颜色文本变了时才做。解析不了（写错了、没有 DOM）返回 null。
 */
export function parseFillColor(text) {
    const s = text.trim();
    if (s === '' || s === 'transparent')
        return TRANSPARENT;
    try {
        return parseTint(s);
    }
    catch {
        // 不是 hex / rgb()：往下交给浏览器
    }
    if (typeof CSS === 'undefined' || !CSS.supports('color', s))
        return null;
    const ctx = colorConverter();
    if (!ctx)
        return null;
    ctx.clearRect(0, 0, 1, 1);
    ctx.fillStyle = s;
    ctx.fillRect(0, 0, 1, 1);
    const d = ctx.getImageData(0, 0, 1, 1).data;
    return [d[0] / 255, d[1] / 255, d[2] / 255, d[3] / 255];
}
/**
 * 四角圆角（CSS 像素，变换之前）：百分比按盒子的宽、高解算；同一条边上相邻两角之和超过边长时一起缩小
 * （CSS 的规则，`border-radius: 999px` 的胶囊靠它变成半圆、`50%` 在长方形上是椭圆）；最后水平的钳到宽的一半、
 * 竖直的钳到高的一半 —— 着色器按象限取角，一个角伸过中线就不对了（CSS 允许单个角占满整条边，这里近似成半条）。
 */
export function fillRadii(radii, width, height) {
    const resolved = radii.map((css) => {
        const [x, y] = parseCornerRadius(css);
        return [x.percent ? (x.value / 100) * width : x.value, y.percent ? (y.value / 100) * height : y.value];
    });
    const scaled = scaleRadii(resolved, width, height);
    const along = (axis, cap) => {
        const [a, b, c, d] = scaled.map((r) => Math.max(0, Math.min(r[axis], cap)));
        return [a, b, c, d];
    };
    return { x: along(0, width / 2), y: along(1, height / 2) };
}
/**
 * 把一块填充写进 uniform 数组的第 index 个槽位（每槽 256B）。
 * 字段顺序必须与 fill.wgsl.ts 的 `struct Fill` 逐一对应（fills.test.ts 从 WGSL 源里解析出来核对）。
 */
export function packFill(data, index, fill) {
    const o = index * FILL_STRIDE_FLOATS;
    // rect @ 0
    data[o + 0] = fill.x;
    data[o + 1] = fill.y;
    data[o + 2] = fill.w;
    data[o + 3] = fill.h;
    // radii @ 16
    data[o + 4] = fill.radii[0];
    data[o + 5] = fill.radii[1];
    data[o + 6] = fill.radii[2];
    data[o + 7] = fill.radii[3];
    // color @ 32
    data[o + 8] = fill.color[0];
    data[o + 9] = fill.color[1];
    data[o + 10] = fill.color[2];
    data[o + 11] = fill.color[3];
    // clip @ 48：没有裁剪的方向写有限的 ±65536（着色器里 ∞ − ∞ 是 NaN）
    const bound = (v) => Math.max(-CLIP_UNBOUNDED_PX, Math.min(CLIP_UNBOUNDED_PX, v));
    data[o + 12] = bound(fill.clip.x0);
    data[o + 13] = bound(fill.clip.y0);
    data[o + 14] = bound(fill.clip.x1);
    data[o + 15] = bound(fill.clip.y1);
    // clipRadii @ 64
    data[o + 16] = fill.clipRadii[0];
    data[o + 17] = fill.clipRadii[1];
    data[o + 18] = fill.clipRadii[2];
    data[o + 19] = fill.clipRadii[3];
    // pose @ 80
    data[o + 20] = fill.rotation[0];
    data[o + 21] = fill.rotation[1];
    data[o + 22] = 0;
    data[o + 23] = 0;
    // 渐变 @ 96 起。纯色时种类写 0，其余清零（槽位是复用的，别留着上一帧别的填充的数）
    data.fill(0, o + 24, o + FILL_STRIDE_FLOATS);
    packCorners(data, o, fill);
    // clipRadiiY @ 304、clipInv @ 320；shapeBox @ 352、shapeRadii @ 368、shapeRadiiY @ 384、shapeInv @ 400
    packClipExtras(data, o + 76, o + 88, fill.clipRadii, fill.clipRadiiY, fill.clipShape);
    // 遮罩 @ 432
    packMask(data, o + 108, fill.mask);
    // 洞 @ 544：holeBox、holeRadii、holeRadiiY、holeInv，holeAlpha @ 624（没有洞时 0：逐位不变）
    const hole = fill.hole;
    if (hole && hole.alpha > 0) {
        packRoundedBox(data, o + 136, hole.shape);
        data[o + 156] = Math.min(hole.alpha, 1);
    }
    const bitmap = fill.bitmap;
    if (bitmap) {
        // paint @ 96：种类 3 是位图；geom @ 112：图集 uv 的原点与每个画布设备像素走多少 uv
        data[o + 24] = 3;
        data[o + 28] = bitmap.geom[0];
        data[o + 29] = bitmap.geom[1];
        data[o + 30] = bitmap.geom[2];
        data[o + 31] = bitmap.geom[3];
        // stops[0] @ 128：格子的范围（图集 uv），范围外按透明
        data[o + 32] = bitmap.cell[0];
        data[o + 33] = bitmap.cell[1];
        data[o + 34] = bitmap.cell[2];
        data[o + 35] = bitmap.cell[3];
        return;
    }
    const g = fill.gradient;
    if (!g)
        return;
    const n = Math.min(g.colors.length, MAX_GRADIENT_STOPS);
    // paint @ 96
    data[o + 24] = g.kind === 'linear' ? 1 : 2;
    data[o + 25] = n;
    data[o + 26] = g.repeating ? 1 : 0;
    // geom @ 112：着色器里不做除法，倒数在这里算
    const [a, b, c, d] = g.geometry;
    data[o + 28] = a;
    data[o + 29] = b;
    if (g.kind === 'linear') {
        const dx = c - a;
        const dy = d - b;
        const len2 = dx * dx + dy * dy;
        data[o + 30] = len2 > 0 ? dx / len2 : 0;
        data[o + 31] = len2 > 0 ? dy / len2 : 0;
    }
    else {
        data[o + 30] = 1 / Math.max(c, 1e-3);
        data[o + 31] = 1 / Math.max(d, 1e-3);
    }
    // stops @ 128
    for (let i = 0; i < n; i++)
        data.set(g.colors[i], o + 32 + i * 4);
    // at @ 208：位置；at[1].y、z 是重复的周期的倒数与周期
    const at = o + 32 + MAX_GRADIENT_STOPS * 4;
    for (let i = 0; i < n; i++)
        data[at + i] = g.offsets[i];
    const period = g.offsets[n - 1] - g.offsets[0];
    data[at + 5] = g.repeating && period > 0 ? 1 / period : 0;
    data[at + 6] = g.repeating && period > 0 ? period : 0;
    // span @ 240：相邻两个色标之间位置之差的倒数（重合的写 0，着色器里那一段是硬边）
    const span = at + 8;
    for (let i = 0; i + 1 < n; i++) {
        const gap = g.offsets[i + 1] - g.offsets[i];
        data[span + i] = gap > 0 ? 1 / gap : 0;
    }
}
/**
 * 椭圆角 @ 256 起：竖直半径与两组倒数（着色器里不做除法）。写在 packFill 的最后 —— 前面的布局一个字节都没挪。
 */
function packCorners(data, o, fill) {
    const inv = (r) => (r > 0 ? 1 / r : 0);
    for (let i = 0; i < 4; i++) {
        data[o + 64 + i] = fill.radiiY[i];
        data[o + 68 + i] = inv(fill.radii[i]);
        data[o + 72 + i] = inv(fill.radiiY[i]);
    }
}
/** 把解算好的渐变（CSS 像素）缩放到画布设备像素。 */
export function scaleGradient(paint, scale) {
    const [a, b, c, d] = paint.geometry;
    return { ...paint, geometry: [a * scale, b * scale, c * scale, d * scale] };
}
/**
 * 画布设备像素的裁剪矩形 → 场景目标像素：往外取整，与目标求交。空的返回 null。
 *
 * 画布上的裁剪矩形已经外扩了 2 个画布像素的抗锯齿余量；场景目标的像素不比画布的小
 * （像素预算只往下压），所以换算过去至少还有一个场景像素 —— 抗锯齿那半个像素够用。
 */
export function sceneScissor(scissor, sceneWidth, sceneHeight, compositeWidth, compositeHeight) {
    const kx = sceneWidth / compositeWidth;
    const ky = sceneHeight / compositeHeight;
    const x0 = Math.max(0, Math.floor(scissor[0] * kx));
    const y0 = Math.max(0, Math.floor(scissor[1] * ky));
    const x1 = Math.min(sceneWidth, Math.ceil((scissor[0] + scissor[2]) * kx));
    const y1 = Math.min(sceneHeight, Math.ceil((scissor[1] + scissor[3]) * ky));
    if (x1 <= x0 || y1 <= y0)
        return null;
    return [x0, y0, x1 - x0, y1 - y0];
}
/** 场景目标的 Dest：一个场景像素是几个画布设备像素，抗锯齿宽度取较大的那个轴。 */
export function sceneDest(sceneWidth, sceneHeight, compositeWidth, compositeHeight) {
    const sx = compositeWidth / sceneWidth;
    const sy = compositeHeight / sceneHeight;
    return [sx, sy, Math.max(sx, sy), 0];
}
/** 画布本身的 Dest：一比一。 */
export const CANVAS_DEST = [1, 1, 1, 0];
