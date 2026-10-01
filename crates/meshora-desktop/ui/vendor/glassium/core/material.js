/**
 * 声明式材质立面 —— 降级到 pipeline.ts 的有序管线。
 *
 * 这一层保留了最初设想的那组参数名（blur / opacity / refraction / distortion /
 * highlight / saturation / tint / cornerRadius）。它们本身是好用的词汇，问题只在于
 * 不能拿它当**内核**：扁平属性包表达不了效果顺序，也表达不了采样余量协商
 * （上游 v1 正是这么设计的，作者后来整块删掉了，见 pipeline.ts 顶部）。
 *
 * 所以两层都要：这里一行写完常见情况，需要精确控制时直接构造 EffectChain。
 */
import { clampRadii } from "./optics.js";
import { resolveMargins } from "./pipeline.js";
/**
 * 默认值 —— 就是 regular 预设。
 *
 * 按 iOS 26 真机截图的实测定（docs/calibration.md「质感对照」）：比早先（与上游 playground 对齐的
 * refraction / distortion 0.2、白色 0.18、单面亮边）更透、更饱和，边缘折射更强，亮边一整圈。
 *
 * 导出是给要在默认值之上做调制的代码用的（`<glass-button>` 的按压动画要知道
 * 「没写 highlight 时 highlight 是多少」）。
 */
export const MATERIAL_DEFAULTS = Object.freeze({
    blur: 12,
    refraction: 0.25,
    distortion: 0.3,
    highlight: 0.9,
    dispersion: 0,
    saturation: 1.15,
    tint: 'rgba(255, 255, 255, 0.1)',
    opacity: 1,
    cornerRadius: '0.5frac',
    squircle: 2,
    depthEffect: 1,
    adaptive: 1,
    shadow: 0.35,
    magnify: 0,
    bodyLight: 0
});
/**
 * 预设。
 *
 * 厚度梯度照 Apple 的说法走：玻璃变厚时「投下更深更浓的阴影、透镜与折射更明显、
 * 光的散射更柔」。所以 thick 不只是模糊更大，折射、depthEffect 与投影也一起上去。
 *
 * clear 对应 Apple 的 Clear 变体：**没有自适应行为**（adaptive: 0）、更透，只该用在媒体内容上，
 * 而且需要调用方自己压一层遮罩来保证上面的内容可读。它不是「更淡的 regular」。
 * 其余预设都自适应（默认 adaptive: 1）：背后太亮或太暗时玻璃自己蒙一层纱。
 */
export const GlassPresets = {
    ultraThin: { blur: 3, refraction: 0.2, distortion: 0.24, saturation: 1.1, tint: 'rgba(255,255,255,0.05)', highlight: 0.7, depthEffect: 0.6, shadow: 0.2 },
    thin: { blur: 6, refraction: 0.22, distortion: 0.27, saturation: 1.12, tint: 'rgba(255,255,255,0.07)', highlight: 0.8, depthEffect: 0.8, shadow: 0.28 },
    regular: { blur: 12, refraction: 0.25, distortion: 0.3, saturation: 1.15, tint: 'rgba(255,255,255,0.1)', highlight: 0.9, depthEffect: 1, shadow: 0.35 },
    thick: { blur: 22, refraction: 0.3, distortion: 0.36, saturation: 1.2, tint: 'rgba(255,255,255,0.14)', highlight: 1, depthEffect: 1, shadow: 0.45 },
    clear: { blur: 0, refraction: 0.35, distortion: 0.5, saturation: 1.1, tint: 'rgba(255,255,255,0)', highlight: 1, dispersion: 0.08, depthEffect: 1, adaptive: 0, shadow: 0 }
};
const HEX = /^#([0-9a-f]{3,8})$/i;
const RGB_FN = /^rgba?\(([^)]+)\)$/i;
/**
 * CSS 颜色 → 预乘前的 [r, g, b, a]，各分量 0–1。
 *
 * 只支持 hex（3/4/6/8 位）和 rgb()/rgba()。**不支持** 具名颜色、hsl()、color()、
 * oklch() 等 —— 那需要一个完整的颜色库，而 tint 这个场景用不上。
 * 不支持的写法**抛错**，不静默当成黑色：一块本该发白的玻璃默默变暗，
 * 会被当成光学 bug 查上半天。
 */
