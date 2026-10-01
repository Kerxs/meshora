# 安卓客户端

::: warning 还没在真的手机上试过
安卓客户端每次打包都在 CI 的安卓 14 模拟器上真的连一次网（见[它是怎么做的](#它是怎么做的)），
**还没在真的手机上、拿手机游戏和电脑联机过**。试了的话，把结果开 [Issue](https://github.com/Kerxs/meshora/issues) 告诉我们。
:::

和 [Windows 客户端](/guide/desktop)是同一个东西：同一个液态玻璃界面、同一套节点代码，建网络、加入网络、网主管理都一样。
手机和电脑可以在同一个网络里。

和电脑上不同的几处：

- **靠系统的 VPN 功能组网。** 第一次连接时系统会问"是否允许 Meshora 设置 VPN 连接"，点"确定"。
  它只接管网里的地址（`100.64.0.0/10`，加上局域网游戏找房间用的广播、组播），**上网的流量不经过它**。
  安卓同一时间只让一个 VPN 工作：开着别的 VPN（加速器之类）时，Meshora 连不上，反过来也一样
- **不能本机当主机。** 手机多半在运营商级 NAT 后面，换个网络地址就变。建网络用官方服务器或你自己的服务器
- 没有"让游戏的广播走 Meshora""设为专用网络"这两项：那是 Windows 网卡的设置
- 导航在屏幕底部；顶上一张卡片是你的局域网地址和"断开"

## 下载、安装

APK 发在 [GitHub Releases](https://github.com/Kerxs/meshora/releases)：`Meshora-<版本>-android.apk`，安卓 7.0 以上。
没有上架应用商店，要允许浏览器（或文件管理器）"安装未知应用"。装之前可以按 `SHA256SUMS.txt` 核对。

APK 是用固定的一把钥匙签的。以后的新版本也是它签的，系统据此认出是同一个 App，才让覆盖安装、保留数据。

## 更新

设置里"检查更新"，或者等它自己查到（每 6 小时一次，设置里可以关）：卡片上出现"可更新"，点一下用浏览器下载新的 APK，
下载完点开安装。系统会核对新 APK 和装着的是同一把钥匙签的，对不上就不让装。

## 它是怎么做的

- 界面用 [Tauri 2](https://tauri.app)，由系统的 WebView 显示，和 Windows 客户端是同一份文件（`crates/meshora-desktop/ui`），
  在手机上换成一栏排
- 节点跑在 App 进程里，和 Windows 客户端、命令行的 `meshorad` 是同一套代码。不同的只有网卡：App 没有 root，
  网卡请系统的 VpnService 建（`crates/meshora-vpn`，Kotlin），建好把文件描述符交给 Rust，加密、选路、广播转发都在 Rust 里
- Meshora 自己的流量排除在 VPN 之外，不会绕回自己
- 私钥和设置存在 App 自己的数据目录里，别的 App 读不到；卸载 App 就一起删了（再装就是换了一个身份）
- APK 由 CI 打（[`.github/workflows/package.yml`](https://github.com/Kerxs/meshora/blob/main/.github/workflows/package.yml)），
  一个 APK 里带 arm64（手机）和 x86_64（模拟器）两份
- 每次打包都在 CI 的安卓 14 模拟器上走一遍（`scripts/android-smoke.sh`）：装上、点界面走完引导、凭网络码加入一个
  建在官方服务器上的网络；CI 这边的节点 ping 它，安卓系统回了才算过 —— VpnService 的网卡、Rust 数据面、
  经官方服务器的中继都通。最后解散这个网络

### 还不知道行不行的

- **手机游戏的局域网发现。** 网卡上加了广播（`255.255.255.255`）和组播（`224.0.0.0/4`）的路由，但安卓上的游戏
  （比如 Minecraft 基岩版）是不是真的把找房间的广播交给 VPN，要真机试了才知道。直接输对方地址加入应该是可以的
- **App 在后台时。** 系统可能为了省电冻结后台的 App；VPN 服务在跑时系统一般不会杀它，但各家手机的省电策略不一样

## 自己编译

要 Android SDK、NDK、JDK 17、Node.js，和 Rust 的 `aarch64-linux-android` 目标：

```bash
cd crates/meshora-android
npm ci
npm run tauri -- android init
npm run tauri -- android build --apk --target aarch64
```

产物在 `gen/android/app/build/outputs/apk/` 下，没签名，要自己用 `apksigner` 签。
界面在电脑的浏览器里就能看：`crates/meshora-desktop/dev/index.html`，把浏览器的设备模拟切成安卓手机（界面按 User-Agent 认手机）。
