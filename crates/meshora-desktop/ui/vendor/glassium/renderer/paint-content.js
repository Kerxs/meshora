/**
 * 把元素里的文字与图标画进 2D 画布：位图填充（stage.registerBitmapFill）的 painter 用它。
 *
 * 用它的：DOM Renderer（runtime/content.ts，玻璃后面的内容块）、组件的 SceneLabels（components/scene-label.ts）。
 * 不依赖任何组件 —— `glassium/runtime` 入口也要用它。
 *
 * `<glass-segmented>`、`<glass-tab-bar>` 按住时把各段的文字、图标画进场景，选中块 / 气泡的透镜就能把它们放大、
 * 在边缘扭弯 —— iOS 26 拖动选中块时就是这样。平时照旧显示 DOM（锐利、可选中、无障碍），只有按住时换成画进
 * 场景的这一份（组件负责交叉淡化）。
 *
 * 画什么：
 * - 文字节点：按父元素的计算样式（字体、颜色、字距、方向）画在 Range 量出来的位置上；折成几行的文字节点
 *   逐字符量位置。宽度与 DOM 量到的差一点（亚像素、字形微调）时水平拉到一样宽。
 * - 内联 `<svg>`：克隆一份、把每个图形的计算样式（fill、stroke……，currentColor 已经解析成颜色）写成内联样式，
 *   序列化成图片异步解码 —— 解码好之前这个图标先不画，好了调 onReady 让调用方重画。
 * - 同源的 `<img>`（已经加载好的）。跨源图片不画：它会污染共享的图集画布，之后整张图集都传不进 GPU。
 *
 * 全开（runtime 把玻璃后面的内容画进场景）时另外画：元素的纯色背景与边框（实线、虚线、点线；四边一样时带圆角）、
 * 文字的下划线 / 上划线 / 删除线（祖先上写的也算，装饰会传给里面的文字）、`<canvas>` 与 `<video>` 的画面。
 *
 * 画不了的（跨源图片、CSS 背景图、text-shadow、渐变文字、波浪线……）跳过：按住时它们就不在场景里，
 * 见 docs/limitations.md。坐标按包围盒换算，不支持旋转。
 */
/** 逐字符量位置的文字节点最多这么长：再长的就只按第一个矩形画（标签不会这么长）。 */
const MAX_PER_CHAR = 400;
/** 画布上的字与 DOM 量到的宽度差在这个比例以内，就水平拉到一样宽（更大的差说明字体不一样，拉了反而难看）。 */
const FIT_TOLERANCE = 0.15;
/**
 * 把 sources 里的内容画进 ctx。ctx 的原点是 origin 元素盒子的左上角（变换之前）、单位是 CSS 像素 ——
 * 与 registerBitmapFill 给 painter 的 ctx 相同。异步的东西（SVG 图标）准备好之后调 onReady。
 */
export function paintContent(ctx, origin, sources, onReady, options = {}) {
    const map = localMapping(origin);
    if (!map)
        return;
    const full = options.backgrounds === true || options.media === true;
    for (const source of sources) {
        if (!full) {
            // 组件标签的老顺序（逐位不变）
            paintTexts(ctx, source, map);
            paintSvgs(ctx, source, map, onReady);
            paintImages(ctx, source, map);
            continue;
        }
        // 从下往上：背景 → 图片与画面 → 图标 → 文字
        if (options.backgrounds)
            paintBackgrounds(ctx, source, map);
        paintImages(ctx, source, map);
        if (options.media)
            paintMedia(ctx, source, map);
        paintSvgs(ctx, source, map, onReady);
        paintTexts(ctx, source, map, options.backgrounds === true);
    }
}
function localMapping(origin) {
    const box = origin.getBoundingClientRect();
    const w = origin.offsetWidth;
    const h = origin.offsetHeight;
    if (!(box.width > 0 && box.height > 0))
        return null;
    return { left: box.left, top: box.top, sx: w > 0 ? box.width / w : 1, sy: h > 0 ? box.height / h : 1 };
}
function toLocal(map, r) {
    return { x: (r.left - map.left) / map.sx, y: (r.top - map.top) / map.sy, w: r.width / map.sx, h: r.height / map.sy };
}
/**
 * 从 el 往上、到 stop 为止（不含 stop）的不透明度之积。stop 是要画的那一段本身：组件按住时正是把它淡出
 * （换成画进场景的这一份），它自己的不透明度不能算进来。
 */
