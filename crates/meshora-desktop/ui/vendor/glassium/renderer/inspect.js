/**
 * 场景检查器的数据：上一帧量到的东西（面板、合并组的成员、填充）换成 CSS 像素、带上材质与降级结果。
 * `stage.debug.scene()` 给它，调试面板的「场景」页用它列出来、点一行看材质。
 */
/** 上一帧的快照 → 检查器的数据。没有画过（null）时是空的。 */
export function inspectFrame(frame) {
    if (!frame)
        return null;
    const v = frame.viewport;
    const kx = v.cssWidth / v.compositeWidth;
    const ky = v.cssHeight / v.compositeHeight;
    const rect = (x, y, w, h) => [x * kx, y * ky, w * kx, h * ky];
    const glass = (p, group) => {
        const r = p.record;
        return {
            element: p.record.element,
            group,
            layer: p.layer,
            rect: rect(p.x, p.y, p.w, p.h),
            rotationDeg: (Math.atan2(p.rotation[1], p.rotation[0]) * 180) / Math.PI,
            visualScale: p.visualScale,
            fade: p.fade,
            material: p.record.material,
            chain: p.chain,
            quality: p.quality ?? null,
            localQuality: r.localQuality ?? null,
            presentation: r.presentation ?? null
        };
    };
    const glasses = frame.panels.map((p) => glass(p, null));
    frame.groups.forEach((g, i) => {
        for (const m of g.members)
            glasses.push(glass(m, i));
    });
    const fills = frame.fills.map((f) => ({
        element: f.record.element,
        layer: f.layer,
        rect: rect(f.x, f.y, f.w, f.h),
        kind: f.bitmap ? 'bitmap' : f.gradient ? 'gradient' : 'color',
        color: f.color
    }));
    return { glasses, groups: frame.groups.length, fills, viewport: v };
}
