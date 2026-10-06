# Glassium（随客户端打包的副本）

客户端的液态玻璃来自 [Glassium](https://github.com/Kerxs/glassium) 1.0.1（Apache-2.0，见同目录的
`LICENSE`、`NOTICE`、`THIRD-PARTY-NOTICES.md`）。

**现在这份是 1.0.1 加上 Glassium 仓库里还没发布的两个提交**（`c55f833`、`5439882`，下一个版本会带上），都是按 Meshora 的需要做的：
- `configure({ backend: 'css' })`：不建 GPU 画布，玻璃全照材质用 CSS 画 —— 安卓客户端用它（滚动和玻璃同步）
- 用 CSS 画的玻璃（`overlay`，弹窗）不再把它后面的背景收进 GPU 场景（以前会把遮罩挪到正文底下、清掉弹窗里按钮的底色）
- 别的玻璃的兜底表面不再被当成背景收进场景（以前卡片里的蓝色按钮底下会垫一块颜色）

和 npm 上的 1.0.1 只差 `renderer/panels.js`、`runtime/absorb.js`、`runtime/config.js`、`runtime/ensure-stage.js`、
`runtime/glassium.js` 五个文件。Glassium 发了新版本之后照下面的步骤换成发布的那份。

为什么是拷进来的文件，不是 npm 依赖：客户端界面没有构建步骤（Tauri 直接加载 `ui/`），
CSP 只允许加载自己的脚本（`script-src 'self'`），也不能联网时才有界面。

这里是 Glassium 发布包 `dist/` 里的 `.js` 和 `glassium.css`，去掉了 `.map`、`.d.ts`
和文件末尾指向 `.map` 的 `//# sourceMappingURL` 注释，其余原样未改。

## 更新

在 Glassium 仓库里 `npm run build` 之后：

```bash
dst=crates/meshora-desktop/ui/vendor/glassium
find "$dst" -name '*.js' -delete
(cd ../glassium/dist && find . -name '*.js' -o -name '*.css' | cpio -pdm "$OLDPWD/$dst")
find "$dst" -name '*.js' -exec sed -i '/^\/\/# sourceMappingURL=/d' {} +
cp ../glassium/{LICENSE,NOTICE,THIRD-PARTY-NOTICES.md} "$dst/"
```

然后改上面的版本号，在 `dev/index.html` 里把每个场景过一遍。