export function parseTint(css) {
    const s = css.trim();
    const hex = HEX.exec(s);
    if (hex) {
        const h = hex[1];
        const expand = (c) => parseInt(c.length === 1 ? c + c : c, 16) / 255;
        if (h.length === 3 || h.length === 4) {
            const a = h.length === 4 ? expand(h[3]) : 1;
            return [expand(h[0]), expand(h[1]), expand(h[2]), a];
        }
        if (h.length === 6 || h.length === 8) {
            const a = h.length === 8 ? expand(h.slice(6, 8)) : 1;
            return [expand(h.slice(0, 2)), expand(h.slice(2, 4)), expand(h.slice(4, 6)), a];
        }
        throw new Error(`[Glassium] tint 的 hex 位数不合法：${css}（支持 3/4/6/8 位）`);
    }
    const fn = RGB_FN.exec(s);
    if (fn) {
        const parts = fn[1].split(/[,\s/]+/).filter(Boolean);
        if (parts.length < 3)
            throw new Error(`[Glassium] tint 的分量不足：${css}`);
        const chan = (t) => t.endsWith('%') ? parseFloat(t) / 100 : parseFloat(t) / 255;
        const alphaRaw = parts[3];
        const a = alphaRaw === undefined ? 1 : alphaRaw.endsWith('%') ? parseFloat(alphaRaw) / 100 : parseFloat(alphaRaw);
        const out = [chan(parts[0]), chan(parts[1]), chan(parts[2]), a];
        if (out.some((v) => !Number.isFinite(v)))
            throw new Error(`[Glassium] tint 解析失败：${css}`);
        return out;
    }
    throw new Error(`[Glassium] 无法解析 tint：${css}。只支持 hex（#rgb/#rgba/#rrggbb/#rrggbbaa）` +
        `与 rgb()/rgba()，不支持具名颜色、hsl()、oklch() 等。`);
}
/** 把 CornerRadius 解算成四角绝对 dp，并钳到几何允许的范围。 */
export function resolveCornerRadii(radius, size) {
    const minDimension = Math.min(size[0], size[1]);
    // 先判 number / string 再兜底到 Radii4。用 Array.isArray 先判会narrow 成 any[]，
    // 而 Radii4 是 readonly 元组，负分支里去不掉，反而把另外两支的类型搞坏。
    if (typeof radius === 'number') {
        return clampRadii([radius, radius, radius, radius], size);
    }
    if (typeof radius === 'string') {
        const frac = parseFloat(radius);
        if (!Number.isFinite(frac))
            throw new Error(`[Glassium] 无法解析 cornerRadius：${radius}`);
        const dp = (frac * minDimension) / 2;
        return clampRadii([dp, dp, dp, dp], size);
    }
    return clampRadii(radius, size);
}
/**
 * 立面 → 有序管线。
 *
 * 降级是**全序且顺序固定**的：colorFilter → blur → lens，永不重排。
 * 无操作的效果会被省略 —— 不是为了省那一点开销，而是为了让
 * `chain.effects` 读起来就是「这块玻璃实际做了什么」。
 */
export function lowerMaterial(material, size) {
    const m = { ...MATERIAL_DEFAULTS, ...material };
    const minDimension = Math.min(size[0], size[1]);
    const effects = [];
    const tint = parseTint(m.tint);
    const saturationIsNoop = m.saturation === 1;
    const tintIsNoop = tint[3] === 0;
    if (!saturationIsNoop || !tintIsNoop) {
        effects.push({ kind: 'colorFilter', saturation: m.saturation, tint });
    }
    if (m.blur > 0) {
        effects.push({ kind: 'blur', sigmaDp: m.blur });
    }
    // 两条缩放规则照抄上游 playground，这样两边的校准数值可以直接对比：
    //   heightDp = refractionHeightFrac × minDimension × 0.5
    //   amountDp = refractionAmountFrac × minDimension
    const heightDp = m.refraction * minDimension * 0.5;
    const amountDp = m.distortion * minDimension;
    if (heightDp > 0 && amountDp > 0) {
        effects.push({
            kind: 'lens',
            heightDp,
            amountDp,
            squircle: Math.max(m.squircle, 1),
            dispersion: m.dispersion,
            highlight: m.highlight,
            depthEffect: m.depthEffect
        });
    }
    return {
        cornerRadiiDp: resolveCornerRadii(m.cornerRadius, size),
        effects,
        paddingDp: resolveMargins(effects),
        opacity: Math.min(Math.max(m.opacity, 0), 1),
        adaptive: Math.min(Math.max(m.adaptive, 0), 1),
        shadow: Math.min(Math.max(m.shadow, 0), 1),
        magnify: Math.max(m.magnify, 0),
        bodyLight: Math.min(Math.max(m.bodyLight, 0), 1)
    };
}
/** 取预设并可选覆盖若干字段。`glass(GlassPresets.thick, { tint: '#0af3' })` */
export function glass(preset, overrides = {}) {
    return { ...preset, ...overrides };
}