function opacityUpTo(el, stop) {
    let o = 1;
    for (let e = el; e && e !== stop; e = e.parentElement) {
        const v = parseFloat(getComputedStyle(e).opacity);
        if (Number.isFinite(v))
            o *= v;
    }
    return o;
}
/** 计算样式 → 画布的 font。字号、粗细、斜体、小型大写、字体族与 DOM 相同。 */
export function canvasFont(cs) {
    const style = cs.fontStyle && cs.fontStyle !== 'normal' ? `${cs.fontStyle} ` : '';
    const variant = cs.fontVariant === 'small-caps' ? 'small-caps ' : '';
    return `${style}${variant}${cs.fontWeight} ${cs.fontSize} ${cs.fontFamily}`;
}
/**
 * 基线的位置：Range 的矩形是字体的内容区（上伸 + 下伸），基线在它顶上往下「上伸」处 —— 内容区与量到的
 * 上伸 + 下伸不一样高时两头平分差值。
 */
export function baselineIn(top, height, ascent, descent) {
    return top + (height - (ascent + descent)) / 2 + ascent;
}
function visible(el) {
    const cs = getComputedStyle(el);
    return cs.visibility !== 'hidden' && cs.display !== 'none';
}
function paintTexts(ctx, source, map, decorate = false) {
    const root = source.element;
    const doc = root.ownerDocument;
    const walker = doc.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    const range = doc.createRange();
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        const parent = node.parentElement;
        if (!parent || !node.data.trim() || !visible(parent))
            continue;
        // SVG 里的文字交给 SVG 那一支
        if (parent.closest('svg'))
            continue;
        const cs = getComputedStyle(parent);
        const alpha = opacityUpTo(parent, root);
        if (!(alpha > 0))
            continue;
        ctx.save();
        ctx.globalAlpha = alpha;
        ctx.font = canvasFont(cs);
        ctx.fillStyle = source.color ?? cs.color;
        ctx.textBaseline = 'alphabetic';
        ctx.textAlign = 'left';
        ctx.direction = cs.direction === 'rtl' ? 'rtl' : 'ltr';
        if ('letterSpacing' in ctx)
            ctx.letterSpacing = cs.letterSpacing === 'normal' ? '0px' : cs.letterSpacing;
        const lines = decorate ? decorationsOf(parent, root) : [];
        range.selectNodeContents(node);
        const rects = Array.from(range.getClientRects()).filter((r) => r.width > 0 && r.height > 0);
        if (lines.length > 0) {
            // 装饰线按每一行的矩形画（折行的文字每行一段），画在字下面
            const fontSize = parseFloat(cs.fontSize) || 16;
            for (const r of rects)
                drawDecorations(ctx, lines, toLocal(map, r), fontSize, source.color);
        }
        if (rects.length === 1) {
            drawRun(ctx, node.data.replace(/\s+/g, ' ').trim(), toLocal(map, rects[0]));
        }
        else if (rects.length > 1) {
            // 折成了几行：逐字符量（Range 一个字符一个字符地取矩形）
            const n = Math.min(node.data.length, MAX_PER_CHAR);
            for (let i = 0; i < n; i++) {
                const ch = node.data[i];
                if (/\s/.test(ch))
                    continue;
                range.setStart(node, i);
                range.setEnd(node, i + 1);
                const r = range.getClientRects()[0];
                if (r && r.width > 0)
                    drawRun(ctx, ch, toLocal(map, r));
            }
        }
        ctx.restore();
    }
    range.detach();
}
/** 解析一个元素的 text-decoration 计算值（没有线返回 null）。 */
export function parseDecoration(cs) {
    const line = cs.textDecorationLine || 'none';
    if (line === 'none')
        return null;
    const underline = /\bunderline\b/.test(line);
    const overline = /\boverline\b/.test(line);
    const lineThrough = /\bline-through\b/.test(line);
    if (!underline && !overline && !lineThrough)
        return null;
    const t = parseFloat(cs.textDecorationThickness);
    return {
        underline,
        overline,
        lineThrough,
        color: cs.textDecorationColor || cs.color,
        thickness: Number.isFinite(t) && t > 0 ? t : null,
        style: cs.textDecorationStyle || 'solid'
    };
}
/**
 * 一个文字节点身上的装饰：自己的父元素一直到 root，每一层写的都算 —— 装饰传给里面的文字，
 * 但计算值不继承（`<a>` 里的 `<span>` 的 text-decoration-line 是 none，字照样有下划线）。
 */
