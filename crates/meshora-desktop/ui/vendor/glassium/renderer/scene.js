/**
 * 场景：一帧里量到的东西收成的有类型、只读的结构 —— 渲染器的正式输入。
 *
 * **不是第二个真源。** DOM 是唯一真源：每帧 `PanelRegistry.measure()` 量出面板、合并组、填充，这里只把它们
 * 编成节点树、按层分好、标上与上一帧相比哪里变了。场景不能从外面改（没有修改接口），下一帧整个重建。
 *
 * - **节点**：面板、合并组、填充。按绘制顺序排（层号从小到大；同一层里先填充、再单独的面板、再合并组，
 *   组的后面紧跟它的成员）—— 与两个后端实际画的顺序相同。
 * - **父子**：最近的、这一帧也是节点的玻璃祖先（沿渲染树往上）；合并组的成员的父亲是组，组的父亲是第一个成员的
 *   玻璃祖先。父子只是结构：层号（决定像素的那个）在测量时就定了，按 MAX_GLASS_LAYER 封顶。
 * - **Z 序**（order）：第几个画的。合并组的成员与组一起画（一次 draw），order 与组相同。
 * - **脏标记**：与上一个画了的帧（stage 的 lastFrame）的场景逐项比，分 transform / layout / material / content 四类。
 *   四类的并集与原来「这一帧画出来会不会与上一帧相同」（idle.ts）逐项等价：拿不准就当作变了。
 *
 * 后端暂时还读三个扁平数组（`panels` / `groups` / `fills`），它们就是测量结果原样，场景是它们的视图。
 */
