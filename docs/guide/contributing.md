# 参与进来

## 现阶段最需要什么

**不是代码，是设计反馈。**

接口契约刚有初版，M1 实现时一定还会改。这时候提功能 PR 大概率会白写 —— 底层一改就得推倒重来。
真正能帮到项目的是下面这些：

### 挑毛病

设计文档里如果有你认为错误或者想当然的地方，直接开 Issue 说。特别欢迎这几类：

- **协议设计上的漏洞** —— 认证、密钥分发、重放、降级攻击
- **契约和威胁模型站不住的地方** —— [接口契约](/guide/interfaces)的五条不变量、
  [威胁模型](/guide/threat-model)的信任假设，哪条有问题都请说
- **"这个做不到"** —— 如果你知道某个平台上某件事根本行不通，请一定告诉我，
  越早知道越好
- **已经有轮子了** —— 如果某个问题已经有成熟的解法，不必重造

### 讲讲你的实际场景

设计最容易跑偏的方式是对着想象中的用户造东西。如果你有真实需求，说出来很有价值：

- 你现在用什么方案，哪里最难受
- 什么网络环境（CGNAT？公司网络？多地组网？）
- 具体要连什么设备，跑什么应用

### 补充对比信息

[对比页](/guide/comparison)里关于其他项目的描述基于公开资料，
如果有出入或者已经过时，欢迎指正。**目标是公道，不是把自己写得更好看。**

## 怎么提

- **Issues**：<https://github.com/Kerxs/meshora/issues>
- 安全相关的问题请走 [SECURITY.md](https://github.com/Kerxs/meshora/blob/main/SECURITY.md) 里的渠道，不要开公开 Issue

## 站点本身的改进

官网和文档的错别字、表述不清、死链、移动端显示问题 —— 这些 PR 随时欢迎，改了就能合。

本地跑起来：

```bash
npm ci
npm run docs:dev
```

需要 Node 20 以上。站点源码在 `docs/`，主题定制在 `docs/.vitepress/theme/`。

```bash
npm run docs:build
```

构建必须零警告通过 —— VitePress 会检查文档内部死链，这是站点的第一道质检。

## 关于代码

Rust 代码在仓库的 `crates/` 下：节点守护进程、协调服务、中继，以及它们用到的数据面、控制面、虚拟网卡、线协议。
需要 [rustup](https://rustup.rs)，工具链版本固定在 `rust-toolchain.toml` 里，第一次跑 cargo 时会自动装上。
提交前跑一遍下面三条，CI 也会跑它们（另外还检查文档）：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

需要管理员权限（Linux 上是 root）的测试默认不跑：

- 真的创建虚拟网卡，真的 meshorad 和测试进程里的另一个节点经隧道收发：
  `cargo test -p meshora-tun -p meshorad -- --ignored`。Windows 上要先把 wintun.dll 放到
  `target/debug/deps/` 和 `target/debug/`，并且防火墙放行来自 198.18.0.0/15 的 ping（测试里另一个节点会主动
  ping 过来）。CI 里怎么下载、核对 wintun.dll，放行 ping 用的什么命令，都在 `.github/workflows/rust.yml`
- 只在 Linux 上：用网络命名空间模拟几台机器和 NAT 路由器，真的 ping 一遍。先 `cargo build -p meshorad -p meshora-coord`，
  再 `sudo scripts/e2e-netns.sh target/debug`（需要 iproute2、iptables、ping）
