# Meshora

**异地好友，同一个局域网。**

开源的局域网游戏联机工具：在客户端里点一下建网络，把网络码发给朋友，朋友贴进来就加入 ——
只支持局域网联机的游戏，隔着城市也能开房间。能打洞就直连，打不通就走中继，全程加密。

**[下载最新版](https://github.com/Kerxs/meshora/releases/latest)**（Windows 安装程序、安卓 APK）·
[官网与文档](https://kerxs.github.io/meshora) ·
[Windows 客户端](https://kerxs.github.io/meshora/guide/desktop) ·
[安卓客户端](https://kerxs.github.io/meshora/guide/android)

> **刚发布不久，真实环境里验证得还很少。** 装在真的电脑和手机上用过，但还没在两台真的电脑之间拿真的游戏联机过、
> 直连模式在真实网络里还没打通过、没经过安全审查，Windows 安装程序也没有代码签名（SmartScreen 会提示"未知发布者"）。欢迎试用，把结果告诉我们。

## 能做什么

- **点一下就建好网络**：默认放在官方服务器上，朋友在哪都连得进来；也可以本机当主机（Windows，UPnP），或者用自己的服务器
- **也能完全不用服务器**：和朋友互发一段连接码，靠打洞直连，[直连模式](https://kerxs.github.io/meshora/guide/direct)；
  没有中继兜底，对称型 NAT 连不上
- **房间列表里直接看见**：游戏找房间用的广播、组播转发给网里的每个人
- **能直连就直连，打不通走中继**：中继只转发加密过的报文；直连断了两三秒内切到中继，换网络不用重连
- **网主管理**：看成员、移出、换网络码、发一次性或 24 小时的网络码、改名、解散
- **电脑和手机在同一个网里**：Windows 和安卓是同一个客户端、同一套液态玻璃界面（[Glassium](https://github.com/Kerxs/glassium)）
- **自动更新**：新版本有 Ed25519 签名，核对过才装；Windows 上点一下就更新好

只做局域网游戏联机。通用组网、网络出口、按应用分流都排在 [1.0 之后](https://kerxs.github.io/meshora/guide/roadmap#_1-0-之后)。
客户端只有 Windows 和安卓。

## 现在做到哪了

| | 状态 |
| --- | --- |
| 建网络、凭网络码加入、网主管理 | 已发布 |
| 打洞直连、打不通走中继、断了自动切 | 已发布：模拟的 NAT 里测过；安卓模拟器每次打包都经官方服务器真的连一次网 |
| 游戏的广播、组播转发 | 已发布：一台 Windows 上实测转发过，还没拿真的游戏验证 |
| Windows 客户端、安装程序、自动更新 | 已发布：一台 Windows 11 上实测连通过 |
| 安卓客户端 | 已发布：真手机上跑过；CI 的模拟器里每次打包都连通 |
| 两台真的电脑之间拿真的游戏联机 | **还没做** |
| 不用服务器的直连模式 | 已发布：真实网络里试过，还没打通过 |
| 安全审查、Windows 代码签名 | **还没做** |

[路线图](https://kerxs.github.io/meshora/guide/roadmap)里每一项的状态都是真实的。

## 怎么造的

> WireGuard 本身不做节点发现、不做 NAT 穿透、不做中继、不做选路。
> 它只负责"两个已知 endpoint 之间的加密隧道"。
>
> **Meshora 自研的控制面，恰好就是这些 WireGuard 不管的部分。**

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
      │  虚拟网卡 │              │ 虚拟网卡  │
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
| 客户端 | Tauri 2 + [Glassium](https://github.com/Kerxs/glassium) 液态玻璃界面 | Windows 用 wintun 网卡，安卓用系统的 VpnService；节点代码同一套 |

中继转发的是**已加密的报文**，读不到明文也改不了内容 —— 它不是一个需要被信任的节点。
但它能看到元数据（谁和谁通信、何时、多大流量），[威胁模型](https://kerxs.github.io/meshora/guide/threat-model)不回避这一点。

仓库里：

- **`crates/`** —— Rust：客户端（`meshora-desktop`、`meshora-android`）、安装程序（`meshora-setup`）、
  节点（`meshorad`）、协调服务和中继（`meshora-coord`、`meshora-relay`）、数据面、控制面、虚拟网卡、线协议
- **`docs/`** —— 官网和设计文档（VitePress）
- **仓库元文件** —— 许可证、贡献指南、安全策略

## 和同类工具的关系

**虚拟局域网联机已经有现成的、被很多人用了很久的工具 —— Meshora 刚发布不久，真实环境里验证得还很少。**
Radmin VPN 完全免费、不限人数，但只有 Windows、不开源；ZeroTier 能转发广播、跨平台，但控制器仅限非商业使用；
Tailscale 和 Meshora 架构最像，但它不转发局域网广播和组播，靠广播找房间的游戏经它看不见对方。

Meshora 想做的是**开源、以联机为唯一目标**的那一个。
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

> 整站的磨砂玻璃用的是 [Glassium](https://github.com/Kerxs/glassium)（和客户端同一套），背景是一层细点阵，指针附近的点会聚拢变亮
> （`docs/.vitepress/theme/DotBackdrop.vue`）。玻璃由 `theme/glass.ts` 按选择器标到 VitePress 的组件上；
> 浮在正文上的顶部导航用 CSS 毛玻璃（Glassium 的玻璃盖不住压在它上面的字）。

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