import { splitLayers } from "./layers.js";
import { sameMask } from "./mask.js";
/** 什么都没变。共用一个对象：静止的帧不为脏标记分配。 */
export const CLEAN = Object.freeze({ transform: false, layout: false, material: false, content: false });
/** 新出现的节点（上一帧没有）：全脏。 */
export const ALL_DIRTY = Object.freeze({ transform: true, layout: true, material: true, content: true });
export function isClean(d) {
    return !(d.transform || d.layout || d.material || d.content);
}
let nextSerial = 0;
export function sameTuple(a, b) {
    if (a.length !== b.length)
        return false;
    for (let i = 0; i < a.length; i++)
        if (a[i] !== b[i])
            return false;
    return true;
}
export function sameBox(a, b) {
    return a.x0 === b.x0 && a.y0 === b.y0 && a.x1 === b.x1 && a.y1 === b.y1;
}
export function sameRoundedBox(a, b) {
    if (a === null || b === null)
        return a === b;
    return sameBox(a.box, b.box) && sameTuple(a.rx, b.rx) && sameTuple(a.ry, b.ry);
}
function sameBitmap(a, b) {
    if (!a || !b)
        return !a && !b;
    return a.version === b.version && sameTuple(a.geom, b.geom) && sameTuple(a.cell, b.cell);
}
function sameHole(a, b) {
    if (!a || !b)
        return !a && !b;
    return a.alpha === b.alpha && sameRoundedBox(a.shape, b.shape);
}
function dirtyOf(transform, layout, material, content) {
    return transform || layout || material || content ? { transform, layout, material, content } : CLEAN;
}
/** 同一块面板（同一个记录）两帧之间哪里变了。 */
export function panelDirty(a, b) {
    if (a.record !== b.record)
        return ALL_DIRTY;
    return dirtyOf(a.x !== b.x || a.y !== b.y || a.w !== b.w || a.h !== b.h || a.visualScale !== b.visualScale || !sameTuple(a.rotation, b.rotation), !sameTuple(a.scissor, b.scissor) ||
        !sameBox(a.clip, b.clip) ||
        !sameTuple(a.clipRadii, b.clipRadii) ||
        !sameTuple(a.clipRadiiY, b.clipRadiiY) ||
        !sameRoundedBox(a.clipShape, b.clipShape) ||
        !sameMask(a.mask, b.mask) ||
        a.layer !== b.layer, 
    // 降级结果与质量系数按引用比：材质或尺寸变了会重新降级，换一个新对象
    a.chain !== b.chain || a.quality !== b.quality || !sameTuple(a.light, b.light) || a.fade !== b.fade || a.tone !== b.tone, false);
}
/** 同一块填充两帧之间哪里变了。 */
export function fillDirty(a, b) {
    if (a.record !== b.record)
        return ALL_DIRTY;
    return dirtyOf(a.x !== b.x || a.y !== b.y || a.w !== b.w || a.h !== b.h || !sameTuple(a.rotation, b.rotation), !sameTuple(a.scissor, b.scissor) ||
        !sameBox(a.clip, b.clip) ||
        !sameTuple(a.clipRadii, b.clipRadii) ||
        !sameTuple(a.clipRadiiY, b.clipRadiiY) ||
        !sameRoundedBox(a.clipShape, b.clipShape) ||
        !sameMask(a.mask, b.mask) ||
        a.layer !== b.layer, 
    // 渐变按引用比：解算结果缓存在记录上，渐变与尺寸没变就是同一个对象
    !sameTuple(a.radii, b.radii) || !sameTuple(a.radiiY, b.radiiY) || !sameTuple(a.color, b.color) || a.gradient !== b.gradient, !sameBitmap(a.bitmap, b.bitmap) || !sameHole(a.hole, b.hole));
}
/** 同一个合并组（第一个成员相同）两帧之间组自己哪里变了；成员各自的变化在成员节点上。 */
export function groupDirty(a, b) {
    let members = a.members.length === b.members.length;
    for (let i = 0; members && i < a.members.length; i++)
        members = a.members[i].record === b.members[i].record;
    return dirtyOf(!sameBox(unionBounds(a.members), unionBounds(b.members)), !members || !sameTuple(a.scissor, b.scissor) || a.layer !== b.layer, a.smoothingPx !== b.smoothingPx, false);
}
function unionBounds(members) {
    let x0 = Infinity;
    let y0 = Infinity;
    let x1 = -Infinity;
    let y1 = -Infinity;
    for (const m of members) {
        x0 = Math.min(x0, m.bounds.x0);
        y0 = Math.min(y0, m.bounds.y0);
        x1 = Math.max(x1, m.bounds.x1);
        y1 = Math.max(y1, m.bounds.y1);
    }
    return { x0, y0, x1, y1 };
}
function unionClip(members) {
    let x0 = Infinity;
    let y0 = Infinity;
    let x1 = -Infinity;
    let y1 = -Infinity;
    for (const m of members) {
        x0 = Math.min(x0, m.clip.x0);
        y0 = Math.min(y0, m.clip.y0);
        x1 = Math.max(x1, m.clip.x1);
        y1 = Math.max(y1, m.clip.y1);
    }
    return { x0, y0, x1, y1 };
}
/** 填充的包围盒：有旋转时是转过之后的矩形的轴对齐包围盒（绕矩形中心转）。 */
function fillBounds(f) {
    const [c, s] = f.rotation;
    if (s === 0)
        return { x0: f.x, y0: f.y, x1: f.x + f.w, y1: f.y + f.h };
    const cx = f.x + f.w / 2;
    const cy = f.y + f.h / 2;
    const hw = (Math.abs(c) * f.w + Math.abs(s) * f.h) / 2;
    const hh = (Math.abs(s) * f.w + Math.abs(c) * f.h) / 2;
    return { x0: cx - hw, y0: cy - hh, x1: cx + hw, y1: cy + hh };
}
const visibleScissor = (s) => s[2] > 0 && s[3] > 0;
/** 两个场景的节点在结构上对得上吗：每种节点的记录序列（扁平数组里的顺序）逐个相同。 */
function sameStructure(prev, next) {
    if (prev.panels.length !== next.panels.length || prev.groups.length !== next.groups.length || prev.fills.length !== next.fills.length) {
        return false;
    }
    for (let i = 0; i < next.panels.length; i++)
        if (prev.panels[i].record !== next.panels[i].record)
            return false;
    for (let i = 0; i < next.groups.length; i++)
        if (prev.groups[i].members[0].record !== next.groups[i].members[0].record)
            return false;
    for (let i = 0; i < next.fills.length; i++)
        if (prev.fills[i].record !== next.fills[i].record)
            return false;
    return true;
}
/**
 * 把一帧的测量结果编成场景。prev 是上一个**画了的**帧的场景（脏标记相对它算）；没有就全脏。
 */
