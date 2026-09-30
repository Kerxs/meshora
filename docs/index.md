---
layout: home

hero:
  name: Meshora
  text: 异地好友，同一个局域网
  tagline: 只支持局域网联机的游戏，隔着城市也能开房间。能直连就直连，打不通走中继，全程加密。1.0.0 只做这一件事。
  actions:
    - theme: brand
      text: 联机是怎么做到的
      link: /guide/lan-play
    - theme: alt
      text: 看路线图
      link: /guide/roadmap
    - theme: alt
      text: GitHub
      link: https://github.com/Kerxs/meshora

features:
  - title: 房间列表里直接看见
    details: 局域网游戏找房间靠广播和组播。Meshora 把它们转发到朋友的电脑上 —— 不然就会互相 ping 得通、房间列表里却看不见对方。
  - title: 能直连就直连
    details: 两边都在普通家用路由器后面时先打洞。打通之后游戏流量走你和朋友之间的最短路径，不绕任何服务器。
  - title: 打不通走中继
    details: 对称型 NAT 后面打不通洞，就经中继转发。中继只转发已经加密的报文，看不到你们在玩什么、说什么。
  - title: 延迟要稳，不只是要低
    details: 稳定的 60ms 比在 20~90ms 之间跳的体验好得多。选路时抖动的权重高于平均延迟。
  - title: 全程加密
    details: 数据面是 WireGuard 协议（boringtun 实现）。不自己写密码学，直接继承 WireGuard 已有的安全分析结论。
  - title: 开源，可以自建
    details: MIT 许可。协调服务和中继可以自己架在一台服务器上，不必依赖任何人的公共服务。
---

<HomeSection eyebrow="1.0.0" title="只做局域网联机" more="/guide/lan-play" moreText="联机要过的四关">

很多游戏只支持局域网联机。朋友不在同一个屋檐下，就只剩两条路：
自己配端口转发（宽带没有公网 IP 时根本配不了），或者装一个虚拟局域网工具。

Meshora 1.0.0 就是后者，**而且只做这一件事**，只做 Windows。联机要过四关：

| | 要做到什么 | 现在 |
| --- | --- | --- |
| **局域网地址** | 每台电脑多一块网卡、一个和朋友同网段的 IP | M1 已有 |
| **找得到房间** | 游戏的广播和组播真的送到朋友那边 | 开发中 —— 1.0.0 最关键的一关 |
| **连得上** | 能打洞就直连，打不通就走中继，断了自动切 | M1 已有（Linux 上测过） |
| **延迟稳** | 选路看抖动，直连断了尽快切走 | 1.0.0 要做 |

通用组网、网络出口、按应用分流这些方向都排到了 [1.0 之后](/guide/roadmap#_1-0-之后)。

</HomeSection>

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

<HomeSection eyebrow="建连流水线" title="自动连上" more="/guide/connection-flow" moreText="六步逐个拆开">

从你点"加入"到游戏里看见朋友，中间所有需要人工介入的环节都收进了一条流水线：

<Pipeline />

换 Wi-Fi、切手机热点、运营商重新分配了地址，都不需要你重新配置 ——
加密会话绑定在节点公钥上，不绑在 IP 和端口上。

</HomeSection>

<HomeSection eyebrow="谁会用它" title="使用场景" more="/guide/lan-play" moreText="局域网联机：1.0.0 做什么">

<ScenarioGrid />

</HomeSection>

<HomeSection eyebrow="公道地说" title="与同类工具对比" more="/guide/comparison" moreText="完整对比">

**虚拟局域网联机已经有现成的工具，而且现在就能用 —— Meshora 还不能。**

| | Meshora | Radmin VPN | Hamachi | ZeroTier | Tailscale |
| --- | --- | --- | --- | --- | --- |
| 现在能用 | 还不能（开发中） | 能 | 能 | 能 | 能 |
| 免费能用多少 | 开源，不设限 | 免费，不限人数 | 每个网络最多 5 台 | 10 台、1 个网络 | 最多 6 个用户 |
| 平台 | 1.0.0 只做 Windows | 仅 Windows | Windows / Mac / Linux | 多平台 | 多平台 |
| 源码 | 开源（MIT） | 不开源 | 不开源 | 客户端 MPL-2.0，控制器仅限非商业 | 客户端 BSD-3 |
| 游戏的局域网广播 | 1.0.0 目标 | 官方主打局域网游戏 | 官方未写明 | 转发（二层虚拟网络） | 不转发 |

Meshora 想提供的是：**开源、可以完全自建、以联机为唯一目标**的那一个。
如果上面哪个已经满足你，现在就用它。

</HomeSection>

<HomeSection title="常见问题" more="/guide/faq" moreText="更多问题">

**现在能用吗？**
还不能用于实际联机。M1 正在做：在 Linux 上从源码编译，两台机器之间能经加密隧道 ping 通；
但没在两台真的 Windows 机器之间试过，没在真实网络里测过打洞，也没经过安全审查。

**每个人的电脑都要装吗？**
要。每个参与联机的人都装一个 Meshora，加入同一个网络。

**会让延迟变高吗？**
能打洞直连时，游戏流量走的就是你和朋友之间的直接路径，不绕服务器。
打不通走中继时会多绕一段，多多少取决于中继离你们多远 —— 具体数字要等真实网络的测试，现在写了就是编的。

**需要公网 IP 吗？**
玩家的电脑不需要。中继需要一台有公网地址的服务器。

</HomeSection>

<HomeSection eyebrow="Pre-alpha" title="现在到哪一步了" more="/guide/roadmap" moreText="看路线图">

**M1 开发中。有了能运行的程序，但没有可下载的二进制，也还不适合实际使用。**

核心引擎用 Rust 写，数据面采用 WireGuard 协议（[boringtun](https://github.com/cloudflare/boringtun) 用户态实现），
控制面自研。在 Linux 上，两台机器之间已经能经加密隧道 ping 通 ——
能直连时走直连，在 NAT 后面先打洞，打不通时经中继。Windows 的[桌面客户端](/guide/desktop)也写好了：
贴一个网络码就能加入。还没有的：两台真的 Windows 机器之间的实测、
真实网络里的打洞验证、安全审查，以及 1.0.0 最关键的一项 —— 游戏广播的转发（数据面已经写了，还没拿真的游戏试过）。

**1.0.0 只做局域网游戏联机。** 其余方向都在 1.0 之后。

如果这个方向对你有意思，[参与进来](/guide/contributing)说明了现阶段最需要什么样的帮助。

</HomeSection>
