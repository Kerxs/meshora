/**
 * 渲染后端的公共接口与一帧的输入输出。
 *
 * stage 持有面板、参数、帧循环与监听器，这些与后端无关；一个后端（WebGPU 或 WebGL2）
 * 持有一台设备 / 一个上下文上的全部 GPU 资源，可以整体丢弃、整体重建。两个后端吃同一份
 * FrameInput（面板的 uniform 打包也是同一份字节 —— WGSL 的 uniform 布局与 GLSL 的 std140
 * 在这些结构体上逐字节相同），吐同样格式的回读与探针，于是验证代码不用知道底下是谁。
 */
/**
 * 回读区域的边长（不给 region 时的默认值）。
 *
 * 256 不是随便取的：copyTextureToBuffer 要求 bytesPerRow 是 256 的倍数，
 * 而 256 像素 × 4 字节 = 1024，正好整除。
 */
export const READBACK_SIZE = 256;
/** 视频还没有可用的帧时不上传（上传会抛），继续用上一帧的内容。其它源总是就绪的。 */
export function sourceReady(source) {
    if (typeof HTMLVideoElement !== 'undefined' && source instanceof HTMLVideoElement) {
        return source.readyState >= 2; // HAVE_CURRENT_DATA
    }
    return true;
}
/** 连着这么多个画了的帧都没有更高的层，就把层的来源与备份放掉（约 10 秒）。 */
export const IDLE_LAYER_FRAMES = 600;
