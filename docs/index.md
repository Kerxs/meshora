---
layout: home
title: Meshora
titleTemplate: 异地好友，同一个局域网
---

<HomeHero />

<HomeSteps />

<FeatureRow eyebrow="广播转发" title="房间列表里直接看见" image="network-desktop" alt="网络页：你的地址、网状图、每个朋友的延迟">

很多游戏只支持局域网联机，找房间靠的是**局域网广播和组播**。只是组了个网的话，互相 ping 得通，
房间列表里却看不见对方。

Meshora 把这些广播转发给网里的每个人，游戏以为大家就在一个屋子里。客户端默认让 Windows 把广播交给 Meshora 的网卡。

已经在一台 Windows 上实测转发过；还没拿真的游戏在两台电脑之间试过。[联机要过的四关](/guide/lan-play)

</FeatureRow>

<FeatureRow eyebrow="直连优先" title="能直连就直连，打不通走中继" image="network-desktop" video="connect" :start="4" alt="连上之后，朋友一个个从中间飞出来，直连是实线，经中继是虚线" reverse>

两边都在普通路由器后面时先**打洞**，游戏流量走你们之间最短的路，不绕服务器。打不通就经**中继**转发，
直连断了两三秒内切过去，换网络也不用重连。

- 网状图上一眼看出谁直连、谁经中继、延迟和抖动多少
- 中继只转发加密过的报文，看不到你们在玩什么（[威胁模型](/guide/threat-model)写了它看得到什么）
- 加密用 WireGuard 协议（boringtun），不自己写密码学

</FeatureRow>

<FeatureRow eyebrow="直连模式" title="不用服务器也能连" image="direct-desktop" video="punch" alt="直连模式：正在打洞的那条线上两个光点往中间跑，打通后变成直连">

没有服务器、也没有公网 IP 时：房主和每位朋友**互发一段连接码**，客户端问公共 STUN 服务器拿到公网地址、
请路由器用 UPnP 开端口，两边同时打洞，直接连上。全程不经过任何 Meshora 的服务器。

**没有中继兜底**：两边都在运营商级 NAT / 对称型 NAT 后面、路由器又开不了端口时打不通，
朋友卡片上会写明卡在哪一边。还没在真实的家用路由器之间验证过。[不用服务器（直连）](/guide/direct)

</FeatureRow>

<FeatureRow eyebrow="Windows + 安卓" title="电脑和手机在同一个网里" image="network-phone" video="phone" kind="phone" alt="安卓客户端：同一个网络、同一套界面" reverse>

Windows 和安卓是**同一个客户端、同一套界面**。手机靠系统的 VPN 功能组网，只接管网里的地址，上网不受影响。

安卓客户端每次打包都在模拟器里经官方服务器真的连一次网；还没在真手机上大范围试过。[安卓客户端](/guide/android)

</FeatureRow>

<FeatureRow eyebrow="网主" title="网主说了算" image="admin-desktop" alt="网主管理页：成员、网络码、一次性和 24 小时的邀请码">

建网络的人是网主：看成员、移出捣乱的人、换网络码、发**一次性**或 **24 小时**的网络码、改名、解散网络，都在客户端里点。

网络码就是门票：谁拿到谁能进来。发给一个人用一次性的，泄露了也只能用那一次。

</FeatureRow>

<FeatureRow eyebrow="自动更新 · 开源" title="打开就是最新的" image="network-desktop" video="boot" alt="客户端启动：点阵从中心扩散，Logo 画出来飞到左上角" reverse>

每次打开都查一次有没有新版本，有就问你要不要更新。新版本的安装包带 **Ed25519 签名**，核对过才装，
Windows 上点一下装好、自动重新打开。

MIT 许可，代码都在 [GitHub](https://github.com/Kerxs/meshora)。

</FeatureRow>

<HomeSection eyebrow="公道地说" title="和同类工具比" more="/guide/comparison" moreText="完整对比和出处">

**虚拟局域网联机已经有现成的、被很多人用了很久的工具 —— Meshora 刚发布，还没经过真实环境的检验。**

| | Meshora | Radmin VPN | ZeroTier | Tailscale |
| --- | --- | --- | --- | --- |
| 成熟度 | 刚发布 | 成熟 | 成熟 | 成熟 |
| 平台 | Windows、安卓 | 仅 Windows | 多平台 | 多平台 |
| 源码 | 开源（MIT） | 不开源 | 客户端开源，控制器仅限非商业 | 客户端开源 |
| 游戏的局域网广播 | 转发（还没拿真的游戏验证） | 主打局域网游戏 | 转发 | 不转发 |

如果上面哪个已经满足你，现在就用它。

</HomeSection>

<HomeSection title="常见问题" more="/guide/faq" moreText="更多问题" faq>

**现在能用吗？**
能下载试用：[Windows 客户端](/guide/desktop)、[安卓客户端](/guide/android)。但它还没在两台真的电脑之间拿真的游戏
验证过，没在真实网络里测过打洞，也没经过安全审查 —— 先当成尝鲜，别指望它一定行。

**每个人都要装吗？**
要。每个参与联机的人都装一个 Meshora，加入同一个网络。

**会让延迟变高吗？**
能打洞直连时，游戏流量走的就是你和朋友之间的直接路径，不绕服务器。
打不通走中继时会多绕一段，多多少取决于中继离你们多远 —— 具体数字要等真实网络的测试，现在写了就是编的。

**需要公网 IP 吗？**
不需要。建网络默认放在官方服务器上，它有公网地址，大家都连它；能打洞时游戏流量直接走你们之间，不经过它。

</HomeSection>

<HomeSection eyebrow="1.0.4" title="现在到哪一步了" more="/guide/roadmap" moreText="看路线图">

**1.0.4 已发布：Windows 安装程序和安卓 APK 在 [Releases](https://github.com/Kerxs/meshora/releases/latest)。**
装过的客户端会自动更新。

做到的：在客户端里建网络、凭网络码加入、网主管理；打洞直连、打不通走中继；游戏的广播和组播转发给网里的每个人；
不用服务器的直连模式；安卓模拟器每次打包都经官方服务器真的连一次网。

还没做到的：两台真的电脑之间拿真的游戏联机过、真实网络里的打洞验证（包括直连模式）、真手机上的验证、安全审查、
Windows 安装程序的代码签名（SmartScreen 会提示"未知发布者"）。

想知道它怎么做到的：[架构](/guide/architecture)、[建连流水线](/guide/connection-flow)。
如果这个方向对你有意思，[参与进来](/guide/contributing)说明了现阶段最需要什么样的帮助。

</HomeSection>

<HomeCta />
