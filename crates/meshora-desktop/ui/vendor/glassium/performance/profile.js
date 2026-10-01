/**
 * 上次的自适应结果记在 localStorage 里，下次从附近起步（不用每次都从满质量掉下来）。
 *
 * 键里有：版本、后端、屏幕尺寸、DPR —— 换了任何一样就是另一份。7 天过期。读写都包 try/catch：隐私模式、
 * 禁了存储的浏览器里 localStorage 会抛，那就当没有。
 */
export const PROFILE_TTL_MS = 7 * 24 * 3600 * 1000;
export function profileKey(version, renderer, width, height, dpr) {
    return `glassium:quality:${version}:${renderer}:${Math.round(width)}x${Math.round(height)}@${Math.round(dpr * 100) / 100}`;
}
/** 解析存下来的文本；格式不对、过期了返回 null。 */
export function parseProfile(text, now) {
    if (!text)
        return null;
    try {
        const p = JSON.parse(text);
        if (typeof p.q !== 'number' || !(p.q > 0 && p.q <= 1))
            return null;
        if (typeof p.at !== 'number' || now - p.at > PROFILE_TTL_MS || p.at > now)
            return null;
        return { q: p.q, frameMs: Number(p.frameMs) || 0, resolution: Number(p.resolution) || 1, at: p.at };
    }
    catch {
        return null;
    }
}
export function loadProfile(key, now = Date.now()) {
    try {
        return parseProfile(globalThis.localStorage?.getItem(key) ?? null, now);
    }
    catch {
        return null;
    }
}
export function saveProfile(key, profile) {
    try {
        globalThis.localStorage?.setItem(key, JSON.stringify(profile));
    }
    catch {
        // 存不了就算了
    }
}
