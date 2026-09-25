# Meshora

**Connect Everything.**

一个开源、跨平台的设备网络连接与能力编排平台。把每台设备抽象成一个 Node，
自动完成发现、认证、NAT 穿透、建连与选路 —— 设备之间共享网络能力，网络自己选择路径。

官网与设计文档：<https://kerxs.github.io/meshora>

---

## 先说清楚仓库里现在有什么

**没有可运行的网络引擎，也没有可下载的二进制。** 项目处于设计阶段。

这个仓库目前有三样东西：

- **`docs/`** —— 官网和设计文档的源码（VitePress）。这是现阶段的主要产出。
- **`crates/`** —— M0 的 Rust workspace，里面是控制面与数据面之间的[接口契约](https://kerxs.github.io/meshora/guide/interfaces)，
  写成了 trait 和类型，带单元测试。**它只定义接口，不收发任何一个报文。**
- **仓库元文件** —— 许可证、贡献指南、安全策略。

boringtun 集成、wintun 虚拟网卡、控制面的实现，这些**一行都还没写**。
[路线图](https://kerxs.github.io/meshora/guide/roadmap)里每一项的状态都是真实的：
目前只有 M0 地基里的几项离开了「设计中」，功能项全都还在「设计中」。

先做官网是因为这个阶段最需要的是把设计讲清楚并收到反馈 —— 方向错了，代码写得再多也是白写。

## 打算怎么造

理解技术选型只需要先接受一个事实：

> WireGuard 本身不做节点发现、不做 NAT 穿透、不做中继、不做选路。
> 它只负责"两个已知 endpoint 之间的加密隧道"。
>
> **Meshora 要自研的控制面，恰好就是这些 WireGuard 不管的部分。**

所以系统分成界限清晰的两层：

```text
                  控制面（自研）
         身份 · 发现 · 穿透协调 · 选路 · ACL
                        │
           ┌────控制信令─┴─控制信令────┐
           │                          │
      ┌────┴─────┐              ┌─────┴────┐
      │  Node A  │              │  Node B  │
      │          │◀── P2P 直连 ─▶│          │
      │ boringtun│              │boringtun │
      │  wintun  │              │  wintun  │
      └────┬─────┘              └─────┬────┘
           │                          │
           └────▶  Relay（转发密文）◀──┘
                     打洞失败时回退
```

| 层 | 选择 | 理由 |
| --- | --- | --- |
| 数据面 | WireGuard 协议，boringtun 用户态实现 | **不自己写密码学**，直接继承 WireGuard 的安全分析结论 |
| 控制面 | 自研 | 这些能力 WireGuard 不提供，只能自己造 |
| 语言 | Rust | 内存安全、无 GC 停顿、交叉编译成熟，boringtun 本身也是 Rust |
| 首个平台 | Windows（wintun） | 开发机是它，且最难的分应用分流在 Windows 上 |

中继转发的是**已加密的报文**，读不到明文也改不了内容 —— 它不是一个需要被信任的节点。
但它能看到元数据（谁和谁通信、何时、多大流量），这一点文档里不回避。

## 六大能力

| 能力 | 技术实质 |
| --- | --- |
| P2P | 打洞成功后把对端 endpoint 写进 WireGuard peer 配置，直连 |
| Relay | 直连失败时中继转发密文。策略是"中继先行"，打洞在后台继续，成功后无缝切换 |
| Gateway | `AllowedIPs 0.0.0.0/0` 指向出口节点，网关侧开转发并做 NAT |
| Virtual LAN | 每节点分配 overlay IP（`100.64.0.0/10` 或 IPv6 ULA），经 TUN 设备路由 |
| Game Node | 难点是**老游戏靠 UDP 广播发现对局**，overlay 必须转发广播帧才算数 |
| Application Routing | 按进程分流在 Windows 上要动 **WFP**，比按 IP/端口分流难一个量级 |

详见[六大能力](https://kerxs.github.io/meshora/guide/capabilities)，每一项都写了已知难点和代价。

## 和 Tailscale 的关系

架构上高度相似，这没必要遮掩 —— 同样是 WireGuard 数据面 + 自研控制面 + 自建中继。
差异在**能力编排**（节点能提供什么是一等概念）和**按应用选路**（进程级分流，不只是 IP 段）。

**如果 Tailscale 满足你的需求，现在就该用它。** 它成熟、稳定、有商业公司维护，
而 Meshora 还没有可用版本。[完整对比](https://kerxs.github.io/meshora/guide/comparison)里也写了别人更好的地方。

## 本地跑这个站点

需要 **Node.js 20+**。

```bash
git clone https://github.com/Kerxs/meshora.git
cd meshora
npm ci

npm run docs:dev      # 开发服务器 → http://localhost:5173
npm run docs:build    # 构建，必须零警告通过
```

站点源码在 `docs/`，主题定制在 `docs/.vitepress/theme/`。

> 首页 hero 的流体背景由 [Paper Shaders](https://shaders.paper.design) 的 `Warp` 着色器驱动。
> 它在 SSG 阶段不渲染（Node 里没有 WebGL），是在客户端动态加载的；
> 系统开启「减少动态效果」时会自动降为静止渲染，且完全停掉渲染循环。
> WebGL 不可用时回落到一层 CSS 渐变，页面不会开天窗。

## 本地构建 Rust 部分

需要 [rustup](https://rustup.rs)。工具链版本固定在 `rust-toolchain.toml` 里，第一次跑 cargo 时会自动装上。

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings   # 和 CI 一样，零警告
cargo test --workspace
```

CI 在 Linux 和 Windows 上都跑测试，见 [`.github/workflows/rust.yml`](.github/workflows/rust.yml)。

## 部署

站点托管在 GitHub Pages 的项目子路径下：<https://kerxs.github.io/meshora>。
推送到 `main` 由 [`.github/workflows/deploy.yml`](.github/workflows/deploy.yml) 自动构建发布。

因为不是根路径，`config.mts` 里 `base` 必须是 `/meshora/`。**组件里动态拼出来的链接要包 `withBase()`** ——
Markdown 里的 `](/guide/x)` 会被 VitePress 自动加前缀，组件里的不会，漏了就会 404。

`cleanUrls` 开着：GitHub Pages 原生支持不重定向地把 `/foo` 当 `/foo.html` 提供。

### 改用自定义域名

三步：

1. `config.mts` 里 `base` 改回 `'/'`，`SITE` 改成你的域名
2. 新建 `docs/public/CNAME`，内容写域名（VitePress 会把 `public/` 原样拷进产物）
3. 在域名商处按 [GitHub Pages 文档](https://docs.github.com/pages/configuring-a-custom-domain-for-your-github-pages-site)配 A/ALIAS 记录，并在仓库 Settings → Pages 填上域名

本文件里的链接也要一并替换。

## 参与

现阶段最需要的是**设计反馈，不是代码** —— 接口契约刚有初版，M1 实现时还会改，提功能 PR 容易白写。
挑毛病、讲你的真实场景、指出"这个做不到"，都比 PR 有用。见
[参与进来](https://kerxs.github.io/meshora/guide/contributing)。

站点本身的错别字、死链、移动端显示问题，PR 随时欢迎。

## 许可

本仓库以 [MIT](LICENSE) 发布。

构建产物中包含第三方代码，其声明见 [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) ——
首页背景使用的 Paper Shaders 是 Apache-2.0，按其第 4(d) 条要求转载了 NOTICE。
