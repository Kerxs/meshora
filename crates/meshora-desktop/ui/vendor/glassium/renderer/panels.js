/**
 * 面板注册表：DOM 元素 → 每帧的几何 → uniform。
 *
 * 面板是 DOM 元素，它负责占位、承载文字与子元素、接收点击和焦点；
 * Glassium 只负责把玻璃画在它后面的画布上。两边的对齐靠每帧量一次
 * `getBoundingClientRect()` —— 滚动、缩放、布局变化都自动跟上，不需要监听任何事件。
 */
import { lowerMaterial } from "../core/material.js";
import { frostForColor } from "../core/transparency.js";
import { MAX_GROUP_MEMBERS, mergeBleed } from "../core/merge.js";
import { magnifyFactor } from "../core/optics.js";
import { GROUP_STRIDE_FLOATS } from "../shaders/glass-group.wgsl.js";
import { DEBUG_MODES, PANEL_STRIDE_FLOATS, PANEL_STRUCT_BYTES } from "../shaders/glass.wgsl.js";
import { levelForSigma } from "./blur.js";
import { parseFillPaint, resolvePaint } from "../core/gradient.js";
import { LabelAtlas, bitmapUv, rasterFit } from "./atlas.js";
import { fillRadii, parseFillColor, readFillStyle, scaleGradient } from "./fills.js";
import { packMask } from "./mask.js";
import { poseOf } from "./pose.js";
import { combineQuality, FULL_QUALITY } from "./quality.js";
import { CLIP_UNBOUNDED_PX, UNBOUNDED, NO_CLIP, roundClipOf, findClipEntries, flatParent, intersect, maskOf, packClipExtras, union } from "./clipping.js";
/** 盒子按呈现变换挪、缩（绕中心）。 */
export function presentRect(r, p) {
    const width = r.width * p.sx;
    const height = r.height * p.sy;
    const left = r.left + p.dx + (r.width - width) / 2;
    const top = r.top + p.dy + (r.height - height) / 2;
    return { left, top, width, height, right: left + width, bottom: top + height, x: left, y: top, toJSON: () => ({}) };
}
export { CLIP_UNBOUNDED_PX };
/**
 * 投影的形状随面板的短边（dp）定：高斯 σ、向下的偏移、四周往里缩的量，都是短边的比例、有上下限。
 * 深浅由材质的 shadow 定。
 *
 * 按 iOS 26 截图定（docs/calibration.md「质感对照」）：按住的滑块旋钮（短边约 30pt）的影子往下约 4pt、σ 约 2.5pt、
 * 两侧往里缩约 3pt，深 7–8 级 —— 只在玻璃正下方露出来，两侧没有。大面板按比例放大、封顶。
 * 颜色是玻璃背后的平均色压暗（着色器的 SHADOW_TINT）。
 */
export const SHADOW_SIGMA_FRAC = 0.09;
export const SHADOW_SIGMA_MIN_DP = 2;
export const SHADOW_SIGMA_MAX_DP = 8;
export const SHADOW_OFFSET_FRAC = 0.16;
export const SHADOW_OFFSET_MIN_DP = 3;
export const SHADOW_OFFSET_MAX_DP = 6;
export const SHADOW_INSET_FRAC = 0.1;
export const SHADOW_INSET_MAX_DP = 4;
/** shadow = 1 时影子最深处的不透明度（影子的颜色是背后平均色的一半，所以实际压暗约是它的一半）。 */
export const SHADOW_OPACITY = 0.3;
/** 影子伸出去多远还要画（按形状的上限算）：2.5σ 之外不到峰值的 5%。 */
const SHADOW_REACH_DP = 2.5 * SHADOW_SIGMA_MAX_DP + SHADOW_OFFSET_MAX_DP;
/** 短边 sideDp 的面板的投影形状，dp。 */
export function shadowShapeDp(sideDp) {
    const clamp = (v, lo, hi) => Math.min(Math.max(v, lo), hi);
    return {
        sigma: clamp(SHADOW_SIGMA_FRAC * sideDp, SHADOW_SIGMA_MIN_DP, SHADOW_SIGMA_MAX_DP),
        offset: clamp(SHADOW_OFFSET_FRAC * sideDp, SHADOW_OFFSET_MIN_DP, SHADOW_OFFSET_MAX_DP),
        inset: Math.min(SHADOW_INSET_FRAC * Math.max(sideDp, 0), SHADOW_INSET_MAX_DP)
    };
}
/** 光斑的高斯 σ 占面板短边的比例。 */
export const LIGHT_SIGMA_FRAC = 0.4;
/** strength = 1 时光斑中心加上的亮度（0–1，加性）。 */
export const LIGHT_GAIN = 0.2;
/**
 * 用 CSS 画的玻璃（core/overlay.ts）的标记。stage 在测量时给这些元素加上它、不画它们的 GPU 玻璃：
 * - 写了 `overlay` 属性的（盖在 DOM 内容上的玻璃：GPU 玻璃画在最底下，会被下面的内容盖住）；
 * - 在浏览器的「顶层」里的：打开的模态对话框、打开的 popover、全屏元素里面 —— 那里的元素画在一切之上；
 * - 玻璃祖先是这样的（它里面的玻璃与填充也得跟着用 CSS 画，否则画在底下的画布上被盖住）。
 * glassium.css 与各组件的影子样式按它换成 CSS 玻璃。
 */
export const OVERLAY_ATTRIBUTE = 'data-glassium-overlay';
/** 写了它的玻璃按 CSS 画（见 OVERLAY_ATTRIBUTE）。 */
export const OVERLAY_OPT_IN = 'overlay';
/**
 * 玻璃最多叠几层（层号 0 起）。写在一块玻璃里面的玻璃（卡片里的按钮、卡片里开关的旋钮）在它上面一层：
 * 画它之前先把画布上已经画好的那一块（下面那层的玻璃也在里面）重新采回场景目标、重建那一块的模糊链，
 * 于是它折射、模糊的是下面那层玻璃，而不是在下面那层上开一个洞。每多一层多一轮局部的重采样与模糊。
 * 更深的按最深的这一层画。
 */
