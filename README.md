# Meshora

**异地好友，同一个局域网。**

一个开源的局域网游戏联机工具：让不在一处的朋友，能用只支持局域网的游戏模式一起玩。
能打洞就直连，打不通就走中继，全程加密；协调服务和中继可以完全自建。

**1.0.0 只做这一件事，只做 Windows。** 通用组网、网络出口、按应用分流都排在 1.0 之后。

官网与设计文档：<https://kerxs.github.io/meshora>

---

## 先说清楚仓库里现在有什么

**有了能运行的程序，但没有可下载的二进制，也还不适合实际使用。** 项目正在做 M1。

这个仓库目前有三样东西：

- **`docs/`** —— 官网和设计文档的源码（VitePress）。
- **`crates/`** —— Rust 代码：节点守护进程 `meshorad`、协调服务 `meshora-coord`（可顺带跑中继），
  以及它们用到的数据面（基于 boringtun）、控制面、虚拟网卡、线协议。
  **在 Linux 上，两台机器之间能经加密隧道 ping 通**：同一网段直连、两边各在一个 NAT 后面打洞直连、
  打不通时经中继，都验证过（NAT 是用 Linux 模拟的）。**Windows 上**，守护进程和 wintun 虚拟网卡在
  CI 的 Windows 虚拟机里实测过：和同一台机器上的另一个节点经加密隧道收发报文，直连和经中继都通。
  还有 Windows 的**桌面客户端** `meshora-desktop`（Tauri）：贴一个网络码就能加入，见
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
安装包由 [`.github/workflows/desktop.yml`](.github/workflows/desktop.yml) 打。

## 自己试一试

> **还没经过任何安全审查，别拿它保护真实的流量。** 这一节是给想亲手验证设计的人看的。

最省事的是跑端到端脚本：它在一台机器上用网络命名空间模拟几台机器和 NAT 路由器，把整个流程走一遍
（需要 root、iproute2、iptables、ping）：

```bash
cargo build -p meshorad -p meshora-coord
sudo scripts/e2e-netns.sh target/debug
```

### 两台机器

两台要连起来的机器 A、B，外加一台两边都连得上的服务器跑协调服务（顺带跑一个中继）。

1. A、B 上各生成一把私钥。`genkey` 把私钥写进文件（已有的文件不覆盖），打出对应的公钥，记下来：

   ```bash
   meshorad genkey node.key
   ```

2. 服务器上也生成一把，打出的公钥要告诉 A、B：

   ```bash
   meshorad genkey coord.key
   ```

3. 服务器上启动协调服务，把 A、B 的公钥写进名单。它要对外开放 TCP 7443、UDP 7443、TCP 7444：

   ```bash
   meshora-coord --key coord.key --listen 0.0.0.0:7443 \
       --probe 0.0.0.0:7443 --probe-public <服务器地址>:7443 \
       --relay-listen 0.0.0.0:7444 --relay-public <服务器地址>:7444 \
       --node <A 的公钥> --node <B 的公钥>
   ```

4. A、B 上以 root 启动节点。overlay 地址按名单顺序分配：A 是 `100.64.0.1`，B 是 `100.64.0.2`：

   ```bash
   sudo meshorad up --key node.key --coord <服务器地址>:7443 --coord-key <协调服务的公钥>
   ```

   协调服务启动时会打出**网络码**（`公钥@地址:端口`），`--coord` 加 `--coord-key` 也可以换成 `--join <网络码>`。
   协调服务带上 `--state coord.state` 的话，网络码末尾还有 `#邀请码`：拿着它的节点不用先进名单，
   直接就能加入，地址自动分配。见[桌面客户端 · 给建网络的人](https://kerxs.github.io/meshora/guide/desktop#给建网络的人-起一个协调服务)。

然后 A 上 `ping 100.64.0.2`，B 上 `ping 100.64.0.1`。节点日志里的 `切换路径` 说明走的是哪条路：
`path=Direct(...)` 是直连，`path=Relay {...}` 是经中继。

### 两台 Windows 机器：M1 的验收

M1 的目标是两台 Windows 机器经 Meshora 互通。**还没有人在真机上做过这一步** —— CI 的 Windows 虚拟机上
跑通的是"一台机器，加上同一台机器里的另一个节点"。照下面做一遍、把结果告诉我们（开个 Issue 就行），
M1 才算有了结论。协调服务照上一节在服务器上跑，下面是每台 Windows 机器上要做的：

1. 编译。装 [rustup](https://rustup.rs) 和 Visual Studio 生成工具（勾选"使用 C++ 的桌面开发"），在仓库里：

   ```powershell
   cargo build --release -p meshorad
   ```

   程序在 `target\release\meshorad.exe`。

2. 从 [wintun.net](https://www.wintun.net) 下载 wintun 0.14.1，核对哈希（CI 核对的也是这个值）：

   ```powershell
   (Get-FileHash .\wintun-0.14.1.zip).Hash
   # 应为 07C256185D6EE3652E09FA55C0B673E2624B565E02C4B9091C79CA7D2F24EF51
   ```

   把压缩包里的 `wintun\bin\amd64\wintun.dll` 放到 `meshorad.exe` 旁边。meshorad 只从自己所在的目录加载它，
   不走系统搜索路径 —— 所以这个目录要只有管理员能写，否则别人放一个假的 `wintun.dll` 进去就能拿到管理员权限。

3. 打开"以管理员身份运行"的 PowerShell，进到 `meshorad.exe` 所在的目录。生成私钥（打出的公钥交给
   跑协调服务的人），再放行别的节点 ping 过来 —— Windows 防火墙默认挡进来的 ping，从这台往外 ping 不受影响：

   ```powershell
   .\meshorad.exe genkey node.key
   New-NetFirewallRule -DisplayName 'Meshora: allow ping' -Direction Inbound -Protocol ICMPv4 -IcmpType 8 -RemoteAddress 100.64.0.0/10 -Action Allow
   ```

   Windows 上不检查私钥文件的权限，它得在别人读不到的地方：仓库放在自己的用户目录里就行。

4. 启动节点。第一次运行时 Windows 可能弹窗问要不要允许 meshorad 访问网络，允许：

   ```powershell
   .\meshorad.exe up --key node.key --coord <服务器地址>:7443 --coord-key <协调服务的公钥>
   ```

算通过的标准：

- 两边的日志里都有 `已注册到协调服务` 和 `虚拟网卡已就绪`
- A 上 `ping 100.64.0.2`、B 上 `ping 100.64.0.1` 都通
- 两台在同一个局域网，或者都在普通的家用路由器后面时，最后一次 `切换路径` 应该是 `path=Direct(...)`；
  洞打不通时是 `path=Relay {...}`，ping 照样通

请告诉我们：两台机器的 Windows 版本、网络情况（同一个局域网？各自在什么样的路由器后面？）、
最后走的路径、ping 的延迟。出了问题的话，加 `-v` 重跑，附上两边的日志。

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