export function buildScene(measured, prev) {
    const { panels, groups, fills } = measured;
    const glassParents = measured.glassParents;
    const layers = splitLayers(panels, groups, fills);
    // 1) 节点，按绘制顺序；脏标记相对上一帧的同一块。节点就地建好（父子在第 2 步填），不另建草稿再拷
    const nodes = [];
    const panelIds = new Map();
    const groupIds = new Map();
    const fillIds = new Map();
    const prevNode = (id) => (id === undefined ? undefined : prev.nodes[id]);
    const addPanel = (p, order, group) => {
        const before = prev ? prevNode(prev.index.panels.get(p.record)) : undefined;
        const id = nodes.length;
        nodes.push({
            id,
            kind: 'panel',
            parent: group,
            children: [],
            layer: p.layer,
            order,
            worldRect: p.bounds,
            clip: p.clip,
            opacity: p.fade,
            visible: visibleScissor(p.scissor),
            dirty: before?.kind === 'panel' ? panelDirty(before.panel, p) : ALL_DIRTY,
            element: p.record.element,
            panel: p,
            group
        });
        panelIds.set(p.record, id);
        return id;
    };
    let order = 0;
    for (const layer of layers) {
        for (const i of layer.fills) {
            const f = fills[i];
            const before = prev ? prevNode(prev.index.fills.get(f.record)) : undefined;
            const id = nodes.length;
            nodes.push({
                id,
                kind: 'fill',
                parent: null,
                children: [],
                layer: f.layer,
                order: order++,
                worldRect: fillBounds(f),
                clip: f.clip,
                opacity: f.color[3],
                visible: visibleScissor(f.scissor),
                dirty: before?.kind === 'fill' ? fillDirty(before.fill, f) : ALL_DIRTY,
                element: f.record.element,
                fill: f
            });
            fillIds.set(f.record, id);
        }
        for (const i of layer.panels)
            addPanel(panels[i], order++, null);
        for (const i of layer.groups) {
            const g = groups[i];
            const key = g.members[0].record;
            const before = prev ? prevNode(prev.index.groups.get(key)) : undefined;
            const id = nodes.length;
            const own = order++;
            let opacity = 0;
            for (const m of g.members)
                opacity = Math.max(opacity, m.fade);
            const members = [];
            nodes.push({
                id,
                kind: 'group',
                parent: null,
                children: members,
                layer: g.layer,
                order: own,
                worldRect: unionBounds(g.members),
                clip: unionClip(g.members),
                opacity,
                visible: visibleScissor(g.scissor),
                dirty: before?.kind === 'group' ? groupDirty(before.group, g) : ALL_DIRTY,
                group: g,
                members
            });
            groupIds.set(key, id);
            for (const m of g.members)
                members.push(addPanel(m, own, id));
        }
    }
    // 2) 父子：最近的、这一帧也是节点的玻璃祖先。祖先不是节点（屏外、单独没画）就接着往上找。
    //    没有玻璃祖先的帧（最常见）整段跳过
    const roots = [];
    if (glassParents && glassParents.size > 0) {
        const nodeAbove = (record) => {
            let p = glassParents.get(record) ?? null;
            for (let guard = 0; p && guard <= nodes.length; guard++) {
                const id = panelIds.get(p);
                if (id !== undefined)
                    return id;
                p = glassParents.get(p) ?? null;
            }
            return null;
        };
        for (const n of nodes) {
            if (n.kind === 'fill')
                n.parent = nodeAbove(n.fill.record);
            else if (n.kind === 'panel' && n.group === null)
                n.parent = nodeAbove(n.panel.record);
        }
        for (const n of nodes) {
            if (n.kind !== 'group')
                continue;
            // 组的父亲：第一个成员的玻璃祖先。成员写在另一个成员里面这种怪情形会成环 —— 那就当作根
            let parent = nodeAbove(n.group.members[0].record);
            for (let p = parent, guard = 0; p !== null && guard <= nodes.length; p = nodes[p].parent, guard++) {
                if (p === n.id) {
                    parent = null;
                    break;
                }
            }
            n.parent = parent;
        }
    }
    for (const n of nodes) {
        if (n.parent === null)
            roots.push(n.id);
        else if (nodes[n.parent].kind !== 'group')
            nodes[n.parent].children.push(n.id); // 组的子节点就是成员，已经填好
    }
    let changed = prev === null;
    for (let i = 0; !changed && i < nodes.length; i++)
        changed = !isClean(nodes[i].dirty);
    const scene = {
        serial: nextSerial++,
        base: prev ? prev.serial : null,
        changed,
        nodes,
        roots,
        layers,
        panels,
        groups,
        fills,
        index: { panels: panelIds, groups: groupIds, fills: fillIds }
    };
    if (!changed && !sameStructure(prev, scene))
        return { ...scene, changed: true };
    return scene;
}
/**
 * next 画出来可能与 prev 不同吗（只看场景，帧级的视口、背景、时间在 idle.ts）。
 * next 正是相对 prev 建的：直接用建的时候算好的；否则现比一遍（同一套比较）。
 */
export function sceneChanged(prev, next) {
    if (prev === null)
        return true;
    if (next.base === prev.serial)
        return next.changed;
    if (!sameStructure(prev, next))
        return true;
    for (let i = 0; i < next.panels.length; i++)
        if (!isClean(panelDirty(prev.panels[i], next.panels[i])))
            return true;
    for (let i = 0; i < next.fills.length; i++)
        if (!isClean(fillDirty(prev.fills[i], next.fills[i])))
            return true;
    for (let i = 0; i < next.groups.length; i++) {
        const a = prev.groups[i];
        const b = next.groups[i];
        if (!isClean(groupDirty(a, b)))
            return true;
        for (let j = 0; j < b.members.length; j++)
            if (!isClean(panelDirty(a.members[j], b.members[j])))
                return true;
    }
    return false;
}
