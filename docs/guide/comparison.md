# 与同类方案对比

::: warning 当前状态
Meshora 还在开发中（M1），下表中它那一列大多是**设计目标**，不是实测结果。
其他项目的信息基于公开资料，如有出入欢迎[指正](https://github.com/Kerxs/meshora/issues)。
:::

先把话说清楚：**这个领域已经有很成熟的方案了。** 如果 Tailscale 能满足你的需求，用它就好 ——
它成熟、稳定、有商业公司维护。这一页的目的是说明 Meshora 打算在哪里做得不一样，而不是贬低谁。

## 总览

| | Meshora | Tailscale | ZeroTier | NetBird | frp |
| --- | --- | --- | --- | --- | --- |
| 数据面 | WireGuard | WireGuard | 自研协议 | WireGuard | 自研（TCP/KCP/QUIC） |
| 拓扑 | 网状 | 网状 | 网状 | 网状 | 星型（必经服务器） |
| NAT 穿透 | ✔ | ✔ | ✔ | ✔ | ✖（设计上就不需要） |
| 中继回退 | ✔ | ✔（DERP） | ✔ | ✔ | 本身就是中继 |
| 虚拟局域网 | ✔ | 部分 | ✔（含二层） | ✔ | ✖ |
| 广播/组播转发 | **设计目标** | ✖ | ✔ | ✖ | ✖ |
| 分应用路由 | **设计目标** | ✖ | ✖ | ✖ | ✖ |
| 语言 | Rust | Go | C++ | Go | Go |
| 许可 | MIT | BSD-3（客户端） | BSL | BSD-3 | Apache-2.0 |

## 和 Tailscale 的关系

**架构上高度相似，这一点没必要遮掩。** Tailscale 同样是
"WireGuard 数据面 + 自研控制面 + 自建中继（DERP）"，Meshora 走的是同一条被验证过的路。

真正的差异在两个地方：

**一、能力编排。** Tailscale 的模型里节点基本是同质的（除了 exit node 和 subnet router 这类角色）。
Meshora 想把"节点能提供什么"做成一等概念 —— 中继、出口、虚拟局域网、游戏档位都是可声明、可组合、可授权的能力。

**二、按应用选路。** Tailscale 的分流粒度是 IP 段。Meshora 想做到按**进程**分流：
这个应用走隧道、那个应用直连出网。这在 Windows 上要动 WFP，工程量不小，
但它是"网络自动选择路径"这句话真正的落点。

Tailscale 的优势也应当承认：成熟度、生态、MagicDNS、SSO 集成、企业支持。这些不是短期能追平的。

## 和 ZeroTier 的关系

ZeroTier 做的是**二层**虚拟网络，能转发广播和组播 —— 这正是 Meshora 在
[Virtual LAN](/guide/capabilities#virtual-lan) 和 [Game Node](/guide/capabilities#game-node)
上想达到的效果，ZeroTier 已经做到了，而且做得很好。

不同点是 ZeroTier 用的是自研协议栈而非 WireGuard，以及它现在采用 BSL 许可（并非严格意义的开源）。
Meshora 选择站在 WireGuard 上，图的是不自己写密码学。

**如果你的核心需求就是"局域网游戏联机"，ZeroTier 目前是更实际的选择。**

## 和 frp 的关系

frp 解决的是另一个问题：把内网服务暴露到公网。它是星型的，所有流量必经服务器，
设计上就不做 NAT 穿透。

如果你只是想让外网访问家里的一个 Web 服务，frp 更轻、更直接。
Meshora 要解决的是"一堆设备之间互相访问"，两者不冲突。

## 什么时候不该选 Meshora

- **现在。** 它还没有可用版本。
- 你需要的功能 Tailscale 已经有了，而你不关心分应用路由。
- 你需要商业支持或合规背书。
- 你只需要暴露单个服务到公网 —— 那是 frp 的场景。

下一步：[路线图](/guide/roadmap)。