function decorationsOf(parent, root) {
    const out = [];
    for (let e = parent; e; e = e.parentElement) {
        const d = parseDecoration(getComputedStyle(e));
        if (d)
            out.push(d);
        if (e === root)
            break;
    }
    return out;
}
/**
 * 装饰线在一行里的位置（相对这一行矩形的顶，CSS 像素）：下划线在基线往下约 0.1em，删除线在基线往上约 0.3em
 * （x 字高的一半），上划线在上伸处。基线按内容区高度估计（上伸约 0.8em、下伸约 0.2em）。
 */
export function decorationOffsets(height, fontSize, thickness) {
    const baseline = baselineIn(0, height, fontSize * 0.8, fontSize * 0.2);
    return {
        underline: baseline + Math.max(1, fontSize * 0.1) + thickness / 2,
        lineThrough: baseline - fontSize * 0.3,
        overline: Math.max(thickness / 2, baseline - fontSize * 0.8 + thickness / 2)
    };
}
function drawDecorations(ctx, lines, box, fontSize, color) {
    for (const d of lines) {
        if (d.style === 'wavy')
            continue; // 波浪线不画
        const thickness = d.thickness ?? Math.max(1, fontSize / 14);
        const at = decorationOffsets(box.h, fontSize, thickness);
        ctx.save();
        ctx.strokeStyle = color ?? d.color;
        ctx.lineWidth = thickness;
        if (d.style === 'dashed')
            ctx.setLineDash([thickness * 3, thickness * 2]);
        else if (d.style === 'dotted')
            ctx.setLineDash([thickness, thickness]);
        const ys = [d.underline ? at.underline : null, d.overline ? at.overline : null, d.lineThrough ? at.lineThrough : null];
        for (const y of ys) {
            if (y === null)
                continue;
            const rows = d.style === 'double' ? [y, y + thickness * 2] : [y];
            for (const yy of rows) {
                ctx.beginPath();
                ctx.moveTo(box.x, box.y + yy);
                ctx.lineTo(box.x + box.w, box.y + yy);
                ctx.stroke();
            }
        }
        ctx.restore();
    }
}
/** 一段文字画在 box 里：基线按字体的上伸、下伸放，宽度差一点时水平拉到与 DOM 一样宽。 */
function drawRun(ctx, text, box) {
    if (!text)
        return;
    const m = ctx.measureText(text);
    const ascent = m.fontBoundingBoxAscent || m.actualBoundingBoxAscent;
    const descent = m.fontBoundingBoxDescent || m.actualBoundingBoxDescent;
    const y = baselineIn(box.y, box.h, ascent, descent);
    const k = m.width > 0 ? box.w / m.width : 1;
    if (Math.abs(k - 1) <= FIT_TOLERANCE && k !== 1) {
        ctx.save();
        ctx.translate(box.x, y);
        ctx.scale(k, 1);
        ctx.fillText(text, 0, 0);
        ctx.restore();
    }
    else {
        ctx.fillText(text, box.x, y);
    }
}
// —— SVG 图标 ——
/** 解码好的 SVG 图片，按序列化出来的文本缓存。 */
const svgCache = new Map();
const SVG_CACHE_LIMIT = 128;
/** 写进克隆的内联样式的计算属性：图形的颜色与描边（currentColor、CSS 变量、类选择器都已经解析了）。 */
const SVG_STYLE_PROPS = [
    'fill',
    'fill-opacity',
    'fill-rule',
    'stroke',
    'stroke-opacity',
    'stroke-width',
    'stroke-linecap',
    'stroke-linejoin',
    'stroke-dasharray',
    'opacity',
    'visibility',
    'display'
];
/**
 * 克隆一个 svg，把原件每个元素的计算样式写进克隆的 style（颜色换成 color 时 fill / stroke 里不是 none 的都换），
 * 宽高写成量到的尺寸，序列化成文本。
 */
