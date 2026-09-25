# 参与 Meshora

完整版见 <https://kerxs.github.io/meshora/guide/contributing>。下面是摘要。

## 现阶段最需要的是设计反馈，不是代码

接口契约刚有初版，M1 实现时一定还会改。这时候提功能 PR 大概率会白写 —— 底层一改就得推倒重来。

真正能帮到项目的：

- **挑毛病** —— 协议设计的漏洞、认证与密钥分发的问题、越权路径
- **挑契约和威胁模型的毛病** —— [接口契约](https://kerxs.github.io/meshora/guide/interfaces)的不变量、
  [威胁模型](https://kerxs.github.io/meshora/guide/threat-model)的信任假设，哪条站不住都请说
- **"这个做不到"** —— 如果你知道某件事在某个平台上根本行不通，请一定早点说
- **"已经有轮子了"** —— 成熟解法不必重造
- **讲你的真实场景** —— 现在用什么方案、哪里最难受、什么网络环境

开 Issue：<https://github.com/Kerxs/meshora/issues>

安全问题请走 [SECURITY.md](SECURITY.md)，**不要开公开 Issue**。

## 站点的 PR 随时欢迎

错别字、表述不清、死链、移动端显示问题 —— 改了就能合。

```bash
npm ci
npm run docs:dev      # → http://localhost:5173
npm run docs:build    # 必须零警告通过（VitePress 会检查死链）
```

需要 Node 20+。站点源码在 `docs/`，主题定制在 `docs/.vitepress/theme/`。

## 关于代码

Rust 代码在 `crates/` 下。需要 [rustup](https://rustup.rs)，工具链版本固定在 `rust-toolchain.toml` 里，
第一次跑 cargo 时会自动装上。提交前跑一遍，CI 也会跑它们（另外还检查文档）：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

需要管理员权限（Linux 上是 root）的测试默认不跑：
`cargo test -p meshora-tun -p meshorad -- --ignored`（真的创建虚拟网卡，真的 meshorad 经隧道收发；
Windows 上要先把 wintun.dll 放到 `target/debug/deps/` 和 `target/debug/`），以及只在 Linux 上的
`sudo scripts/e2e-netns.sh target/debug`（用网络命名空间模拟几台机器和 NAT 路由器，真的 ping 一遍；
先编译好 meshorad 和 meshora-coord，需要 iproute2、iptables、ping）。
