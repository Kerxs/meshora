/**
 * 自适应质量：把帧监测（monitor.ts）、质量控制（quality.ts）、上次的结果（profile.ts）接到一个 stage 上 ——
 * stage 每圈报帧，攒满一个窗口就喂给控制器，q 变了就 `stage.setQuality(factorsFor(q))`。
 *
 * runtime 自己建的 stage 默认挂一个（configure({ quality: 'auto' })）；固定档（high / medium / low / 数）直接定死系数、
 * 不监测。自己调 createGlassStage 的页面不受影响（验证页、性能测试页都是满质量）。
 *
 * 后端不因为掉帧切换：掉帧只降质量；后端只在初始化失败、设备丢失重建失败时才换（stage 自己的逻辑）。
 */
import { FrameMonitor } from "./monitor.js";
import { loadProfile, profileKey, saveProfile } from "./profile.js";
import { allocateQuality, factorsFor, QualityController } from "./quality.js";
export class AdaptiveQuality {
    #stage;
    #monitor = new FrameMonitor();
    #controller;
    #fixed;
    #remember;
    #key;
    #unsubscribe = null;
    #lastWindow = null;
    #locals;
    /** 上一次单独降了的那几块（下一次不在里面的要放回去）。 */
    #lowered = new Set();
    #global = 1;
    constructor(stage, options = {}) {
        this.#stage = stage;
        this.#fixed = options.fixed ?? null;
        this.#remember = options.remember ?? false;
        this.#locals = options.locals ?? null;
        const vp = stage.debug.stats().viewport;
        this.#key =
            this.#remember && typeof window !== 'undefined'
                ? profileKey(options.version ?? '0', stage.backend, vp?.cssWidth ?? window.innerWidth, vp?.cssHeight ?? window.innerHeight, window.devicePixelRatio || 1)
                : null;
        const saved = this.#key ? loadProfile(this.#key) : null;
        // 有记下的结果就从它起步、不再探测
        this.#controller = new QualityController(saved?.q ?? options.initial ?? 1, !saved);
        if (options.listen ?? true)
            this.#unsubscribe = stage.onFrame((f) => this.feed(f));
        this.#apply();
    }
    /** 当前的质量值。 */
    get quality() {
        return this.#fixed ?? this.#controller.quality;
    }
    get fixed() {
        return this.#fixed;
    }
    get probing() {
        return this.#fixed === null && this.#controller.probing;
    }
    /** 刷新间隔的估计（ms）。 */
    get budgetMs() {
        return this.#monitor.refreshMs;
    }
    /** 整页那一档（局部质量先降贵的那几块时，它比 quality 高）。 */
    get globalQuality() {
        return this.#global;
    }
    /** 正在单独降的块数（调试面板读）。 */
    get loweredCount() {
        return this.#lowered.size;
    }
    /** 最近一个窗口（调试面板读）。 */
    get lastWindow() {
        return this.#lastWindow;
    }
    /** 定死质量（null 回到自适应）。 */
    setFixed(q) {
        this.#fixed = q === null ? null : Math.min(1, Math.max(0, q));
        this.#apply();
    }
    /** 喂一圈（stage.onFrame 的回调；测试直接调）。 */
    feed(frame) {
        if (this.#fixed !== null)
            return;
        const w = this.#monitor.frame(frame.time, frame.rendered, frame.cpuMs, frame.gpuMs ?? null);
        if (!w)
            return;
        this.#lastWindow = w;
        const before = this.#controller.quality;
        const after = this.#controller.sample(w);
        // 质量没变也重分一次：玻璃挪了、多了少了，谁贵谁便宜会变
        if (after !== before || (after < 1 && this.#locals))
            this.#apply();
        if (after !== before) {
            if (this.#key) {
                saveProfile(this.#key, { q: after, frameMs: this.#monitor.refreshMs, resolution: factorsFor(after).resolution, at: Date.now() });
            }
        }
    }
    dispose() {
        this.#unsubscribe?.();
        this.#unsubscribe = null;
        this.#stage.setQuality(null);
        for (const t of this.#lowered)
            t.setLocalQuality(null);
        this.#lowered.clear();
    }
    /** 按当前的 q 重新分配（整页 + 贵的那几块）。 */
    #apply() {
        const q = this.quality;
        const targets = this.#locals && this.#fixed === null ? this.#locals() : [];
        const alloc = allocateQuality(q, targets.map((t) => t.cost()));
        this.#global = alloc.global;
        this.#stage.setQuality(alloc.global >= 1 ? null : factorsFor(alloc.global));
        const lowered = new Set();
        targets.forEach((t, i) => {
            const lq = alloc.local[i];
            if (lq === null || lq === undefined)
                return;
            const { resolution: _shared, ...rest } = factorsFor(lq);
            t.setLocalQuality(rest);
            lowered.add(t);
        });
        for (const t of this.#lowered)
            if (!lowered.has(t))
                t.setLocalQuality(null);
        this.#lowered = lowered;
    }
}