export function serializeSvg(svg, width, height, color) {
    const clone = svg.cloneNode(true);
    const originals = [svg, ...Array.from(svg.querySelectorAll('*'))];
    const copies = [clone, ...Array.from(clone.querySelectorAll('*'))];
    for (let i = 0; i < originals.length && i < copies.length; i++) {
        const cs = getComputedStyle(originals[i]);
        const parts = [];
        for (const prop of SVG_STYLE_PROPS) {
            let v = cs.getPropertyValue(prop);
            if (!v)
                continue;
            if (color && (prop === 'fill' || prop === 'stroke') && v !== 'none' && !v.startsWith('url('))
                v = color;
            parts.push(`${prop}:${v}`);
        }
        ;
        copies[i].setAttribute('style', parts.join(';'));
    }
    clone.setAttribute('width', String(width));
    clone.setAttribute('height', String(height));
    if (!clone.getAttribute('xmlns'))
        clone.setAttribute('xmlns', 'http://www.w3.org/2000/svg');
    return new XMLSerializer().serializeToString(clone);
}
function paintSvgs(ctx, source, map, onReady) {
    const root = source.element;
    const svgs = root.localName === 'svg' ? [root] : [];
    for (const s of root.querySelectorAll('svg'))
        if (!s.parentElement?.closest('svg'))
            svgs.push(s);
    for (const svg of svgs) {
        if (!visible(svg))
            continue;
        const r = svg.getBoundingClientRect();
        if (!(r.width > 0 && r.height > 0))
            continue;
        const box = toLocal(map, r);
        const markup = serializeSvg(svg, Math.round(box.w * 100) / 100, Math.round(box.h * 100) / 100, source.color);
        const cached = svgCache.get(markup);
        if (cached instanceof HTMLImageElement) {
            ctx.save();
            ctx.globalAlpha = opacityUpTo(svg.parentElement ?? svg, root);
            ctx.drawImage(cached, box.x, box.y, box.w, box.h);
            ctx.restore();
        }
        else if (cached === undefined) {
            loadSvg(markup, onReady);
        }
    }
}
function loadSvg(markup, onReady) {
    if (typeof Image === 'undefined')
        return;
    if (svgCache.size >= SVG_CACHE_LIMIT)
        svgCache.delete(svgCache.keys().next().value);
    svgCache.set(markup, 'pending');
    const img = new Image();
    img.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(markup)}`;
    img
        .decode()
        .then(() => {
        svgCache.set(markup, img);
        onReady();
    })
        .catch(() => svgCache.set(markup, 'failed'));
}
// —— 图片 ——
/** 同源（含 data: / blob:）的图片才画：跨源的会污染画布。 */
export function sameOriginImage(src, base) {
    try {
        const url = new URL(src, base);
        if (url.protocol === 'data:' || url.protocol === 'blob:')
            return true;
        return url.origin === new URL(base).origin;
    }
    catch {
        return false;
    }
}
function paintImages(ctx, source, map) {
    const root = source.element;
    const images = root.localName === 'img' ? [root] : [];
    images.push(...Array.from(root.querySelectorAll('img')));
    for (const img of images) {
        if (!img.complete || !(img.naturalWidth > 0) || !visible(img))
            continue;
        if (!sameOriginImage(img.currentSrc || img.src, img.ownerDocument.baseURI))
            continue;
        const r = img.getBoundingClientRect();
        if (!(r.width > 0 && r.height > 0))
            continue;
        const box = toLocal(map, r);
        ctx.save();
        ctx.globalAlpha = opacityUpTo(img, root);
        drawFitted(ctx, img, img.naturalWidth, img.naturalHeight, box, getComputedStyle(img));
        ctx.restore();
    }
}
/**
 * 按 object-fit / object-position 算一张图（或一帧）在 box 里的位置。fill（img 的默认值）就是 box 本身，
 * 画出来与原来的 drawImage(box) 逐位相同。
 */
export function objectFitRect(box, naturalW, naturalH, fit, position) {
    if (fit === 'fill' || fit === '' || !(naturalW > 0 && naturalH > 0))
        return box;
    let k;
    if (fit === 'contain')
        k = Math.min(box.w / naturalW, box.h / naturalH);
    else if (fit === 'cover')
        k = Math.max(box.w / naturalW, box.h / naturalH);
    else if (fit === 'scale-down')
        k = Math.min(1, box.w / naturalW, box.h / naturalH);
    else
        k = 1; // none
    const w = naturalW * k;
    const h = naturalH * k;
    const [px = '50%', py = '50%'] = position.trim().split(/\s+/);
    const at = (v, free) => (v.endsWith('%') ? (parseFloat(v) / 100) * free : parseFloat(v) || 0);
    return { x: box.x + at(px, box.w - w), y: box.y + at(py, box.h - h), w, h };
}
function drawFitted(ctx, image, naturalW, naturalH, box, cs) {
    const dest = objectFitRect(box, naturalW, naturalH, cs.objectFit, cs.objectPosition);
    if (dest === box) {
        ctx.drawImage(image, box.x, box.y, box.w, box.h);
        return;
    }
    ctx.save();
    ctx.beginPath();
    ctx.rect(box.x, box.y, box.w, box.h);
    ctx.clip();
    ctx.drawImage(image, dest.x, dest.y, dest.w, dest.h);
    ctx.restore();
}
let probe = null;
/**
 * 画布能不能画进图集：把它缩画到一块 1×1 的小画布上读一个像素（被跨源内容污染过的会抛）。
 * 不在画布本身上 getContext —— 还没有上下文的画布会被占成 2D，页面之后就拿不到 WebGL 了。
 */
export function canvasIsClean(canvas) {
    if (!(canvas.width > 0 && canvas.height > 0))
        return true;
    try {
        probe ??= canvas.ownerDocument.createElement('canvas').getContext('2d', { willReadFrequently: true });
        if (!probe)
            return true;
        probe.clearRect(0, 0, 1, 1);
        probe.drawImage(canvas, 0, 0, 1, 1);
        probe.getImageData(0, 0, 1, 1);
        return true;
    }
    catch {
        // 污染过的画布画上去之后小画布也被污染了：换一块新的
        probe = null;
        return false;
    }
}
/** 视频能不能画进图集：同源（含 blob / data）的 src，或者 srcObject（摄像头、MediaStream）；还没有来源的也算（没有东西可画）。 */
export function videoIsClean(video) {
    if (video.srcObject)
        return true;
    const src = video.currentSrc || video.src;
    return src === '' || sameOriginImage(src, video.ownerDocument.baseURI);
}
function paintMedia(ctx, source, map) {
    const root = source.element;
    const media = [];
    if (root instanceof HTMLCanvasElement || root instanceof HTMLVideoElement)
        media.push(root);
    media.push(...Array.from(root.querySelectorAll('canvas, video')));
    for (const el of media) {
        if (!visible(el))
            continue;
        const r = el.getBoundingClientRect();
        if (!(r.width > 0 && r.height > 0))
            continue;
        let natW;
        let natH;
        if (el instanceof HTMLVideoElement) {
            if (el.readyState < 2 || !videoIsClean(el))
                continue;
            natW = el.videoWidth;
            natH = el.videoHeight;
        }
        else {
            if (!(el.width > 0 && el.height > 0) || !canvasIsClean(el))
                continue;
            natW = el.width;
            natH = el.height;
        }
        const box = toLocal(map, r);
        ctx.save();
        ctx.globalAlpha = opacityUpTo(el, root);
        drawFitted(ctx, el, natW, natH, box, getComputedStyle(el));
        ctx.restore();
    }
}
/** 颜色的 alpha 是不是 0（计算值是 rgb() / rgba() / transparent）。 */
function transparentColor(color) {
    const c = color.trim().toLowerCase();
    if (c === '' || c === 'transparent')
        return true;
    const m = /^rgba\(([^)]*)\)$/.exec(c);
    if (!m)
        return false;
    const parts = m[1].split(/[\s,/]+/).filter(Boolean);
    return parts.length >= 4 && parseFloat(parts[3]) === 0;
}
/** 读四条边（上、右、下、左）；一条都不画时返回 null。 */
export function bordersOf(cs) {
    const side = (s) => {
        const style = cs[`border${s}Style`];
        const width = style === 'none' || style === 'hidden' ? 0 : parseFloat(cs[`border${s}Width`]) || 0;
        return { width, color: cs[`border${s}Color`], style };
    };
    const sides = [side('Top'), side('Right'), side('Bottom'), side('Left')];
    return sides.some((s) => s.width > 0 && !transparentColor(s.color)) ? sides : null;
}
/**
 * 画一个盒子的边框。四边一样（宽度、颜色、线型）时沿圆角描一圈（虚线、点线按线型近似）；否则逐边画实心的条
 * （不管圆角 —— 四边不一样又带圆角的边框很少见）。double、groove 这些按实线画。
 */
function drawBorders(ctx, sides, b, radius) {
    const [t, r, btm, l] = sides;
    const same = sides.every((s) => s.width === t.width && s.color === t.color && s.style === t.style);
    if (same) {
        const w = t.width;
        ctx.save();
        ctx.strokeStyle = t.color;
        ctx.lineWidth = w;
        if (t.style === 'dashed')
            ctx.setLineDash([w * 3, w * 2]);
        else if (t.style === 'dotted') {
            ctx.setLineDash([0, w * 2]);
            ctx.lineCap = 'round';
        }
        const iw = b.w - w;
        const ih = b.h - w;
        const ir = Math.max(0, radius - w / 2);
        ctx.beginPath();
        if (ir > 0 && 'roundRect' in ctx)
            ctx.roundRect(b.x + w / 2, b.y + w / 2, iw, ih, Math.min(ir, iw / 2, ih / 2));
        else
            ctx.rect(b.x + w / 2, b.y + w / 2, iw, ih);
        ctx.stroke();
        ctx.restore();
        return;
    }
    const bar = (s, x, y, w, h) => {
        if (!(s.width > 0) || transparentColor(s.color))
            return;
        ctx.fillStyle = s.color;
        ctx.fillRect(x, y, w, h);
    };
    bar(t, b.x, b.y, b.w, t.width);
    bar(btm, b.x, b.y + b.h - btm.width, b.w, btm.width);
    bar(l, b.x, b.y, l.width, b.h);
    bar(r, b.x + b.w - r.width, b.y, r.width, b.h);
}
/**
 * 元素的纯色背景与边框：块级按盒子与圆角，内联按每一行的矩形。一个元素先背景后边框，外层的先画。
 * 渐变、背景图不画（那是 absorb 的事）。
 */
function paintBackgrounds(ctx, source, map) {
    const root = source.element;
    const els = [root, ...Array.from(root.querySelectorAll('*'))];
    for (const el of els) {
        if (el.localName !== 'svg' && el.closest('svg'))
            continue;
        const cs = getComputedStyle(el);
        if (cs.visibility === 'hidden' || cs.display === 'none')
            continue;
        const fill = !transparentColor(cs.backgroundColor);
        const borders = bordersOf(cs);
        if (!fill && !borders)
            continue;
        ctx.save();
        ctx.globalAlpha = el === root ? 1 : opacityUpTo(el, root);
        ctx.fillStyle = cs.backgroundColor;
        const inline = cs.display === 'inline';
        const rects = inline ? Array.from(el.getClientRects()) : [el.getBoundingClientRect()];
        for (const r of rects) {
            if (!(r.width > 0 && r.height > 0))
                continue;
            const b = toLocal(map, r);
            const radius = inline ? 0 : parseFloat(cs.borderTopLeftRadius) || 0;
            if (fill) {
                ctx.beginPath();
                if (radius > 0 && 'roundRect' in ctx)
                    ctx.roundRect(b.x, b.y, b.w, b.h, Math.min(radius, b.w / 2, b.h / 2));
                else
                    ctx.rect(b.x, b.y, b.w, b.h);
                ctx.fill();
            }
            if (borders)
                drawBorders(ctx, borders, b, radius);
        }
        ctx.restore();
    }
}
// —— 组件用的镜像 ——
/**
 * 一排段（分段控件的各段、标签栏的各格）画进场景的镜像：一个盖在它们上面的元素注册成位图填充，
 * 内容（文字、颜色、字体、图标）变了就作废，下次看得见时重画。`<glass-segmented>` 与 `<glass-tab-bar>` 共用。
 *
 * 平时镜像透明（不画、不占图集）；组件按住时让它不透明、把 DOM 的字淡出（见 ready）。
 */
/** SceneLabels 的「透镜里的那一份」：只在透镜的窗口里露出来，内容统一换成选中色。 */
