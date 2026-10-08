# Glassium（随客户端打包的副本）

客户端的液态玻璃来自 [Glassium](https://github.com/Kerxs/glassium) 1.1.1（Apache-2.0，见同目录的
`LICENSE`、`NOTICE`、`THIRD-PARTY-NOTICES.md`）。1.1.0、1.1.1 里有几处是按 Meshora 的需要做的：`backend: 'css'`（安卓客户端用）、
弹窗这种用 CSS 画的玻璃不再把它后面的东西收进 GPU 场景、玻璃自己的兜底表面不再被当成背景收走；
CSS 画玻璃时标签栏按住、拖动是液态玻璃透镜（安卓客户端底部的导航）、GPU 玻璃生效时元素的默认底色换成透明。

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
