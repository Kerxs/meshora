#!/usr/bin/env bash
# 把托管很多网络的协调服务（客户端里的"官方服务器"，也可以是你自己的）装到一台 Linux 服务器上。
#
#   scripts/deploy-hub.sh <用户@服务器> <服务器公网 IP> [版本，默认最新 | 本地的服务端 .tar.gz]
#
# 在服务器上（经 ssh，要能 sudo；要密码的话会在终端里问）：
#   1. 从 GitHub Releases 下载对应架构的服务端，按 SHA256SUMS.txt 核对；
#      第三个参数是本地文件时（比如 CI 编出来、还没发版的包），把它传上去，按本地算的 SHA-256 核对
#   2. 装到 /usr/local/bin，建系统用户 meshora，数据放 /var/lib/meshora（私钥第一次时生成）
#   3. 装 systemd 服务 meshora-hub.service（--hub /var/lib/meshora，同进程带中继），开机自启
#   4. 打出服务器地址：填进客户端设置里的"官方服务器"或"我的服务器"
#
# 端口：TCP 7443（协调服务）、UDP 7443（端点探测）、TCP 7444（中继）。开着 ufw 的话脚本会放行；
# 云厂商的安全组要你自己去控制台放行。
set -euo pipefail

if [[ $# -lt 2 ]]; then
  sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'
  exit 1
fi
target=$1
public=$2
version=${3:-latest}

# 服务器上要跑的那一段先传上去再跑：sudo 要密码时得有终端（ssh -t），标准输入就不能拿来传脚本
remote=$(cat <<'REMOTE'
set -euo pipefail
public=$1
version=$2
expected=${3:-}
repo=Kerxs/meshora

case "$(uname -m)" in
  x86_64 | amd64) arch=x86_64 ;;
  aarch64 | arm64) arch=aarch64 ;;
  *) echo "不支持的架构：$(uname -m)（只有 x86_64 和 aarch64 的包）" >&2; exit 1 ;;
esac

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
if [[ $version == upload:* ]]; then
  # 本地传上来的包：按本地算的哈希核对，传的过程中没坏、没被换
  package=${version#upload:}
  echo "$expected  $package" | sha256sum -c -
  tar -xzf "$package" -C "$work"
  rm -f "$package"
  tag="（传上来的包）"
else
  if [[ $version == latest ]]; then
    tag=$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest" | grep -m1 '"tag_name"' | cut -d'"' -f4)
  else
    tag=v${version#v}
  fi
  file="meshora-server-${tag#v}-linux-$arch.tar.gz"
  base="https://github.com/$repo/releases/download/$tag"
  echo "==> 下载 $file（$tag）"
  curl -fsSL -o "$work/$file" "$base/$file"
  curl -fsSL -o "$work/SHA256SUMS.txt" "$base/SHA256SUMS.txt"
  # 只核对这一个文件；SHA256SUMS.txt 里没有它也算失败
  (cd "$work" && grep " $file\$" SHA256SUMS.txt | sha256sum -c -)
  tar -xzf "$work/$file" -C "$work"
fi
dirs=("$work"/meshora-server-*-linux-"$arch")
if [[ ! -x ${dirs[0]}/meshora-coord ]]; then
  echo "包里没有 $arch 的 meshora-coord：架构对不上？" >&2
  exit 1
fi
name=$(basename "${dirs[0]}")
work_bin="$work/$name"
# --help 打完用法后以非零退出（和别的参数错误一样）：先存下来再查，免得 pipefail 把它当成失败
usage=$("$work_bin/meshora-coord" --help 2>&1 || true)
if ! grep -q -- '--hub' <<<"$usage"; then
  echo "$tag 的 meshora-coord 还不支持 --hub（托管很多网络）：换一个更新的版本" >&2
  exit 1
fi

echo "==> 安装到 /usr/local/bin"
install -m 755 "$work_bin/meshora-coord" "$work_bin/meshorad" /usr/local/bin/

id meshora >/dev/null 2>&1 || useradd --system --home-dir /var/lib/meshora --shell /usr/sbin/nologin meshora
install -d -o meshora -g meshora -m 700 /var/lib/meshora
if [[ ! -f /var/lib/meshora/coord.key ]]; then
  runuser -u meshora -- /usr/local/bin/meshorad genkey /var/lib/meshora/coord.key >/dev/null
  echo "==> 生成了服务器私钥 /var/lib/meshora/coord.key（换了它，所有网络码都要重发）"
fi

cat >/etc/systemd/system/meshora-hub.service <<UNIT
[Unit]
Description=Meshora 协调服务（托管很多网络）
After=network-online.target
Wants=network-online.target

[Service]
User=meshora
Group=meshora
ExecStart=/usr/local/bin/meshora-coord --key /var/lib/meshora/coord.key \\
  --listen 0.0.0.0:7443 --probe 0.0.0.0:7443 --probe-public $public:7443 \\
  --relay-listen 0.0.0.0:7444 --relay-public $public:7444 \\
  --hub /var/lib/meshora
Restart=on-failure
RestartSec=3
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
ReadWritePaths=/var/lib/meshora
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
UNIT

if command -v ufw >/dev/null && ufw status | grep -q '^Status: active'; then
  ufw allow 7443/tcp >/dev/null
  ufw allow 7443/udp >/dev/null
  ufw allow 7444/tcp >/dev/null
  echo "==> ufw 放行了 TCP 7443、UDP 7443、TCP 7444"
fi

systemctl daemon-reload
systemctl enable meshora-hub.service >/dev/null
systemctl restart meshora-hub.service
sleep 2
systemctl is-active --quiet meshora-hub.service || {
  journalctl -u meshora-hub.service -n 30 --no-pager >&2
  exit 1
}

key=$(/usr/local/bin/meshora-coord pubkey /var/lib/meshora/coord.key)
echo
echo "服务器地址（填进客户端）：$key@$public:7443"
echo "日志：journalctl -u meshora-hub -f"
echo "别忘了在云厂商的安全组里放行 TCP 7443、UDP 7443、TCP 7444"
REMOTE
)

script=/tmp/meshora-deploy-$$.sh
expected=
if [[ -f $version ]]; then
  expected=$( (sha256sum "$version" 2>/dev/null || shasum -a 256 "$version") | cut -d' ' -f1)
  echo "==> 上传 $(basename "$version")（SHA-256 $expected）"
  scp -q "$version" "$target:/tmp/meshora-server-$$.tar.gz"
  version="upload:/tmp/meshora-server-$$.tar.gz"
fi
ssh "$target" "cat > $script" <<<"$remote"
ssh -t "$target" "sudo bash $script '$public' '$version' '$expected'; status=\$?; rm -f $script; exit \$status"
