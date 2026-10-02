# Meshora

**异地好友，同一个局域网。**

一个开源的局域网游戏联机工具：让不在一处的朋友，能用只支持局域网的游戏模式一起玩。
能打洞就直连，打不通就走中继，全程加密；协调服务和中继可以完全自建。

**1.0.0 只做这一件事，客户端有 Windows 和安卓。** 通用组网、网络出口、按应用分流都排在 1.0 之后。

官网与设计文档：<https://kerxs.github.io/meshora>

---

## 先说清楚仓库里现在有什么

**1.0.0 已发布：Windows 客户端的安装程序和安卓客户端的 APK 可以从 [Releases](https://github.com/Kerxs/meshora/releases) 下载。** 但还没经过安全审查，Windows 安装程序没有代码签名，也还没在两台真的电脑之间拿真的游戏验证过、没在真手机上试过 —— 欢迎试用，把结果告诉我们。

这个仓库目前有三样东西：

- **`docs/`** —— 官网和设计文档的源码（VitePress）。
- **`crates/`** —— Rust 代码：节点守护进程 `meshorad`、协调服务 `meshora-coord`（可顺带跑中继），
  以及它们用到的数据面（基于 boringtun）、控制面、虚拟网卡、线协议。
  **在 Linux 上，两台机器之间能经加密隧道 ping 通**：同一网段直连、两边各在一个 NAT 后面打洞直连、
  打不通时经中继，都验证过（NAT 是用 Linux 模拟的）。**Windows 上**，守护进程和 wintun 虚拟网卡在
  CI 的 Windows 虚拟机里实测过：和同一台机器上的另一个节点经加密隧道收发报文，直连和经中继都通。
  还有 Windows 的**桌面客户端** `meshora-desktop`（Tauri）和安卓客户端 `meshora-android`：在客户端里建网络、
  贴网络码加入、当网主管理，见
  [桌面客户端](https://kerxs.github.io/meshora/guide/desktop)。它和 `meshorad` 用同一套节点代码。
- **仓库元文件** —— 许可证、贡献指南、安全策略。

还差的：**没在两台真的 Windows 机器之间试过**（M1 的目标）；**没在真实网络里测过打洞**（家用路由器、运营商的 NAT）；
**没经过任何安全审查**。
[路线图](https://kerxs.github.io/meshora/guide/roadmap)里每一项的状态都是真实的。

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

## 1.0.0 做什么

联机要过四关：

| | 要做到什么 | 现在 |
| --- | --- | --- |
| 局域网地址 | 每个节点从 `100.64.0.0/10` 分到一个 overlay 地址，经虚拟网卡（Windows 上是 wintun）收发 | M1 已有（按配置名单静态分配） |
| 找得到房间 | 局域网游戏靠**广播和组播**找房间，要把它们真正送到朋友那边 —— 不然就是 ping 得通、房间列表里看不见 | 开发中，1.0.0 最关键的一关 |
| 连得上 | 打洞成功直连；打不通经中继转发密文；断了回落中继、恢复后切回 | M1 已有（Linux 上、NAT 是模拟的） |
| 延迟稳 | 选路时抖动的权重高于平均延迟；直连断了尽快切走（2~3 秒） | 开发中 |

详见[局域网联机：1.0.0 做什么](https://kerxs.github.io/meshora/guide/lan-play)。

## 和同类工具的关系

**虚拟局域网联机已经有现成的工具，而且现在就能用 —— Meshora 还不能。**
Radmin VPN 完全免费、不限人数，但只有 Windows、不开源；ZeroTier 能转发广播、跨平台，但控制器仅限非商业使用；
Tailscale 和 Meshora 架构最像，但它不转发局域网广播和组播，靠广播找房间的游戏经它看不见对方。

Meshora 想做的是**开源、服务端可以完全自建、以联机为唯一目标**的那一个。
如果上面哪个已经满足你，现在就用它。[完整对比](https://kerxs.github.io/meshora/guide/comparison)写了出处。

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

桌面客户端的界面部分（Tauri）只在 Windows 上编译；在别的平台上 `meshora-desktop` 只编出一个打一句话就退出的程序，
所以 Linux 上的 `cargo build --workspace` 不需要 WebKit。它的可执行文件要求管理员权限，
开发时设环境变量 `MESHORA_DESKTOP_AS_INVOKER=1` 再编译就不要求（这时连网络会停在建虚拟网卡那一步）。
Windows 安装程序和安卓 APK 由 [`.github/workflows/package.yml`](.github/workflows/package.yml) 打；它另外编一份 Linux 服务端，只给官方服务器升级用，不发布。

## 自己试一试

> **还没经过任何安全审查，别拿它保护真实的流量。**

从 [Releases](https://github.com/Kerxs/meshora/releases) 下载 Windows 安装程序或安卓 APK，装好打开：一个人建网络
（默认放在官方服务器上），把网络码发给朋友，朋友贴进"加入网络"。用法见
[Windows 客户端](https://kerxs.github.io/meshora/guide/desktop)、[安卓客户端](https://kerxs.github.io/meshora/guide/android)。

开发时验证核心的打洞和中继，有一个端到端脚本：在一台 Linux 机器上用网络命名空间模拟几台机器和 NAT 路由器，
把整个流程走一遍（需要 root、iproute2、iptables、ping；CI 每次都跑）。这只是测试用的，不是一个 Linux 版本：

```bash
cargo build -p meshorad -p meshora-coord
sudo scripts/e2e-netns.sh target/debug
```

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
首页背景使用的 Paper Shaders、桌面客户端界面使用的 Glassium 都是 Apache-2.0，按其第 4(d) 条要求转载了 NOTICE。
