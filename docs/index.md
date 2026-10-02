---
layout: home

hero:
  name: Meshora
  text: 异地好友，同一个局域网
  tagline: 点一下建网络，把网络码发给朋友，贴进来就加入。只支持局域网联机的游戏，隔着城市也能开房间 —— 能直连就直连，打不通走中继，全程加密。
  actions:
    - theme: brand
      text: 下载
      link: https://github.com/Kerxs/meshora/releases/latest
    - theme: alt
      text: 怎么用
      link: /guide/desktop
    - theme: alt
      text: GitHub
      link: https://github.com/Kerxs/meshora

features:
  - title: 点一下就建好
    details: 起个名字点"建网络"，默认放在官方服务器上，朋友在哪都连得进来。把网络码发给朋友，他们贴进来就加入，不用配端口、不用开服务器。
  - title: 房间列表里直接看见
    details: 局域网游戏找房间靠广播和组播。Meshora 把它们转发到朋友那边 —— 不然就会互相 ping 得通、房间列表里却看不见对方。
  - title: 能直连就直连，打不通走中继
    details: 两边都在普通路由器后面时先打洞，游戏流量走最短的路；打不通就经中继转发。中继只转发加密过的报文，看不到你们在玩什么。
  - title: 电脑和手机在同一个网里
    details: Windows 和安卓是同一个客户端、同一套界面。手机靠系统的 VPN 功能组网，只接管网里的地址，上网不受影响。
  - title: 网主说了算
    details: 建网络的人是网主：看成员、移出捣乱的人、换网络码、发一次性或 24 小时的网络码、解散网络，都在客户端里点。
  - title: 自动更新，开源
    details: 新版本带签名，核对过才装，Windows 上点一下就更新好。MIT 许可，加密用 WireGuard 协议，不自己写密码学。
---

<HomeSection eyebrow="做什么" title="只做局域网联机" more="/guide/lan-play" moreText="联机要过的四关">

很多游戏只支持局域网联机。朋友不在同一个屋檐下，就只剩两条路：
自己配端口转发（宽带没有公网 IP 时根本配不了），或者装一个虚拟局域网工具。

Meshora 就是后者，**而且只做这一件事**。联机要过四关：

| | 要做到什么 | 现在 |
| --- | --- | --- |
| **局域网地址** | 每台设备多一块网卡、一个和朋友同网段的 IP | 已发布 |
| **找得到房间** | 游戏的广播和组播真的送到朋友那边 | 已发布，还没拿真的游戏验证过 |
| **连得上** | 能打洞就直连，打不通就走中继，断了自动切 | 已发布：模拟的 NAT、安卓模拟器经官方服务器都连通过 |
| **延迟稳** | 选路看抖动，直连断了尽快切走 | 已发布，还没在真实网络里量过 |

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

**虚拟局域网联机已经有现成的、被很多人用了很久的工具 —— Meshora 刚发布，还没经过真实环境的检验。**

| | Meshora | Radmin VPN | Hamachi | ZeroTier | Tailscale |
| --- | --- | --- | --- | --- | --- |
| 成熟度 | 刚发布，还没在真实环境里验证过 | 成熟 | 成熟 | 成熟 | 成熟 |
| 免费能用多少 | 开源，不设限 | 免费，不限人数 | 每个网络最多 5 台 | 10 台、1 个网络 | 最多 6 个用户 |
| 平台 | Windows、安卓 | 仅 Windows | Windows / Mac / Linux | 多平台 | 多平台 |
| 源码 | 开源（MIT） | 不开源 | 不开源 | 客户端 MPL-2.0，控制器仅限非商业 | 客户端 BSD-3 |
| 游戏的局域网广播 | 转发（还没拿真的游戏验证） | 官方主打局域网游戏 | 官方未写明 | 转发（二层虚拟网络） | 不转发 |

Meshora 想提供的是：**开源、可以完全自建、以联机为唯一目标**的那一个。
如果上面哪个已经满足你，现在就用它。

</HomeSection>

<HomeSection title="常见问题" more="/guide/faq" moreText="更多问题">

**现在能用吗？**
能下载试用：[Windows 客户端](/guide/desktop)、[安卓客户端](/guide/android)。但它还没在两台真的电脑之间拿真的游戏
验证过，没在真实网络里测过打洞，也没经过安全审查 —— 先当成尝鲜，别指望它一定行。

**每个人的电脑都要装吗？**
要。每个参与联机的人都装一个 Meshora，加入同一个网络。

**会让延迟变高吗？**
能打洞直连时，游戏流量走的就是你和朋友之间的直接路径，不绕服务器。
打不通走中继时会多绕一段，多多少取决于中继离你们多远 —— 具体数字要等真实网络的测试，现在写了就是编的。

**需要公网 IP 吗？**
不需要。建网络默认放在官方服务器上，它有公网地址，大家都连它；能打洞时游戏流量直接走你们之间，不经过它。

</HomeSection>

<HomeSection eyebrow="1.0.1" title="现在到哪一步了" more="/guide/roadmap" moreText="看路线图">

**1.0.1 已发布：Windows 安装程序和安卓 APK 在 [Releases](https://github.com/Kerxs/meshora/releases/latest)。**
装好打开，建网络默认用官方服务器；装过的客户端会自动更新。

做到的：在客户端里建网络、凭网络码加入、网主管理；打洞直连、打不通走中继；游戏的广播和组播转发给网里的每个人；
安卓模拟器每次打包都经官方服务器真的连一次网。

还没做到的：两台真的电脑之间拿真的游戏联机过、真实网络里的打洞验证、真手机上的验证、安全审查、
Windows 安装程序的代码签名（SmartScreen 会提示"未知发布者"）。

**只做局域网游戏联机。** 其余方向都在 [1.0 之后](/guide/roadmap#_1-0-之后)。
如果这个方向对你有意思，[参与进来](/guide/contributing)说明了现阶段最需要什么样的帮助。

</HomeSection>
