# Glassium（随客户端打包的副本）

客户端的液态玻璃来自 [Glassium](https://github.com/Kerxs/glassium) 1.0.1（Apache-2.0，见同目录的
`LICENSE`、`NOTICE`、`THIRD-PARTY-NOTICES.md`）。

**现在这份是 1.0.1 加上一个还没发布的修复**（Glassium 仓库的提交 c55f833，下一个补丁版本会带上）：
用 CSS 画的玻璃（`overlay`）不再把它后面的背景、文字收进 GPU 场景 —— 收了会把弹窗的遮罩挪到正文底下、
把弹窗里着色按钮的底色清掉。和 npm 上的 1.0.1 只差 `renderer/panels.js`、`runtime/absorb.js` 两个文件。
Glassium 发了新版本之后照下面的步骤换成发布的那份。

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
