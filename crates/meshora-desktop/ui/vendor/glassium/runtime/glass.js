/**
 * `glass(element, options?)`：让任意元素变成一块玻璃。`<div glass>` 走的也是它（auto.ts 读属性、调它）。
 *
 * ```js
 * const handle = glass(el)                                   // Default Glass
 * glass(el, { preset: 'tinted' })
 * glass(el, { material: { blur: 20, refraction: 0.3 }, interaction: { press: true } })
 * glass(el, { interaction: { jelly: true, glide: true } })   // 元素动起来玻璃拉长；跳到别处时玻璃飞过去
 * glass(el, { quality: 0.6 })                                // 这一块固定降一点（别的照旧自适应）
 * handle.update({ preset: 'clear' })
 * handle.destroy()
 * ```
 *
 * 材质 = 预设（默认 default）⊕ 按元素算的圆角（CSS 的 border-radius，material 里写了 cornerRadius 就用它）⊕ material。
 * 交互默认按元素是否可交互（presets.ts 的 isInteractiveElement）；`interaction: false` 全关，`true` 全开。
 * 元素本身不被改写：语义、焦点、键盘、无障碍都照旧是它自己的 —— 玻璃只是画在它后面。
 *
 * 每个元素最多一块玻璃：再调一次等于 update。元素离开文档时玻璃跟着注销，回来时再注册（MutationObserver，见 auto.ts）。
 */
