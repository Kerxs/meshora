/**
 * GPU 与 CPU 光学实现的逐像素比对。
 *
 * GPU 端没法单步调试，CPU 端可以。只要两者在每个像素上算出同样的数，GPU 上的任何
 * 视觉问题就都不是光学写错了 —— 这条比对把「查光学」这一大类嫌疑一次排除掉。
 *
 * ## 比什么
 *
 * 不直接比折射方向 dir，而是比**实际的采样偏移** dir × displacement。
 *
 * 面板内部深处有一条 cy = cx 的对角线，gradSdRoundedRect 在那里从 (1,0) 跳到 (0,1)。
 * f32 与 f64 在紧贴这条线的像素上会落到不同的一侧，dir 差出 90° —— 但那里早已在
 * 折射带之外，displacement 恰为 0，这个差异根本不影响渲染。直接比 dir 会把它报成错，
 * 比偏移则不会。偏移才是真正移动像素的那个量。
 */
import { evalMergedOptics } from "../core/merge.js";
import { gradRadiusOf, gradSdRoundedRect, radiusAt, refractionDirection, refractionProfile, safeNormalize, sdRoundedRect } from "../core/optics.js";
/** 逐纹素比对 GPU 探针与一个 CPU 实现。单块面板与合并组共用。 */
function compareAgainst(width, height, origin, data, cpu) {
    let nonFinite = 0;
    let maxSd = 0;
    let maxDisp = 0;
    let maxOffset = 0;
    let worstSd = 0;
    const offsetErrs = [];
    for (let j = 0; j < height; j++) {
        for (let i = 0; i < width; i++) {
            const k = (j * width + i) * 4;
            const gSd = data[k];
            const gDx = data[k + 1];
            const gDy = data[k + 2];
            const gDisp = data[k + 3];
            if (!Number.isFinite(gSd) ||
                !Number.isFinite(gDx) ||
                !Number.isFinite(gDy) ||
                !Number.isFinite(gDisp)) {
                nonFinite++;
                continue;
            }
            // 片元位置取像素中心，与 @builtin(position) 的约定一致。
            const px = [origin[0] + i + 0.5, origin[1] + j + 0.5];
            const c = cpu(px);
            const eSd = Math.abs(gSd - c.sd);
            const eDisp = Math.abs(gDisp - c.displacement);
            const eOffset = Math.hypot(gDx * gDisp - c.dir[0] * c.displacement, gDy * gDisp - c.dir[1] * c.displacement);
            if (eSd > maxSd)
                maxSd = eSd;
            if (eDisp > maxDisp)
                maxDisp = eDisp;
            if (eOffset > maxOffset) {
                maxOffset = eOffset;
                worstSd = c.sd;
            }
            offsetErrs.push(eOffset);
        }
    }
    offsetErrs.sort((a, b) => a - b);
    const p99 = offsetErrs.length ? offsetErrs[Math.floor(offsetErrs.length * 0.99)] : 0;
    return {
        texels: width * height,
        gpuNonFinite: nonFinite,
        maxErr: { sd: maxSd, displacement: maxDisp, offset: maxOffset },
        p99OffsetErr: p99,
        worstOffsetAtSd: worstSd
    };
}
export function compareOptics(probe) {
    const { panel } = probe;
    const [rx, ry, rw, rh] = panel.rect;
    const halfSize = [rw / 2, rh / 2];
    const center = [rx + halfSize[0], ry + halfSize[1]];
    return compareAgainst(probe.width, probe.height, probe.origin, probe.data, (px) => {
        const centered = [px[0] - center[0], px[1] - center[1]];
        const radius = radiusAt(centered, panel.radii);
        const sd = sdRoundedRect(centered, halfSize, radius);
        return {
            sd,
            dir: refractionDirection(centered, halfSize, gradRadiusOf(radius, halfSize), panel.depthEffect),
            displacement: refractionProfile(sd, panel.heightPx, panel.amountPx, panel.squircle)
        };
    });
}
/**
 * 合并组的 GPU 探针与 core/merge.ts 的 CPU 实现逐像素比对。判据与单块面板相同：
 * 零个非有限值、采样偏移 p99 在 1e-4 像素以下。
 */
export function compareGroupOptics(probe) {
    return compareAgainst(probe.width, probe.height, probe.origin, probe.data, (px) => evalMergedOptics(px, probe.members, probe.smoothingPx));
}
export const SECTORS = ['T', 'TR', 'R', 'BR', 'B', 'BL', 'L', 'TL'];
const SECTOR_BY_OCTANT = ['L', 'TL', 'T', 'TR', 'R', 'BR', 'B', 'BL', 'L'];
/** 外法线 → 扇区。atan2 在 y 向下的屏幕坐标里：0 朝右、π/2 朝下。 */
export function sectorOf(n) {
    const octant = Math.round(Math.atan2(n[1], n[0]) / (Math.PI / 4)); // −4 … 4
    return SECTOR_BY_OCTANT[octant + 4];
}
/**
 * 按像素对齐探针与颜色回读。两者必须覆盖同一块区域：
 * 先 probeOptics(i)，再用它的 origin/width/height 去 readback。
 */
export function joinProbeAndColors(probe, rgba) {
    const { width, height, origin, data, panel } = probe;
    if (rgba.length !== width * height * 4) {
        throw new Error(`[Glassium] 探针 ${width}x${height} 与颜色回读的尺寸对不上（${rgba.length / 4} 像素）`);
    }
    const [rx, ry, rw, rh] = panel.rect;
    const halfSize = [rw / 2, rh / 2];
    const center = [rx + halfSize[0], ry + halfSize[1]];
    const out = [];
    for (let j = 0; j < height; j++) {
        for (let i = 0; i < width; i++) {
            const t = j * width + i;
            const px = [origin[0] + i + 0.5, origin[1] + j + 0.5];
            const centered = [px[0] - center[0], px[1] - center[1]];
            const r = radiusAt(centered, panel.radii);
            const normal = safeNormalize(gradSdRoundedRect(centered, halfSize, gradRadiusOf(r, halfSize)));
            out.push({
                px,
                sd: data[t * 4],
                displacement: data[t * 4 + 3],
                normal,
                sector: sectorOf(normal),
                rgb: [rgba[t * 4] / 255, rgba[t * 4 + 1] / 255, rgba[t * 4 + 2] / 255]
            });
        }
    }
    return out;
}
/**
 * 按扇区汇总某个量。pick 返回 null 表示这个像素不参与统计。
 * 没有样本的扇区照样出现在结果里（n = 0），免得「没测到」被读成「测到了 0」。
 */
export function summarizeBySector(pixels, pick) {
    const acc = new Map(SECTORS.map((k) => [k, []]));
    for (const p of pixels) {
        const v = pick(p);
        if (v === null || !Number.isFinite(v))
            continue;
        acc.get(p.sector).push(v);
    }
    const out = {};
    for (const k of SECTORS) {
        const vs = acc.get(k);
        if (vs.length === 0) {
            out[k] = { n: 0, mean: Number.NaN, min: Number.NaN, max: Number.NaN, positive: Number.NaN };
            continue;
        }
        // 不用 Math.min(...vs)：展开到参数列表会在十万量级的数组上撑爆调用栈，
        // 而一整块面板就有十八万个像素。
        let sum = 0;
        let min = Infinity;
        let max = -Infinity;
        let pos = 0;
        for (const v of vs) {
            sum += v;
            if (v < min)
                min = v;
            if (v > max)
                max = v;
            if (v > 0)
                pos++;
        }
        out[k] = { n: vs.length, mean: sum / vs.length, min, max, positive: pos / vs.length };
    }
    return out;
}