export const MAX_GLASS_LAYER = 3;
/** 平滑半径的默认值，dp。并排两个按钮留 8dp 左右的缝时，默认就会连起来。 */
export const DEFAULT_SMOOTHING_DP = 20;
/**
 * 材质能不能降级。不能就在**调用处**抛。
 *
 * 不提前校验的话，写错的 tint 要等到帧循环里 lowerMaterial 才抛 —— 那里抛出的异常会让
 * 下一帧的 requestAnimationFrame 排不上，整个 stage 就此冻住，报错位置还离写错的地方很远。
 */
function assertLowerable(material) {
    lowerMaterial(material, [100, 100]);
}
/**
 * 决定面板 CSS 不透明度的那几层：面板自己，以及往上直到（不含）同时包含画布的祖先。
 * 共同祖先上的 opacity 同时作用在画布与面板上，两边一致，不用管。
 * 返回计算样式的活对象：每帧读它们的 opacity 就跟得上过渡与动画，找这一串只在样式代数变了时做。
 */
function opacityChainOf(element, canvas) {
    if (typeof getComputedStyle !== 'function')
        return []; // 没有 DOM（Node 里的单元测试）
    const out = [];
    for (let e = element; e; e = flatParent(e)) {
        if (canvas && e.contains(canvas))
            break;
        out.push(getComputedStyle(e));
    }
    return out;
}
/**
 * 文字深浅：按元素的计算颜色，与减少透明度时选磨砂同一个规则（core/transparency.ts）。
 * 没有 DOM 时（Node 里的单元测试）当作浅色文字 —— 与那边解析不了颜色时的约定一致。
 */
function toneOf(element) {
    if (typeof getComputedStyle !== 'function')
        return 1;
    return frostForColor(getComputedStyle(element).color) === 'dark' ? 1 : -1;
}
/**
 * 元素是不是真的画出来了。
 *
 * `visibility: hidden` 与 `opacity: 0`（自身或任一祖先）的元素照样有盒子，
 * getBoundingClientRect 量得到 —— 不跳过的话，DOM 已经看不见了，玻璃还留在原地。
 * 渐隐收起的菜单就是这样。部分透明（0 < opacity < 1）照画，玻璃乘上实际不透明度（见 fade）。
 */
export function isRendered(element) {
    if (typeof element.checkVisibility !== 'function')
        return true;
    return element.checkVisibility({ visibilityProperty: true, opacityProperty: true });
}
/**
 * 裁剪边界落在分数像素上时取最近的整数像素。DOM 在那条边上是精确裁的，
 * 往外取整会漏出一条玻璃，往里取整会少一条，四舍五入两边各差不到半个像素。
 */
function roundBox(b) {
    const r = (v) => (Number.isFinite(v) ? Math.round(v) : v);
    return { x0: r(b.x0), y0: r(b.y0), x1: r(b.x1), y1: r(b.y1) };
}
/**
 * 视觉缩放：getBoundingClientRect 量的是变换之后的盒子（width × height），offsetWidth / offsetHeight 是布局尺寸，
 * 两个轴的比取平均。offsetWidth 取整过，所以差不到 1% 时当作没有缩放（正好 1）；布局尺寸是 0 时也是 1。
 * morphGlass 量两头用的也是它：过渡玻璃结束时要与真正的面板对得上。
 */
export function visualScaleOf(width, height, layoutW, layoutH) {
    if (!(layoutW > 0 && layoutH > 0))
        return 1;
    const s = (width / layoutW + height / layoutH) / 2;
    return Math.abs(s - 1) > 0.01 ? s : 1;
}
/** 一块面板的质量系数：没有自己的就是全局那一组（同一个对象）；有就乘起来，按全局那一组的引用缓存。 */
function qualityOf(record, global) {
    const local = record.localQuality;
    if (!local)
        return global;
    if (record.combinedQuality?.global !== global) {
        record.combinedQuality = { global, value: Object.freeze(combineQuality(global, local)) };
    }
    return record.combinedQuality.value;
}
/** 抗锯齿需要在面板矩形外多画的像素。sd 的覆盖率过渡宽 1px，留 2px 足够。 */
const AA_MARGIN_PX = 2;
/**
 * 边缘高光的宽度，dp。
 *
 * 上游 Highlight 默认 0.5dp、再按宽度的一半模糊 —— 那基本就是抗锯齿那一个像素。
 * iOS 26 截图上亮边峰值在最外一个像素、往里约 1pt 衰减完（面板、圆按钮、选中块都是），这里取 1dp；
 * 但不窄于 RIM_MIN_PX 个设备像素，否则 DPR 1 的屏幕上只剩抗锯齿那半个像素，看不见。
 */
