# 参与 Meshora

完整版见 <https://kerxs.github.io/meshora/guide/contributing>。下面是摘要。

## 现阶段最需要的是设计反馈，不是代码

接口契约还没定下来，这时候提功能 PR 大概率会白写 —— 底层一改就得推倒重来。

真正能帮到项目的：

- **挑毛病** —— 协议设计的漏洞、认证与密钥分发的问题、越权路径
- **"这个做不到"** —— 如果你知道某件事在某个平台上根本行不通，请一定早点说
- **"已经有轮子了"** —— 成熟解法不必重造
- **讲你的真实场景** —— 现在用什么方案、哪里最难受、什么网络环境

开 Issue：<https://github.com/Kerxs/meshora/issues>

安全问题请走 [SECURITY.md](SECURITY.md)，**不要开公开 Issue**。

## 站点的 PR 随时欢迎

错别字、表述不清、死链、移动端显示问题 —— 改了就能合。

```bash
npm ci
npm run docs:dev      # → http://localhost:5173
npm run docs:build    # 必须零警告通过（VitePress 会检查死链）
```

需要 Node 20+。站点源码在 `docs/`，主题定制在 `docs/.vitepress/theme/`。

## 关于代码

等 [M1](https://kerxs.github.io/meshora/guide/roadmap) 的接口契约定下来，会补上开发环境搭建说明
（rustup、wintun、交叉编译配置）。在那之前，讨论比写代码有用。
