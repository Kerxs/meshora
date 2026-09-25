# Meshora

**Connect Everything.**

一个开源、跨平台的设备网络连接与能力编排平台。把每台设备抽象成一个 Node，
自动完成发现、认证、NAT 穿透、建连与选路 —— 设备之间共享网络能力，网络自己选择路径。

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

## 自己试一试

> **还没经过任何安全审查，别拿它保护真实的流量。** 这一节是给想亲手验证设计的人看的。

最省事的是跑端到端脚本：它在一台机器上用网络命名空间模拟几台机器和 NAT 路由器，把整个流程走一遍
（需要 root、iproute2、iptables、ping）：

```bash
cargo build -p meshorad -p meshora-coord
sudo scripts/e2e-netns.sh target/debug
```

真的用两台机器的话，先在每台机器上生成密钥，把公钥记下来：

```bash
meshorad genkey > node.key && chmod 600 node.key
meshorad pubkey < node.key
```

再找一台两边都连得上的服务器跑协调服务（顺带跑一个中继）。它也要一把密钥，公钥要告诉节点：

```bash
meshorad genkey > coord.key && chmod 600 coord.key
meshora-coord pubkey < coord.key
```

把节点的公钥写进名单，启动：

```bash
meshora-coord --key coord.key --listen 0.0.0.0:7443 \
    --probe 0.0.0.0:7443 --probe-public <服务器地址>:7443 \
    --relay-listen 0.0.0.0:7444 --relay-public <服务器地址>:7444 \
    --node <节点A的公钥> --node <节点B的公钥>
```

每台机器上以 root 启动节点。overlay 地址按名单顺序分配：A 是 `100.64.0.1`，B 是 `100.64.0.2`：

```bash
sudo meshorad up --key node.key --coord <服务器地址>:7443 --coord-key <协调服务的公钥>
```

### Windows 上

只在 CI 的 Windows 虚拟机里跑过，还没在真机上试过。节点的步骤和上面一样，另外：

- 从 [wintun.net](https://www.wintun.net) 下载 wintun 0.14.1，把压缩包里对应 CPU 架构的 `wintun.dll`
  （一般是 `bin/amd64/`）放到 `meshorad.exe` 旁边。meshorad 只从自己所在的目录加载它，不走系统搜索路径。
  所以这个目录要只有管理员能写，否则别人放一个假的 `wintun.dll` 进去就能拿到管理员权限
- 在"以管理员身份运行"的终端里启动 `meshorad up`（不用 `sudo`）
- Windows 上不检查私钥文件的权限，自己把它放在别人读不到的地方
- Windows 防火墙默认挡进来的 ping：从别的节点 ping 这台 Windows，要先在防火墙里放行 ICMPv4 回显请求。
  从 Windows 往外 ping 不受影响

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