export const RIM_WIDTH_DP = 1;
export const RIM_MIN_PX = 1.5;
const readFillStyleFromDom = (record) => {
    if (typeof getComputedStyle !== 'function')
        return null;
    record.style ??= getComputedStyle(record.element);
    return readFillStyle(record.style);
};
export class PanelRegistry {
    #records = [];
    #groups = [];
    #fills = [];
    #onChange;
    #readFillStyle;
    /** 样式代数：DOM 或样式每变一次加一，从样式读出来的缓存（裁剪祖先、材质变换的 key）据此过期。 */
    #styleGeneration = 0;
    /** 树代数：DOM 变了、或者注册的玻璃变了（多一块少一块）就加一。玻璃祖先（层号）据此重找。 */
    #treeGeneration = 0;
    #filter = null;
    /** 自适应质量的系数（stage.setQuality）：量出来的每块面板都带着它。 */
    quality = FULL_QUALITY;
    /** 位图填充的图集：第一块位图填充画的时候才建（拿不到 2D 画布时是 null，位图填充就不画）。 */
    #atlas;
    constructor(onChange, options = {}) {
        this.#onChange = onChange;
        this.#readFillStyle = options.readFillStyle ?? readFillStyleFromDom;
    }
    get size() {
        return this.#records.length;
    }
    /** 注册过的填充数（含不在屏上的）。 */
    get fillCount() {
        return this.#fills.length;
    }
    /** 位图填充的图集（后端上传它）；还没有位图填充画过时是 null。 */
    get atlas() {
        return this.#atlas ?? null;
    }
    /**
     * 把一个元素注册成填充：它的盒子与 `--glass-fill` 颜色画进场景（见 fills.ts）。
     * 重复注册同一个元素返回同一块。
     */
    registerFill(element, options = {}) {
        let record = this.#fills.find((r) => r.element === element);
        if (!record) {
            record = { element };
            if (options.back)
                record.back = true;
            this.#insertFill(record);
            this.#onChange();
        }
        if (options.paint) {
            record.paintSource = options.paint;
            delete record.paintText;
        }
        const r = record;
        return {
            element,
            unregister: () => {
                const i = this.#fills.indexOf(r);
                if (i >= 0)
                    this.#fills.splice(i, 1);
                this.#onChange();
            }
        };
    }
    /**
     * 把一个元素注册成**位图**填充：它的盒子（圆角、变换、裁剪、不透明度与普通填充相同）里画 painter 画的内容 ——
     * 分段控件、标签栏把文字画进场景用它，玻璃就能折射、放大那些字。
     *
     * 内容按需画：只有看得见（不透明度 > 0、在屏上）的时候才画，画一次缓存在图集里；尺寸、缩放变了，或者
     * invalidate() 之后，下次看得见时重画。painter 在测量阶段调用，可以读布局（这一帧已经量过了，不会多一次重排）。
     */
    registerBitmapFill(element, painter, options = {}) {
        let record = this.#fills.find((r) => r.element === element);
        if (!record) {
            record = { element };
            if (options.back)
                record.back = true;
            this.#insertFill(record);
        }
        record.bitmap = {
            painter,
            anchor: options.anchor ?? null,
            oversample: options.oversample ?? 1,
            hole: options.hole ?? null,
            cell: null,
            pxW: 0,
            pxH: 0,
            scale: 0,
            dirty: true,
            version: 0,
            warned: false
        };
        this.#onChange();
        const r = record;
        return {
            element,
            invalidate: () => {
                if (!r.bitmap || r.bitmap.dirty)
                    return;
                r.bitmap.dirty = true;
                this.#onChange();
            },
            unregister: () => {
                const i = this.#fills.indexOf(r);
                if (i >= 0)
                    this.#fills.splice(i, 1);
                this.#onChange();
            }
        };
    }
    /** 新填充放进列表：背景层排在最后一个背景层后面，普通填充排在最后。 */
    #insertFill(record) {
        if (!record.back) {
            this.#fills.push(record);
            return;
        }
        let i = 0;
        while (i < this.#fills.length && this.#fills[i].back)
            i++;
        this.#fills.splice(i, 0, record);
    }
    /**
     * 位图填充这一帧的图集位置：格子没有、过期、尺寸或缩放变了就重新分配；内容过期就重画。
     * target 是画的那块（元素自己，或者锚点）：画布设备像素的原点与尺寸（转之前）、CSS 尺寸、一个 CSS 像素几个设备像素。
     * boxX / boxY 是填充盒子的原点：与 target 不同（有锚点）时，uv 的原点挪过去，只露出锚点画面里盒子盖住的那一块。
     */
    #bitmapOf(record, target, boxX, boxY) {
        const b = record.bitmap;
        const { w: deviceW, h: deviceH, cssW, cssH, k: scale } = target;
        if (!b || !(deviceW > 0 && deviceH > 0))
            return null;
        if (this.#atlas === undefined)
            this.#atlas = LabelAtlas.create();
        const atlas = this.#atlas;
        if (!atlas)
            return null;
        // 按设备像素画（与旁边的 DOM 一样锐利，oversample 更细）；比图集一格的上限还大就整体缩小
        const { fit, pxW, pxH } = rasterFit(deviceW, deviceH, atlas.maxCell, b.oversample);
        const rasterScale = scale * fit;
        if (!atlas.holds(b.cell) || pxW !== b.pxW || pxH !== b.pxH || rasterScale !== b.scale) {
            b.cell = atlas.allocate(pxW, pxH);
            b.pxW = pxW;
            b.pxH = pxH;
            b.scale = rasterScale;
            b.dirty = true;
        }
        const cell = b.cell;
        if (!cell)
            return null;
        if (b.dirty) {
            b.dirty = false;
            b.version++;
            try {
                atlas.draw(cell, rasterScale, (ctx) => b.painter(ctx, cssW, cssH));
            }
            catch (err) {
                if (!b.warned) {
                    b.warned = true;
                    console.warn('[Glassium] 位图填充画不出来，这一块留空：', record.element, err);
                }
            }
        }
        return {
            geom: bitmapUv(cell, target, boxX, boxY, fit, atlas.width, atlas.height),
            cell: [cell.x / atlas.width, cell.y / atlas.height, (cell.x + cell.w) / atlas.width, (cell.y + cell.h) / atlas.height],
            version: b.version
        };
    }
    register(element, material) {
        assertLowerable(material);
        const existing = this.#records.find((r) => r.element === element);
        if (existing) {
            console.warn('[Glassium] 这个元素已经注册过了，更新材质而不是重复注册：', element);
            existing.material = material;
            existing.cached = null;
            this.#onChange();
            return this.#handle(existing);
        }
        const record = { element, material, cached: null };
        this.#records.push(record);
        this.#treeGeneration++;
        this.#onChange();
        return this.#handle(record);
    }
    group(options = {}) {
        const record = {
            elements: [],
            smoothingDp: Math.max(options.smoothing ?? DEFAULT_SMOOTHING_DP, 0),
            warnedOverflow: false
        };
        this.#groups.push(record);
        this.#onChange();
        return {
            setMembers: (elements) => {
                record.elements = [...elements];
                this.#onChange();
            },
            setSmoothing: (dp) => {
                record.smoothingDp = Math.max(Number.isFinite(dp) ? dp : 0, 0);
                this.#onChange();
            },
            dissolve: () => {
                const i = this.#groups.indexOf(record);
                if (i >= 0)
                    this.#groups.splice(i, 1);
                this.#onChange();
            }
        };
    }
    get groupCount() {
        return this.#groups.length;
    }
    /**
     * DOM 或样式变了（stage 的 MutationObserver 调它）：从样式读出来的缓存作废 —— 裁剪祖先、
     * 材质变换的 key —— 下一帧重读。只是把代数加一，不在这里读任何样式：变化可能很频繁，
     * 重读推迟到真正要画的那一帧。
     */
    invalidateStyles() {
        this.#styleGeneration++;
        this.#treeGeneration++;
    }
    /** 换注册表级的材质变换（null 去掉）。所有面板下一帧重新降级。 */
    setMaterialFilter(filter) {
        if (filter === this.#filter)
            return;
        this.#filter = filter;
        for (const record of this.#records) {
            record.cached = null;
            delete record.filterKey;
            delete record.filterGeneration;
        }
        this.#onChange();
    }
    #handle(record) {
        return {
            element: record.element,
            setMaterial: (material) => {
                assertLowerable(material);
                record.material = material;
                record.cached = null;
                this.#onChange();
            },
            setLight: (light) => {
                const next = light && light.strength > 0 ? light : null;
                const prev = record.light ?? null;
                if (next === prev)
                    return;
                if (next && prev && next.x === prev.x && next.y === prev.y && next.strength === prev.strength)
                    return;
                record.light = next;
                this.#onChange();
            },
            setPresentation: (presentation) => {
                const prev = record.presentation ?? null;
                const identity = presentation && presentation.dx === 0 && presentation.dy === 0 && presentation.sx === 1 && presentation.sy === 1;
                const next = presentation && !identity ? { ...presentation } : null;
                if (next === prev)
                    return;
                if (next && prev && next.dx === prev.dx && next.dy === prev.dy && next.sx === prev.sx && next.sy === prev.sy)
                    return;
                record.presentation = next;
                this.#onChange();
            },
            setQuality: (factors) => {
                const next = factors && Object.keys(factors).length > 0 ? Object.freeze({ ...factors }) : null;
                const prev = record.localQuality ?? null;
                if (next === prev)
                    return;
                if (next && prev && JSON.stringify(next) === JSON.stringify(prev))
                    return;
                record.localQuality = next;
                delete record.combinedQuality;
                this.#onChange();
            },
            unregister: () => {
                const i = this.#records.indexOf(record);
                if (i >= 0)
                    this.#records.splice(i, 1);
                this.#treeGeneration++;
                this.#onChange();
            }
        };
    }
    /**
     * 量出本帧所有可见面板与合并组。
     *
     * **所有 getBoundingClientRect 在这里一次读完，帧内之后不再碰布局。**
     * 读写交错会触发强制同步布局（layout thrash），面板一多就是实打实的掉帧。
     */
    measure(viewport, originX = 0, originY = 0, canvas = null) {
        // CSS px → 画布设备像素。用合成目标尺寸除以 CSS 尺寸，而不是直接乘 dpr ——
        // 画布的像素数是取整过的，差那一点在 DPR 1.5 这类非整数倍率下会累积成可见的错位。
        const sx = viewport.compositeWidth / viewport.cssWidth;
        const sy = viewport.compositeHeight / viewport.cssHeight;
        const W = viewport.compositeWidth;
        const H = viewport.compositeHeight;
        const clip = (x0, y0, x1, y1) => {
            const cx0 = Math.max(0, Math.floor(x0));
            const cy0 = Math.max(0, Math.floor(y0));
            const cx1 = Math.min(W, Math.ceil(x1));
            const cy1 = Math.min(H, Math.ceil(y1));
            return [cx0, cy0, Math.max(0, cx1 - cx0), Math.max(0, cy1 - cy0)];
        };
        // 裁剪祖先的矩形这一帧只量一次，多块面板共用同一个滚动容器时不重复量
        const clipRects = new Map();
        const toDevice = (b) => ({
            x0: (b.x0 - originX) * sx,
            y0: (b.y0 - originY) * sy,
            x1: (b.x1 - originX) * sx,
            y1: (b.y1 - originY) * sy
        });
        const toDevicePoint = (x, y) => [(x - originX) * sx, (y - originY) * sy];
        const styleGeneration = this.#styleGeneration;
        // 一块面板或填充的几何：包围盒、（有旋转时）转之前的矩形、视觉缩放、裁剪、CSS 上的不透明度
        const geometry = (record) => {
            if (!record.element.isConnected)
                return null;
            if (!isRendered(record.element))
                return null;
            const raw = record.element.getBoundingClientRect();
            if (raw.width <= 0 || raw.height <= 0)
                return null;
            // 呈现变换（果冻、飞行）：当作元素的盒子变了 —— 下面与 CSS 的 scale 同一条路
            const presentation = record.presentation;
            const r = presentation ? presentRect(raw, presentation) : raw;
            // 相对画布原点。inset:0 的画布原点通常就是 (0,0)，但宿主不是 body 时未必。
            // 先按包围盒算；有旋转时下面再换成转之前的矩形
            const bounds = {
                x0: (r.left - originX) * sx,
                y0: (r.top - originY) * sy,
                x1: (r.left - originX + r.width) * sx,
                y1: (r.top - originY + r.height) * sy
            };
            let x = bounds.x0;
            let y = bounds.y0;
            let w = r.width * sx;
            let h = r.height * sy;
            // 视觉缩放（见 visualScaleOf）：没有缩放时仍按量到的尺寸降级，结果与之前逐位相同；
            // 真有缩放时按布局尺寸降级、打包时乘上缩放 —— 缩放动画期间也不必每帧重新降级。
            const layoutW = record.element.offsetWidth;
            const layoutH = record.element.offsetHeight;
            let visualScale = visualScaleOf(r.width, r.height, layoutW, layoutH);
            // 旋转：自己与祖先的变换合起来是「转过的矩形」时，包围盒的中心就是它的中心，尺寸是布局尺寸 × 缩放。
            // 读的是不透明度那一串的计算样式（活对象，同一批祖先），不多读样式。
            // 倾斜、3D 这类画不了的照旧按包围盒画，由 layering.ts 警告
            if (record.opacityStyles === undefined || record.opacityGeneration !== styleGeneration) {
                record.opacityStyles = opacityChainOf(record.element, canvas);
                record.opacityGeneration = styleGeneration;
            }
            const pose = poseOf(record.opacityStyles);
            let rotation = [1, 0];
            if (pose.supported && Math.abs(pose.angle) > 1e-6 && layoutW > 0 && layoutH > 0) {
                const cx = (bounds.x0 + bounds.x1) / 2;
                const cy = (bounds.y0 + bounds.y1) / 2;
                w = layoutW * pose.scaleX * sx;
                h = layoutH * pose.scaleY * sy;
                x = cx - w / 2;
                y = cy - h / 2;
                visualScale = Math.sqrt(pose.scaleX * pose.scaleY);
                rotation = [Math.cos(pose.angle), Math.sin(pose.angle)];
            }
            const rotated = rotation[1] !== 0;
            const cssW = visualScale === 1 && !rotated ? r.width : layoutW;
            const cssH = visualScale === 1 && !rotated ? r.height : layoutH;
            if (record.clips === undefined || record.clipGeneration !== styleGeneration) {
                record.clips = findClipEntries(record.element);
                record.clipGeneration = styleGeneration;
            }
            const visible = record.clips.length > 0 ? roundClipOf(record.clips, clipRects) : NO_CLIP;
            const clipBox = visible === NO_CLIP ? UNBOUNDED : toDevice(visible.box);
            // 圆角（两个半径相等）两个轴都按 sx 换算：仍是圆角，着色器走原来的算法（sx、sy 只差画布取整的那一点）
            const deviceCorners = (rx, ry) => {
                const x = rx.map((r) => r * sx);
                const y = ry.map((r, i) => (r === rx[i] ? x[i] : r * sy));
                return [x, y];
            };
            const [clipRadii, clipRadiiY] = deviceCorners(visible.rx, visible.ry);
            let clipShape = null;
            if (visible.shape) {
                const [rx, ry] = deviceCorners(visible.shape.rx, visible.shape.ry);
                clipShape = { box: toDevice(visible.shape.box), rx, ry };
            }
            const mask = record.clips.length > 0 ? maskOf(record.clips, clipRects, toDevicePoint, sx, sy) : null;
            let fade = 1;
            for (const s of record.opacityStyles) {
                const o = parseFloat(s.opacity);
                if (Number.isFinite(o))
                    fade *= o;
            }
            return { bounds, x, y, w, h, cssW, cssH, visualScale, rotation, clip: clipBox, clipRadii, clipRadiiY, clipShape, mask, fade };
        };
        // 层：最近的玻璃祖先（沿渲染树往上，自己不算）是谁，缓存到树代数变了为止；层号 = 玻璃祖先的层号 + 1
        const treeGeneration = this.#treeGeneration;
        let glassByElement = null;
        const glassParentOf = (record) => {
            if (record.glassParentGeneration !== treeGeneration) {
                glassByElement ??= new Map(this.#records.map((r) => [r.element, r]));
                let found = null;
                for (let e = flatParent(record.element); e; e = flatParent(e)) {
                    const hit = glassByElement.get(e);
                    if (hit) {
                        found = hit;
                        break;
                    }
                }
                record.glassParent = found;
                record.glassParentGeneration = treeGeneration;
            }
            return record.glassParent ?? null;
        };
        const layerOf = (record) => {
            let layer = 0;
            for (let p = glassParentOf(record); p && layer < MAX_GLASS_LAYER; p = glassParentOf(p))
                layer++;
            return layer;
        };
        // overlay（OVERLAY_ATTRIBUTE）：写了 overlay 属性、在顶层里（打开的模态对话框 / popover、全屏元素里）、
        // 或者玻璃祖先是 overlay。对话框 / popover 祖先按树代数缓存，开没开每帧看（开关它们不一定改 DOM）
        const fullscreen = typeof document !== 'undefined' ? document.fullscreenElement : null;
        const overlayMemo = new Map();
        const isOverlay = (record) => {
            const memo = overlayMemo.get(record);
            if (memo !== undefined)
                return memo;
            const el = record.element;
            let overlay = typeof el.hasAttribute === 'function' && el.hasAttribute(OVERLAY_OPT_IN);
            if (!overlay) {
                if (record.topAnchorGeneration !== treeGeneration) {
                    record.topAnchor = topLayerAnchor(el);
                    record.topAnchorGeneration = treeGeneration;
                }
                overlay = (record.topAnchor != null && isOpenInTopLayer(record.topAnchor)) || (fullscreen != null && flatContains(fullscreen, el));
            }
            if (!overlay) {
                const parent = glassParentOf(record);
                if (parent)
                    overlay = isOverlay(parent);
            }
            overlayMemo.set(record, overlay);
            return overlay;
        };
        // 包围盒外扩 reach、与裁剪祖先求交、钳到画布
        const scissorOf = (g, reach) => {
            const b = g.bounds;
            const own = intersect({ x0: b.x0 - reach, y0: b.y0 - reach, x1: b.x1 + reach, y1: b.y1 + reach }, roundBox(g.clip));
            return clip(own.x0, own.y0, own.x1, own.y1);
        };
        // 1) 每块画出来了的面板都量一遍。屏外的也量 —— 它可能是某个组的成员，
        //    自己不在屏上，与邻居连起来的颈部却在。
        const measured = new Map();
        for (const record of this.#records) {
            const overlay = isOverlay(record);
            markOverlay(record.element, overlay);
            if (overlay)
                continue; // 用 CSS 画（core/overlay.ts）
            const g = geometry(record);
            if (!g)
                continue;
            // 材质变换的 key 只在样式可能变了之后重读；变了才让降级缓存作废
            const filter = this.#filter;
            if (filter && record.filterGeneration !== styleGeneration) {
                const key = filter.key(record.element);
                if (key !== record.filterKey) {
                    record.filterKey = key;
                    record.cached = null;
                }
                record.filterGeneration = styleGeneration;
            }
            // 降级按尺寸缓存：材质的分数参数（refraction / distortion / 'frac' 圆角）
            // 是按短边算的，尺寸不变就不必重算。
            const cached = record.cached;
            let chain;
            if (cached && cached.w === g.cssW && cached.h === g.cssH) {
                chain = cached.chain;
            }
            else {
                const material = filter ? filter.apply(record.material, record.filterKey ?? '') : record.material;
                chain = lowerMaterial(material, [g.cssW, g.cssH]);
                record.cached = { w: g.cssW, h: g.cssH, chain };
            }
            // 有投影时 scissor 往外扩到影子够得着的地方
            const reach = AA_MARGIN_PX + (chain.shadow > 0 ? SHADOW_REACH_DP * sx * g.visualScale : 0);
            const scissor = scissorOf(g, reach);
            if (record.tone === undefined || record.toneGeneration !== styleGeneration) {
                record.tone = toneOf(record.element);
                record.toneGeneration = styleGeneration;
            }
            const l = record.light;
            const light = l
                ? // 光的位置相对元素的包围盒（组件用 getBoundingClientRect 量的），画在屏幕坐标里
                    [g.bounds.x0 + l.x * sx, g.bounds.y0 + l.y * sy, LIGHT_SIGMA_FRAC * Math.min(g.w, g.h), Math.min(1, Math.max(0, l.strength)) * LIGHT_GAIN]
                : [0, 0, 1, 0];
            measured.set(record, {
                record,
                quality: qualityOf(record, this.quality),
                x: g.x,
                y: g.y,
                w: g.w,
                h: g.h,
                scissor,
                clip: g.clip,
                clipRadii: g.clipRadii,
                clipRadiiY: g.clipRadiiY,
                clipShape: g.clipShape,
                mask: g.mask,
                light,
                fade: g.fade,
                tone: record.tone,
                visualScale: g.visualScale,
                rotation: g.rotation,
                bounds: g.bounds,
                chain,
                layer: layerOf(record)
            });
        }
        // 2) 合并组。一块面板只能属于一个组（先到先得），一组最多 MAX_GROUP_MEMBERS 块。
        const grouped = new Set();
        const groups = [];
        if (this.#groups.length > 0) {
            const byElement = new Map();
            for (const record of this.#records)
                byElement.set(record.element, record);
            for (const g of this.#groups) {
                const members = [];
                let overflow = 0;
                for (const element of g.elements) {
                    const record = byElement.get(element);
                    const m = record ? measured.get(record) : undefined;
                    if (!record || !m || grouped.has(record))
                        continue;
                    if (members.length >= MAX_GROUP_MEMBERS) {
                        overflow++;
                        continue;
                    }
                    members.push(m);
                    grouped.add(record);
                }
                if (overflow > 0 && !g.warnedOverflow) {
                    g.warnedOverflow = true;
                    console.warn(`[Glassium] 一组最多合并 ${MAX_GROUP_MEMBERS} 块玻璃，这一组有 ${members.length + overflow} 块。` +
                        `第 ${MAX_GROUP_MEMBERS + 1} 块起单独绘制，不参与合并。`);
                }
                if (members.length === 0)
                    continue;
                // smoothing 以 dp 计，跟着容器的视觉缩放走（取第一个成员的：同一个容器里的成员缩放相同）
                const k = g.smoothingDp * sx * members[0].visualScale;
                const shadowReach = members.some((m) => m.chain.shadow > 0)
                    ? SHADOW_REACH_DP * sx * Math.max(...members.map((m) => m.visualScale))
                    : 0;
                const bleed = mergeBleed(k) + AA_MARGIN_PX + shadowReach;
                let x0 = Infinity;
                let y0 = Infinity;
                let x1 = -Infinity;
                let y1 = -Infinity;
                // 成员各自的可见区域取并集：通常同在一个滚动容器里，那就是那个容器
                let visible = null;
                for (const m of members) {
                    x0 = Math.min(x0, m.bounds.x0);
                    y0 = Math.min(y0, m.bounds.y0);
                    x1 = Math.max(x1, m.bounds.x1);
                    y1 = Math.max(y1, m.bounds.y1);
                    visible = visible ? union(visible, m.clip) : m.clip;
                }
                const bounded = intersect({ x0: x0 - bleed, y0: y0 - bleed, x1: x1 + bleed, y1: y1 + bleed }, roundBox(visible ?? UNBOUNDED));
                const scissor = clip(bounded.x0, bounded.y0, bounded.x1, bounded.y1);
                if (scissor[2] === 0 || scissor[3] === 0)
                    continue; // 整组都在屏外
                groups.push({ members, smoothingPx: k, scissor, layer: Math.max(...members.map((m) => m.layer)) });
            }
        }
        // 3) 单独绘制的面板：不在组里、且裁剪矩形不为空（完全在屏外的不占 draw call）
        const panels = [];
        for (const m of measured.values()) {
            if (grouped.has(m.record))
                continue;
            if (m.scissor[2] === 0 || m.scissor[3] === 0)
                continue;
            panels.push(m);
        }
        // 4) 填充：几何与面板同一套，颜色每帧读（CSS 过渡要逐帧跟上），圆角、渐变按变换之前的尺寸解算。
        //    位图填充按需在图集里画（看得见才画）；画的过程中图集满了清空重排时，排在前面的位图填充的格子作废，
        //    量完再把它们补一遍（见下面）
        const fills = [];
        const atlasGeneration = this.#atlas?.generation;
        /** 这一帧量到的位图填充画的那块与盒子原点：图集清空过时拿它们补画。 */
        const bitmapArgs = new Map();
        /** 锚点元素（不是注册过的东西）：包围盒换到画布设备像素，CSS 尺寸取布局尺寸（变换之前）。 */
        const anchorTarget = (el) => {
            const r = el.getBoundingClientRect();
            const w = r.width * sx;
            const h = r.height * sy;
            const cssW = el.offsetWidth || r.width;
            const cssH = el.offsetHeight || r.height;
            if (!(w > 0 && h > 0 && cssW > 0 && cssH > 0))
                return null;
            return { x: (r.left - originX) * sx, y: (r.top - originY) * sy, w, h, cssW, cssH, k: w / cssW };
        };
        for (const record of this.#fills) {
            const overlay = isOverlay(record);
            markOverlay(record.element, overlay);
            if (overlay)
                continue; // 用 CSS 画背景
            const g = geometry(record);
            if (!g)
                continue;
            const scissor = scissorOf(g, AA_MARGIN_PX);
            if (scissor[2] === 0 || scissor[3] === 0)
                continue;
            const style = this.#readFillStyle(record);
            if (!style)
                continue;
            const k = sx * g.visualScale;
            let color;
            let gradient = null;
            let bitmap = null;
            if (record.bitmap) {
                if (!(g.fade > 0))
                    continue;
                const anchor = record.bitmap.anchor;
                const target = anchor ? anchorTarget(anchor) : { x: g.x, y: g.y, w: g.w, h: g.h, cssW: g.cssW, cssH: g.cssH, k };
                if (!target)
                    continue;
                bitmapArgs.set(record, [target, g.x, g.y]);
                bitmap = this.#bitmapOf(record, target, g.x, g.y);
                if (!bitmap)
                    continue;
                color = [0, 0, 0, g.fade];
            }
            else {
                const paint = fillPaintOf(record, style);
                if (!paint)
                    continue; // 解析不了：按透明处理（已警告）
                if (paint.kind === 'solid') {
                    const alpha = paint.color[3] * g.fade;
                    if (!(alpha > 0))
                        continue;
                    color = [paint.color[0], paint.color[1], paint.color[2], alpha];
                }
                else {
                    if (!(g.fade > 0))
                        continue;
                    color = [0, 0, 0, g.fade];
                    gradient = gradientOf(record, paint, g.cssW, g.cssH, k);
                }
            }
            const corners = fillRadii(style.radii, g.cssW, g.cssH);
            const scale = (r, cap) => {
                const [a, b, c, d] = r.map((v) => Math.min(v * k, cap));
                return [a, b, c, d];
            };
            fills.push({
                record,
                x: g.x,
                y: g.y,
                w: g.w,
                h: g.h,
                rotation: g.rotation,
                scissor,
                clip: g.clip,
                clipRadii: g.clipRadii,
                clipRadiiY: g.clipRadiiY,
                clipShape: g.clipShape,
                mask: g.mask,
                radii: scale(corners.x, g.w / 2),
                radiiY: scale(corners.y, g.h / 2),
                color,
                gradient,
                bitmap,
                layer: layerOf(record)
            });
        }
        // 图集在这一帧里清空过：之前量到的位图填充的格子作废，在新图集里重新分配、重画（还放不下的这一帧不画）
        const atlas = this.#atlas;
        if (atlas && atlasGeneration !== undefined && atlas.generation !== atlasGeneration) {
            for (let i = fills.length - 1; i >= 0; i--) {
                const f = fills[i];
                const b = f.record.bitmap;
                const args = bitmapArgs.get(f.record);
                if (!f.bitmap || !b || !args || atlas.holds(b.cell))
                    continue;
                const again = this.#bitmapOf(f.record, args[0], args[1], args[2]);
                if (again)
                    fills[i] = { ...f, bitmap: again };
                else
                    fills.splice(i, 1);
            }
        }
        // 洞：挖的那块这一帧也画了才挖（它看不见、或者图集满了这一帧没画，就不挖）
        if (fills.some((f) => f.record.bitmap?.hole)) {
            const byElement = new Map();
            for (const f of fills)
                byElement.set(f.record.element, f);
            for (let i = 0; i < fills.length; i++) {
                const f = fills[i];
                const holeElement = f.record.bitmap?.hole;
                const h = holeElement ? byElement.get(holeElement) : undefined;
                if (!h || !(h.color[3] > 0))
                    continue;
                const hole = {
                    shape: { box: { x0: h.x, y0: h.y, x1: h.x + h.w, y1: h.y + h.h }, rx: h.radii, ry: h.radiiY },
                    alpha: h.color[3]
                };
                fills[i] = { ...f, hole };
            }
        }
        return { panels, groups, fills };
    }
}
/** 最近的对话框或 popover 祖先（沿渲染树往上，含自己）。 */
function topLayerAnchor(el) {
    for (let e = el; e; e = flatParent(e)) {
        if (e.localName === 'dialog' || (typeof e.hasAttribute === 'function' && e.hasAttribute('popover')))
            return e;
    }
    return null;
}
/** 对话框以模态打开、popover 打开着 —— 在顶层里。不认识这些伪类的浏览器里当作没有。 */
function isOpenInTopLayer(anchor) {
    try {
        return anchor.matches(anchor.localName === 'dialog' ? ':modal' : ':popover-open');
    }
    catch {
        return false;
    }
}
/** outer 是不是 inner 自己或它渲染树上的祖先（跨影子树）。 */
function flatContains(outer, inner) {
    for (let e = inner; e; e = flatParent(e))
        if (e === outer)
            return true;
    return false;
}
/** 只在变了的时候改属性：这是每帧都走的路径。 */
function markOverlay(el, on) {
    if (typeof el.hasAttribute !== 'function')
        return; // 单元测试里的假元素
    if (el.hasAttribute(OVERLAY_ATTRIBUTE) !== on)
        el.toggleAttribute(OVERLAY_ATTRIBUTE, on);
}
/**
 * 填充的颜色或渐变：`--glass-fill` 的计算值（`currentcolor`，包括渐变色标里的，取元素的 color）。文本没变就用
 * 上一次解析的结果；解析不了（conic-gradient、写错的颜色）返回 null、按透明处理，只能近似的（颜色提示、色标
 * 太多）照画 —— 两种都只警告一次。
 */
function fillPaintOf(record, style) {
    let text = (record.paintSource ? record.paintSource() : style.color).trim();
    if (text.toLowerCase() === 'currentcolor')
        text = style.currentColor.trim();
    // 渐变里有 currentcolor 时，元素的 color 变了也要重新解析
    const key = /currentcolor/i.test(text) ? `${text}|${style.currentColor}` : text;
    if (record.paint !== undefined && key === record.paintText)
        return record.paint;
    const parsed = parseFillPaint(text, (css) => parseFillColor(css.toLowerCase() === 'currentcolor' ? style.currentColor : css));
    const problems = parsed ? parsed.warnings : [`解析不了（${text}），按透明处理`];
    if (problems.length > 0 && !record.warnedPaint) {
        record.warnedPaint = true;
        console.warn(`[Glassium] 填充的 --glass-fill：${problems.join('；')}：`, record.element);
    }
    record.paintText = key;
    record.paint = parsed?.paint ?? null;
    return record.paint;
}
/** 渐变解算到盒子上（画布设备像素）。渐变、尺寸、缩放都没变就还是上一次那个对象 —— idle.ts 按引用比。 */
function gradientOf(record, paint, width, height, scale) {
    const cached = record.gradient;
    if (cached && cached.paint === paint && cached.width === width && cached.height === height && cached.scale === scale) {
        return cached.value;
    }
    const value = scaleGradient(resolvePaint(paint, width, height), scale);
    record.gradient = { paint, width, height, scale, value };
    return value;
}
/**
 * 把一块面板写进 uniform 数组的第 index 个槽位（每槽 512B）。
 *
 * 字段顺序必须与 glass.wgsl.ts 的 `struct Panel` 逐一对应。这里写错一个偏移，
 * WebGPU 不会报任何错，你只会看到一块位置或形状微妙不对的玻璃 —— 所以两处的注释
 * 都写了字节偏移，改一处就去对另一处。
 */
export function packPanel(data, index, panel, viewport, blurLevels, debugMode) {
    writePanel(data, index * PANEL_STRIDE_FLOATS, panel, viewport, blurLevels, debugMode);
}
/** Panel 结构体占几个 float（416B / 4）。合并组里的成员按这个步长紧挨着排。 */
export const PANEL_STRUCT_FLOATS = PANEL_STRUCT_BYTES / 4;
/**
 * 把一个合并组写进 uniform 数组的第 index 个组槽位（每槽 1792B）。
 *
 * 布局必须与 glass-group.wgsl.ts 的 `struct Group` 一致：16B 的头
 * （成员数、k、调试模式、空）之后是 4 个紧挨着的 Panel。
 */
export function packGroup(data, index, group, viewport, blurLevels, debugMode) {
    const o = index * GROUP_STRIDE_FLOATS;
    data[o + 0] = group.members.length;
    data[o + 1] = group.smoothingPx;
    data[o + 2] = DEBUG_MODES.indexOf(debugMode);
    data[o + 3] = 0;
    for (let i = 0; i < MAX_GROUP_MEMBERS; i++) {
        const at = o + 4 + i * PANEL_STRUCT_FLOATS;
        const member = group.members[i];
        if (member)
            writePanel(data, at, member, viewport, blurLevels, debugMode);
        else
            data.fill(0, at, at + PANEL_STRUCT_FLOATS); // 不用的槽位清零，免得留着上一帧别的组的数
    }
}
function writePanel(data, o, panel, viewport, blurLevels, debugMode) {
    // dp（= CSS px）→ 画布设备像素，再乘视觉缩放（transform: scale）
    const scale = (viewport.compositeWidth / viewport.cssWidth) * panel.visualScale;
    const chain = panel.chain;
    const q = panel.quality ?? FULL_QUALITY;
    let saturation = 1;
    let tint = [0, 0, 0, 0];
    let sigmaDp = 0;
    let heightDp = 0;
    let amountDp = 0;
    let squircle = 2;
    let depthEffect = 0;
    let dispersion = 0;
    let highlight = 0;
    for (const e of chain.effects) {
        if (e.kind === 'colorFilter') {
            saturation = e.saturation;
            tint = e.tint;
        }
        else if (e.kind === 'blur') {
            sigmaDp = e.sigmaDp;
        }
        else {
            heightDp = e.heightDp;
            amountDp = e.amountDp;
            squircle = e.squircle;
            depthEffect = e.depthEffect;
            dispersion = e.dispersion;
            highlight = e.highlight;
        }
    }
    // rect: vec4f @ 0
    data[o + 0] = panel.x;
    data[o + 1] = panel.y;
    data[o + 2] = panel.w;
    data[o + 3] = panel.h;
    // radii: vec4f @ 16
    data[o + 4] = chain.cornerRadiiDp[0] * scale;
    data[o + 5] = chain.cornerRadiiDp[1] * scale;
    data[o + 6] = chain.cornerRadiiDp[2] * scale;
    data[o + 7] = chain.cornerRadiiDp[3] * scale;
    // tint: vec4f @ 32
    data[o + 8] = tint[0];
    data[o + 9] = tint[1];
    data[o + 10] = tint[2];
    data[o + 11] = tint[3];
    // 标量 @ 48 起
    data[o + 12] = heightDp * scale;
    data[o + 13] = amountDp * scale * q.refraction;
    // 模糊 σ 以**场景像素**计：模糊链的第 0 级就是场景分辨率，不是画布分辨率。
    data[o + 14] = levelForSigma(sigmaDp * q.blur * viewport.sceneScale * panel.visualScale, blurLevels);
    data[o + 15] = saturation;
    data[o + 16] = squircle;
    data[o + 17] = depthEffect * q.depth;
    data[o + 18] = dispersion * q.dispersion;
    data[o + 19] = highlight;
    data[o + 20] = chain.opacity * panel.fade; // 材质的 opacity × CSS 上的实际不透明度
    data[o + 21] = DEBUG_MODES.indexOf(debugMode);
    data[o + 22] = Math.max(RIM_WIDTH_DP * scale, RIM_MIN_PX);
    // adapt @ 92：自适应强度带上文字深浅的符号（> 0 浅色文字，< 0 深色文字，0 关掉）
    data[o + 23] = chain.adaptive * panel.tone;
    // clip: vec4f @ 96 —— 可见区域 x0, y0, x1, y1
    const bound = (v) => Math.max(-CLIP_UNBOUNDED_PX, Math.min(CLIP_UNBOUNDED_PX, v));
    data[o + 24] = bound(panel.clip.x0);
    data[o + 25] = bound(panel.clip.y0);
    data[o + 26] = bound(panel.clip.x1);
    data[o + 27] = bound(panel.clip.y1);
    // clipRadii: vec4f @ 112 —— TL, TR, BR, BL
    data[o + 28] = panel.clipRadii[0];
    data[o + 29] = panel.clipRadii[1];
    data[o + 30] = panel.clipRadii[2];
    data[o + 31] = panel.clipRadii[3];
    // light: vec4f @ 128 —— 中心 x、y，σ，强度
    data[o + 32] = panel.light[0];
    data[o + 33] = panel.light[1];
    data[o + 34] = panel.light[2];
    data[o + 35] = panel.light[3];
    // shadow: vec4f @ 144 —— 峰值 alpha、σ、向下的偏移、形状往里缩的量（画布设备像素）
    const shadowShape = shadowShapeDp(scale > 0 ? Math.min(panel.w, panel.h) / scale : 0);
    data[o + 36] = chain.shadow * SHADOW_OPACITY * q.shadow;
    data[o + 37] = shadowShape.sigma * scale;
    data[o + 38] = shadowShape.offset * scale;
    data[o + 39] = shadowShape.inset * scale;
    // pose: vec4f @ 160 —— 旋转的 cos θ、sin θ；放大系数 m / (1 + m)；1 ÷ 面板的高（体光用，着色器里不除以 uniform）
    data[o + 40] = panel.rotation[0];
    data[o + 41] = panel.rotation[1];
    data[o + 42] = magnifyFactor(chain.magnify);
    data[o + 43] = panel.h > 0 ? 1 / panel.h : 0;
    // clipRadiiY @ 176、clipInv @ 192；shapeBox @ 224、shapeRadii @ 240、shapeRadiiY @ 256、shapeInv @ 272
    packClipExtras(data, o + 44, o + 56, panel.clipRadii, panel.clipRadiiY, panel.clipShape);
    // 遮罩 @ 304：maskPaint、maskGeom、maskAlpha[2]、maskAt[2]、maskSpan
    packMask(data, o + 76, panel.mask);
    // extra @ 416：体光的强度；其余空
    data[o + 104] = chain.bodyLight;
    data[o + 105] = 0;
    data[o + 106] = 0;
    data[o + 107] = 0;
}
