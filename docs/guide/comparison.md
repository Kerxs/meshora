# 与同类工具对比

::: warning 当前状态
Meshora 还在开发中（M1），**现在还不能用来联机**，下表中它那一列大多是 1.0.0 的**设计目标**，不是实测结果。
其他工具的信息来自各自的官网和官方仓库（2026 年 9 月查的），如有出入或已经过时，欢迎[指正](https://github.com/Kerxs/meshora/issues)。
:::

先把话说清楚：**虚拟局域网联机已经有现成的工具，而且现在就能用。**
如果下面哪个已经满足你，就用它。这一页说明 Meshora 1.0.0 打算在哪里不一样，不是为了贬低谁。

## 总览

| | Meshora | Radmin VPN | Hamachi | ZeroTier | Tailscale |
| --- | --- | --- | --- | --- | --- |
| 现在能用 | 还不能（开发中） | 能 | 能 | 能 | 能 |
| 免费能用多少 | 开源，不设限 | 完全免费，不限人数 | 每个网络最多 5 台 | 免费档 10 台、1 个网络 | 免费档最多 6 个用户 |
| 平台 | 1.0.0 只做 Windows | 仅 Windows | Windows / Mac / Linux | 多平台 | 多平台 |
| 源码 | 开源（MIT） | 不开源 | 不开源 | 客户端 MPL-2.0；控制器等组件 source-available、仅限非商业 | 客户端 BSD-3 |
| 游戏的局域网广播 | 1.0.0 目标 | 官方主打局域网游戏联机 | 官方未写明 | 转发（二层虚拟网络） | 不转发 |
| 数据面 | WireGuard | 自有协议 | 自有协议 | 自有协议 | WireGuard |
| 服务端能否自建 | 能（协调服务、中继都开源） | 不能 | 不能 | 控制器可以自建（非商业） | 官方不提供；社区有 Headscale |

::: details 这些说法的出处
- Radmin VPN：官网写"完全免费，没有广告也没有付费功能""不限制玩家数量"，兼容 Windows 11/10/8/7
- Hamachi：官网写"每个网络最多 5 台电脑免费"，支持 Windows、Mac、Linux
- ZeroTier：定价页的免费档是 10 台设备、1 个网络；仓库的 LICENSE 写明客户端（Agent）是 MPL-2.0，
  `nonfree/` 目录下的控制器等组件用 ZeroTier 自己的 source-available 许可，仅限非商业使用
- Tailscale：定价页的 Personal 档免费，最多 6 个用户；不转发一般的局域网广播 / 组播，
  官方仓库里有长期开着的功能请求 [#11134](https://github.com/tailscale/tailscale/issues/11134)，
  提问者描述的正是"游戏只能靠局域网广播找房间，经 Tailscale 看不见"
:::

## 和 Radmin VPN、Hamachi 的关系

它们就是为局域网联机这件事做的，现在就能用，Radmin VPN 还完全免费、不限人数。
**如果你只想今晚和朋友开一局，用它们。**

Meshora 1.0.0 的不同只有两点：

- **开源**。客户端和服务端的代码都公开，MIT 许可
- **服务端可以完全自建**。协调服务和中继可以架在你自己的服务器上，不经过任何人的公共服务 ——
  前面两个工具的服务端都在厂商手里

代价是：Meshora 还不能用；而且 1.0.0 和 Radmin VPN 一样只做 Windows。

## 和 ZeroTier 的关系

ZeroTier 做的是**二层**虚拟网络，能转发广播和组播 —— 这正是 Meshora 1.0.0
[最关键的一关](/guide/lan-play#_2-房间列表里要能看见对方-——-真正的难点)要做到的，ZeroTier 已经做到了，而且做得很好。

不同点是 ZeroTier 用的是自有协议而不是 WireGuard，控制器的许可也不允许商业使用。
Meshora 选择站在 WireGuard 上，图的是不自己写密码学。

**如果你现在就要跨平台联机，ZeroTier 是更实际的选择。**

## 和 Tailscale 的关系

**架构上高度相似，这一点没必要遮掩。** Tailscale 同样是
"WireGuard 数据面 + 自研控制面 + 自建中继（DERP）"，Meshora 走的是同一条被验证过的路。

但 Tailscale 是三层网络，**不转发一般的局域网广播和组播** —— 对靠广播找房间的游戏，
就是互相 ping 得通、房间列表里看不见对方。这恰好是 Meshora 1.0.0 唯一要解决的事。

Tailscale 的优势也应当承认：成熟度、跨平台、生态、SSO 集成、企业支持。这些不是短期能追平的。
如果你联机的游戏支持直接输 IP 连接，Tailscale 现在就能用。

## 什么时候不该选 Meshora

- **现在。** 它还不能用。
- 你只想尽快和朋友开一局 —— 用 Radmin VPN 或 ZeroTier。
- 你需要 Mac、Linux 或手机上的客户端 —— 1.0.0 只做 Windows。
- 你要的不是游戏联机，而是通用组网、网络出口、按应用分流 —— 这些都在 [1.0 之后](/guide/roadmap#_1-0-之后)。

下一步：[路线图](/guide/roadmap)。
