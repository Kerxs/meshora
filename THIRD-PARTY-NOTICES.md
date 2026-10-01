# 第三方声明

本仓库自身以 MIT 协议发布（见 [LICENSE](LICENSE)）。官网和桌面客户端的构建产物中包含以下第三方代码，其许可条款如下。

---

## Paper Shaders

首页 hero 的流体背景使用 [`@paper-design/shaders`](https://github.com/paper-design/shaders) 的 `Warp` 着色器。该库以 **Apache License 2.0** 发布，并随包附带 NOTICE 文件。

由于 `npm run docs:build` 会把该库打包进 `docs/.vitepress/dist/`，属于 Apache-2.0 第 4(d) 条意义上的再分发，故在此转载其 NOTICE 内容：

```
Paper Shaders
Copyright 2026 Paper

Powered by Paper Shaders:
https://shaders.paper.design
```

Apache License 2.0 全文见 <https://www.apache.org/licenses/LICENSE-2.0>。

---

## IBM Plex

文档站使用 IBM Plex Sans / IBM Plex Mono，经 Google Fonts 加载。该字体家族以 **SIL Open Font License 1.1** 发布，版权归 IBM Corp. 所有。

---

## Glassium

桌面客户端的液态玻璃界面使用 [Glassium](https://github.com/Kerxs/glassium) 1.0.0，以 **Apache License 2.0** 发布。
它的发布文件随客户端打包（`crates/meshora-desktop/ui/vendor/glassium/`，同目录有许可证全文、NOTICE 与它自己的第三方声明），
属于再分发，故在此转载其 NOTICE：

```
Glassium
Copyright 2026 Glassium Contributors

This product includes software developed as a derivative of
AndroidLiquidGlass / Backdrop (io.github.kyant0:backdrop),
Copyright 2025 Kyant, licensed under the Apache License, Version 2.0.
https://github.com/Kyant0/AndroidLiquidGlass

The optical mathematics in src/core/optics.ts and
src/shaders/optics.wgsl.ts are ported from that project's
internal/Shaders.kt and have been modified. See docs/porting-notes.md
for the full list of changes.
```
