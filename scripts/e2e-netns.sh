#!/usr/bin/env bash
# 端到端测试：两个网络命名空间模拟两台机器，各跑一个 meshorad，用真的 ping 验证加密隧道。
#
#   机器 A（命名空间 msh-e2e-a）         机器 B（命名空间 msh-e2e-b）
#   10.99.0.1 ──────── 一根网线（veth）──────── 10.99.0.2
#   协调服务 + 中继 + meshorad            meshorad
#   overlay 100.64.0.1                    overlay 100.64.0.2
#
# 跑两个场景：能直连时走直连；--relay-only 时全程经中继。
#
# 需要 root（建命名空间和虚拟网卡）、iproute2、ping。先编译：
#   cargo build -p meshorad -p meshora-coord
#   sudo scripts/e2e-netns.sh [可执行文件所在目录，默认 target/debug]
set -euo pipefail

BIN=$(cd "${1:-target/debug}" && pwd)
WORK=$(mktemp -d)
NS_A=msh-e2e-a
NS_B=msh-e2e-b
PIDS=()

stop_all() {
  for pid in "${PIDS[@]}"; do kill "$pid" 2>/dev/null || true; done
  for pid in "${PIDS[@]}"; do wait "$pid" 2>/dev/null || true; done
  PIDS=()
}

cleanup() {
  stop_all
  ip netns del "$NS_A" 2>/dev/null || true
  ip netns del "$NS_B" 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT

show_logs() {
  for log in "$WORK"/*.log; do
    echo "---- $(basename "$log")"
    cat "$log"
  done
}

fail() {
  echo "失败：$*"
  show_logs
  exit 1
}

# 两台"机器"之间一根网线
ip netns add "$NS_A"
ip netns add "$NS_B"
ip link add veth-a netns "$NS_A" type veth peer name veth-b netns "$NS_B"
ip -n "$NS_A" addr add 10.99.0.1/24 dev veth-a
ip -n "$NS_B" addr add 10.99.0.2/24 dev veth-b
for ns in "$NS_A" "$NS_B"; do ip -n "$ns" link set lo up; done
ip -n "$NS_A" link set veth-a up
ip -n "$NS_B" link set veth-b up

for name in coord a b; do "$BIN/meshorad" genkey > "$WORK/$name.key"; done
chmod 600 "$WORK"/*.key
COORD=$("$BIN/meshorad" pubkey < "$WORK/coord.key")
A=$("$BIN/meshorad" pubkey < "$WORK/a.key")
B=$("$BIN/meshorad" pubkey < "$WORK/b.key")

# 等某个命名空间里的 TCP 端口能连上
wait_for_port() {
  local ns=$1 host=$2 port=$3
  for _ in $(seq 1 50); do
    if ip netns exec "$ns" bash -c "exec 3<>/dev/tcp/$host/$port" 2>/dev/null; then
      return 0
    fi
    sleep 0.1
  done
  fail "$host:$port 一直没起来"
}

# run_scenario <名字> <期望的路径：Direct 或 Relay> [meshorad 的额外参数...]
run_scenario() {
  local name=$1 expected=$2
  shift 2
  echo "== $name"

  ip netns exec "$NS_A" "$BIN/meshora-coord" --key "$WORK/coord.key" \
    --listen 10.99.0.1:7443 --probe 10.99.0.1:7443 --relay-listen 10.99.0.1:7444 \
    --node "$A" --node "$B" > "$WORK/coord.log" 2>&1 &
  PIDS+=($!)
  wait_for_port "$NS_A" 10.99.0.1 7443

  ip netns exec "$NS_A" "$BIN/meshorad" up --key "$WORK/a.key" --coord 10.99.0.1:7443 \
    --coord-key "$COORD" --tun msh0 "$@" > "$WORK/a.log" 2>&1 &
  PIDS+=($!)
  ip netns exec "$NS_B" "$BIN/meshorad" up --key "$WORK/b.key" --coord 10.99.0.1:7443 \
    --coord-key "$COORD" --tun msh0 "$@" > "$WORK/b.log" 2>&1 &
  PIDS+=($!)

  # 等隧道通，最多 20 秒
  local up=no
  for _ in $(seq 1 20); do
    if ip netns exec "$NS_A" ping -c 1 -W 1 100.64.0.2 > /dev/null 2>&1; then
      up=yes
      break
    fi
    sleep 1
  done
  [ "$up" = yes ] || fail "$name：20 秒内没 ping 通"

  ip netns exec "$NS_A" ping -c 3 -W 2 100.64.0.2 || fail "$name：A → B 丢包"
  ip netns exec "$NS_B" ping -c 3 -W 2 100.64.0.1 || fail "$name：B → A 丢包"

  # 两边最后选定的路径都得是期望的那种
  for log in "$WORK/a.log" "$WORK/b.log"; do
    grep "切换路径" "$log" | tail -n 1 | grep -q "path=$expected" \
      || fail "$name：$(basename "$log") 最后选定的路径不是 $expected"
  done
  echo "== $name：通了，路径是 $expected"
  stop_all
}

run_scenario "直连" Direct
run_scenario "只走中继" Relay --relay-only
echo "全部通过"
