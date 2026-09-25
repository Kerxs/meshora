---
layout: home

hero:
  name: Meshora
  text: 设备自己连上，网络自己选路
  tagline: 把每台设备抽象成一个 Node，自动完成发现、认证、NAT 穿透、建连与选路。
  actions:
    - theme: brand
      text: 了解架构
      link: /guide/architecture
    - theme: alt
      text: 它解决什么问题
      link: /guide/what-is-meshora
    - theme: alt
      text: GitHub
      link: https://github.com/Kerxs/meshora

features:
  - title: P2P 直连
    details: 打洞成功后把对端 endpoint 写进 WireGuard peer 配置，流量走设备之间的最短路径，不绕服务器。
  - title: Relay 回退
    details: 打洞失败时自动经中继转发。中继转发的是已加密的报文，看不到明文，它不是一个需要被信任的节点。
  - title: Gateway 出口
    details: 指定某个节点作为网络出口，其余节点把默认路由指向它 —— 等于给整张网加了一个可选的落地点。
  - title: Virtual LAN
    details: 每个节点分到一个 overlay IP，跨地域的设备像插在同一台交换机上。广播帧也会转发，局域网发现才真正可用。
  - title: Game Node
    details: 为多人游戏优化的低延迟档位。难点不在带宽，在于让依赖 UDP 广播发现对局的老游戏在虚拟局域网里也能互相看见。
  - title: Application Routing
    details: 按应用、域名、IP、端口选择走哪条路径。哪些流量进隧道、哪些直连出网，由规则决定，不是全局一刀切。
---

<HomeSection eyebrow="控制面 + 数据面" title="架构" more="/guide/architecture" moreText="读完整架构">

理解 Meshora 的技术选型，只需要先接受一个事实：

> WireGuard 本身**不做**节点发现、**不做** NAT 穿透、**不做**中继、**不做**选路。
> 它只负责"两个已知 endpoint 之间的加密隧道"。
>
> **Meshora 要自研的控制面，恰好就是这些 WireGuard 不管的部分。**

<MeshDiagram />

| | 用什么 | 为什么 |
| --- | --- | --- |
| **数据面** | WireGuard 协议，boringtun 用户态实现；Windows 侧虚拟网卡用 wintun | **不自己写密码学**，直接继承 WireGuard 已有的安全分析结论 |
| **控制面** | 自研：身份、发现、端点探测、穿透协调、中继、链路探测、选路、ACL | 这些能力 WireGuard 不提供，只能自己造 |

</HomeSection>

<HomeSection eyebrow="建连流水线" title="自动网络" more="/guide/connection-flow" moreText="六步逐个拆开">

Meshora 把建连过程中所有需要人工介入的环节都收进了一条流水线：

<Pipeline />

系统持续探测延迟、丢包、抖动与带宽，据此选择合适路径，并在网络环境变化时自动调整 ——
换 Wi-Fi、切蜂窝网、运营商改了 NAT 行为，都不需要你重新配置。

路径是会变的，连接不会断：加密会话绑定在节点公钥上，不绑在 IP 和端口上。

</HomeSection>

<HomeSection eyebrow="谁会用它" title="使用场景" more="/guide/what-is-meshora" moreText="它解决什么问题">

<ScenarioGrid />

</HomeSection>

<HomeSection eyebrow="公道地说" title="与同类方案对比" more="/guide/comparison" moreText="完整对比">

**这个领域已经有很成熟的方案了。** 如果 Tailscale 能满足你的需求，用它就好。

| | Meshora | Tailscale | ZeroTier | frp |
| --- | --- | --- | --- | --- |
| 数据面 | WireGuard | WireGuard | 自研协议 | 自研 |
| 拓扑 | 网状 | 网状 | 网状 | 星型（必经服务器） |
| 广播 / 组播转发 | **设计目标** | ✖ | ✔ | ✖ |
| 分应用路由 | **设计目标** | ✖ | ✖ | ✖ |
| 许可 | MIT | BSD-3 | BSL | Apache-2.0 |

Meshora 和 Tailscale 架构上高度相似，这没必要遮掩 —— 同样是 WireGuard 数据面 + 自研控制面 + 自建中继。
差异在**能力编排**（节点能提供什么是一等概念）和**按应用选路**（进程级分流，不只是 IP 段）。

**什么时候不该选 Meshora：** 现在（还没有可用版本）；你的需求 Tailscale 已经满足；
你只想把单个服务暴露到公网（那是 frp 的场景）。

</HomeSection>

<HomeSection title="常见问题" more="/guide/faq" moreText="更多问题">

**现在能用吗？**
还不能用于实际用途。M1 正在做：在 Linux 上从源码编译，两台机器之间能经加密隧道 ping 通；但没在 Windows 上跑过，没在真实网络里测过打洞，也没经过安全审查。

**中继能看到我的数据吗？**
看不到内容 —— 转发的是已经用 WireGuard 加密的报文，改了也会在对端被丢弃。
但中继**能看到元数据**：谁在和谁通信、什么时候、流量多大。所有中继方案都是如此，在意的话可以自建。

**需要公网 IP 吗？**
普通节点不需要。但中继节点和出口节点需要稳定的公网可达地址，一张网里至少要有一个。

**支持 CGNAT 吗？**
这正是要解决的场景之一。CGNAT 下端口转发完全失效，只能靠打洞加中继兜底。

</HomeSection>

<HomeSection eyebrow="Pre-alpha" title="现在到哪一步了" more="/guide/roadmap" moreText="看路线图">

**M1 开发中。有了能运行的程序，但没有可下载的二进制，也还不适合实际使用。**

核心引擎用 Rust 写，数据面采用 WireGuard 协议（[boringtun](https://github.com/cloudflare/boringtun) 用户态实现），
控制面自研，首个目标平台是 Windows。在 Linux 上，两台机器之间已经能经加密隧道 ping 通 ——
能直连时走直连，在 NAT 后面先打洞，打不通时经中继。还没有的：Windows 上的实测、真实网络里的打洞验证、安全审查。

先做官网是因为这个阶段最需要的是把设计讲清楚并收到反馈 —— 方向错了，代码写得再多也是白写。

> **设备可以共享网络能力，网络可以自动选择路径。**

如果这个方向对你有意思，[参与进来](/guide/contributing)说明了现阶段最需要什么样的帮助 ——
目前主要是设计层面的讨论，而不是代码。

</HomeSection>