import { glass as mergeMaterial, MATERIAL_DEFAULTS } from "../core/material.js";
import { ElementMotion } from "../interaction/element-motion.js";
import { PressInteraction } from "../interaction/press.js";
import { factorsFor } from "../performance/quality.js";
import { GlassBinding } from "./binding.js";
import { scheduleAbsorb } from "./absorb.js";
import { ensureStage } from "./ensure-stage.js";
import { cornerRadiusFromCss, isInteractiveElement, runtimePreset, runtimePresetNames, RUNTIME_PRESETS } from "./presets.js";
import { getConfig } from "./config.js";
import { dropRefraction, syncRefraction } from "./refraction.js";
import { GLASS_ID_ATTRIBUTE, installRuntimeStyles, removeGlassVars, setGlassVars } from "./styles.js";
const handles = new WeakMap();
/** 活着的 runtime 玻璃（自适应质量的局部目标从这里取）。 */
const live = new Set();
let nextId = 1;
export function glass(target, options = {}) {
    if (!isElement(target))
        return mergeMaterial(target, options);
    const element = target;
    options = options;
    const existing = handles.get(element);
    if (existing && !existing.destroyed) {
        existing.update(options);
        return existing;
    }
    const g = new RuntimeGlass(element, options);
    handles.set(element, g);
    live.add(g);
    return g;
}
function isElement(x) {
    return typeof x === 'object' && x !== null && x.nodeType === 1;
}
/** 元素上的玻璃（没有是 null）。 */
export function glassOf(element) {
    const g = handles.get(element);
    return g && !g.destroyed ? g : null;
}
class RuntimeGlass {
    element;
    #id = String(nextId++);
    #options;
    #material = {};
    #binding;
    #press = null;
    #motion = null;
    #pinned = false;
    #wasPinned = false;
    destroyed = false;
    constructor(element, options) {
        this.element = element;
        this.#options = options;
        installRuntimeStyles();
        element.setAttribute(GLASS_ID_ATTRIBUTE, this.#id);
        this.#binding = new GlassBinding(element, {
            material: () => (this.#press ? this.#press.present(this.#material) : this.#material),
            light: () => this.#press?.light() ?? null
        });
        this.#resolve();
        this.#syncInteraction();
        this.#syncQuality();
        if (element.isConnected)
            this.#binding.connect();
        void ensureStage();
        scheduleAbsorb(true);
    }
    get panel() {
        return this.#binding.panel;
    }
    get material() {
        return this.#material;
    }
    update(options) {
        if (this.destroyed)
            return;
        const material = options.material ? { ...this.#options.material, ...options.material } : this.#options.material;
        this.#options = { ...this.#options, ...options, ...(material ? { material } : {}) };
        this.#resolve();
        this.#syncInteraction();
        this.#syncQuality();
        this.#binding.refresh();
    }
    /** 整个换掉选项（属性驱动的玻璃：属性删了的那一项要回到预设值，不能与旧的合并）。 */
    replace(options) {
        if (this.destroyed)
            return;
        this.#options = options;
        this.#resolve();
        this.#syncInteraction();
        this.#syncQuality();
        this.#binding.refresh();
    }
    /** 元素的样式可能变了（class、style、border-radius）：重算按元素的默认值。 */
    restyle() {
        if (this.destroyed)
            return;
        const before = JSON.stringify(this.#material.cornerRadius);
        this.#resolve();
        if (JSON.stringify(this.#material.cornerRadius) !== before)
            this.#binding.refresh();
    }
    /** 元素进 / 出文档（auto.ts 的 MutationObserver 告诉）。 */
    setConnected(connected) {
        if (this.destroyed)
            return;
        syncRefraction(this.element, this.#id, this.#material, connected);
        if (connected && !this.#binding.connected)
            this.#binding.connect();
        else if (!connected && this.#binding.connected) {
            this.#binding.disconnect();
            this.#press?.reset();
            this.#motion?.reset();
        }
        scheduleAbsorb(true);
    }
    destroy() {
        if (this.destroyed)
            return;
        this.destroyed = true;
        live.delete(this);
        this.#binding.disconnect();
        this.#press?.dispose();
        this.#press = null;
        this.#motion?.dispose();
        this.#motion = null;
        dropRefraction(this.#id);
        removeGlassVars(this.#id);
        this.element.removeAttribute(GLASS_ID_ATTRIBUTE);
        if (handles.get(this.element) === this)
            handles.delete(this.element);
        scheduleAbsorb(true);
    }
    #resolve() {
        const o = this.#options;
        let base = RUNTIME_PRESETS.default;
        if (o.preset !== undefined) {
            const p = runtimePreset(o.preset);
            if (p)
                base = p;
            else
                console.warn(`[Glassium] preset "${o.preset}" 不认识，可用：${runtimePresetNames().join(' / ')}；按 default`);
        }
        const radius = o.material?.cornerRadius === undefined && typeof getComputedStyle === 'function'
            ? { cornerRadius: cornerRadiusOf(this.element) }
            : {};
        this.#material = { ...base, ...radius, ...o.material };
        setGlassVars(this.#id, this.#material);
        syncRefraction(this.element, this.#id, this.#material, this.element.isConnected);
    }
    /** 固定的单块质量；'auto' 时由自适应质量（performance/adaptive.ts）通过 binding 设。 */
    #syncQuality() {
        const fixed = localFactors(this.#options.quality);
        this.#pinned = fixed !== null;
        if (fixed)
            this.#binding.setQuality(fixed);
        else if (this.#wasPinned)
            this.#binding.setQuality(null);
        this.#wasPinned = this.#pinned;
    }
    /** 这一块的质量是写死的（自适应质量不碰它）。 */
    get pinnedQuality() {
        return this.#pinned;
    }
    /** 自适应质量设的单块系数（写死的不理）。 */
    setLocalQuality(factors) {
        if (!this.#pinned && !this.destroyed)
            this.#binding.setQuality(factors);
    }
    /**
     * 成本估计（相对值）：视口里看得见的面积 × 模糊（σ 越大模糊链越深）× 色散（三次采样）。
     * 没有 GPU 计时，这只是估计；看不见的是 0。
     */
    cost() {
        if (!this.element.isConnected || !this.#binding.connected)
            return 0;
        const r = this.element.getBoundingClientRect();
        const vw = typeof innerWidth === 'number' ? innerWidth : r.right;
        const vh = typeof innerHeight === 'number' ? innerHeight : r.bottom;
        const w = Math.max(0, Math.min(r.right, vw) - Math.max(r.left, 0));
        const h = Math.max(0, Math.min(r.bottom, vh) - Math.max(r.top, 0));
        const m = this.#material;
        const blur = m.blur ?? MATERIAL_DEFAULTS.blur;
        const dispersion = m.dispersion ?? MATERIAL_DEFAULTS.dispersion;
        return w * h * (1 + blur / 16) * (dispersion > 0 ? 1.3 : 1);
    }
    #syncInteraction() {
        const i = this.#options.interaction;
        const auto = isInteractiveElement(this.element);
        // 果冻、飞行：跟着元素的位置走（element-motion.ts），与按压各管各的
        const motion = i === true ? { jelly: true, glide: true } : i === false ? { jelly: false, glide: false } : { jelly: i?.jelly ?? false, glide: i?.glide ?? false };
        // CSS 画的玻璃（backend: 'css'）没有 GPU 面板可推变换：不跑。它每帧都要量一次元素的位置，白量
        if ((motion.jelly || motion.glide) && getConfig().backend !== 'css') {
            if (this.#motion)
                this.#motion.setOptions(motion);
            else
                this.#motion = ElementMotion.forElement(this.element, (p) => this.#binding.setPresentation(p), motion);
        }
        else if (this.#motion) {
            this.#motion.dispose();
            this.#motion = null;
        }
        const want = i === false
            ? null
            : i === true
                ? { hover: true, press: true, focus: true }
                : {
                    hover: i?.hover ?? auto,
                    press: i?.press ?? auto,
                    focus: i?.focus ?? auto
                };
        const on = want !== null && (want.hover || want.press || want.focus);
        if (!on) {
            this.#press?.dispose();
            this.#press = null;
            return;
        }
        const options = {
            hover: want.hover,
            press: want.press,
            focus: want.focus,
            keys: true,
            isDisabled: () => this.element.matches(':disabled') || this.element.getAttribute('aria-disabled') === 'true'
        };
        if (this.#press)
            this.#press.setOptions(options);
        else
            this.#press = new PressInteraction(this.element, options, () => this.#binding.refresh(), () => this.#binding.refreshLight());
    }
}
/** 固定的单块质量（'auto' 与不写时交给自适应质量，这里不管）。 */
function localFactors(quality) {
    if (typeof quality !== 'number' || !Number.isFinite(quality))
        return null;
    const { resolution: _whole, ...rest } = factorsFor(Math.min(1, Math.max(0, quality)));
    return { resolution: 1, ...rest };
}
function cornerRadiusOf(el) {
    const s = getComputedStyle(el);
    return cornerRadiusFromCss([s.borderTopLeftRadius, s.borderTopRightRadius, s.borderBottomRightRadius, s.borderBottomLeftRadius]);
}
/** 能单独降质量的 runtime 玻璃（没写死 quality 的）。 */
export function adaptiveTargets() {
    return [...live].filter((g) => !g.destroyed && !g.pinnedQuality);
}
/** auto.ts 用：内部的类型（replace / restyle / setConnected）。 */
export function runtimeGlassOf(element) {
    const g = handles.get(element);
    return g && !g.destroyed ? g : null;
}
