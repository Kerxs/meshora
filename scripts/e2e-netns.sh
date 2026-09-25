#!/usr/bin/env bash
# 端到端测试：用网络命名空间模拟几台机器，各跑一个 meshorad，用真的 ping 验证加密隧道。
#
# 拓扑一：两台机器在同一个网段
#
#   机器 A（msh-e2e-a）                   机器 B（msh-e2e-b）
#   10.99.0.1 ──────── 一根网线（veth）──────── 10.99.0.2
#   协调服务 + 中继 + meshorad             meshorad
#
# 拓扑二：两台机器各自在一个 NAT 路由器后面，协调服务和中继在"公网"上
#
#                    服务器（msh-e2e-srv）192.0.2.1
#                    协调服务 + 中继；它的网桥就是"公网"
#               ┌──────────────┴──────────────┐
#     NAT 路由器（msh-e2e-nat-a）       NAT 路由器（msh-e2e-nat-b）
#     公网 192.0.2.2，内网 10.1.0.1      公网 192.0.2.3，内网 10.2.0.1
#               │                             │
#     机器 A（msh-e2e-a）10.1.0.2       机器 B（msh-e2e-b）10.2.0.2
#
# NAT 路由器按家用路由器的典型配置：出去的做 MASQUERADE，进来的只放行已经建立的连接。
# "公网"不路由内网地址，所以两边上报的局域网端点互相够不着，想直连只能靠打洞。
#
# 场景（overlay 地址按名单顺序分配：A 是 100.64.0.1，B 是 100.64.0.2）：
#   1. 同一网段：走直连
#   2. 同一网段，--relay-only：全程经中继
#   3. 两边都是普通 NAT（外部端口不随目标变）：打洞成功，走直连。
#      然后掐断两个 NAT 之间的 UDP：回落中继；恢复之后切回直连
#   4. 同样的两边，A 的路由器换了公网地址（运营商重新分配）：A 的连接全断，
#      重连、重新探测到新的公网端点、重新打洞，回到直连
#   5. B 在对称 NAT 后面（每个目标换一个外部端口）：打洞打不通，经中继照样通
#
# 需要 root（建命名空间和虚拟网卡）、iproute2、iptables、ping。先编译：
#   cargo build -p meshorad -p meshora-coord
#   sudo scripts/e2e-netns.sh [可执行文件所在目录，默认 target/debug]
set -euo pipefail

BIN=$(cd "${1:-target/debug}" && pwd)
WORK=$(mktemp -d)
PIDS=()
NAMESPACES=()

stop_all() {
  for pid in "${PIDS[@]}"; do kill "$pid" 2>/dev/null || true; done
  for pid in "${PIDS[@]}"; do wait "$pid" 2>/dev/null || true; done
  PIDS=()
}

del_namespaces() {
  for ns in "${NAMESPACES[@]}"; do ip netns del "$ns" 2>/dev/null || true; done
  NAMESPACES=()
}

cleanup() {
  stop_all
  del_namespaces
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

new_ns() {
  ip netns add "$1"
  NAMESPACES+=("$1")
  ip -n "$1" link set lo up
}

# 拓扑一。协调服务跑在机器 A 上
setup_lan() {
  new_ns msh-e2e-a
  new_ns msh-e2e-b
  ip link add veth-a netns msh-e2e-a type veth peer name veth-b netns msh-e2e-b
  ip -n msh-e2e-a addr add 10.99.0.1/24 dev veth-a
  ip -n msh-e2e-b addr add 10.99.0.2/24 dev veth-b
  ip -n msh-e2e-a link set veth-a up
  ip -n msh-e2e-b link set veth-b up
  COORD_NS=msh-e2e-a
  COORD_IP=10.99.0.1
}

# 拓扑二。参数是 B 那边的 NAT 类型：cone（普通）或 symmetric（对称）。A 那边总是普通的
setup_nat() {
  new_ns msh-e2e-srv
  ip -n msh-e2e-srv link add br0 type bridge
  ip -n msh-e2e-srv addr add 192.0.2.1/24 dev br0
  ip -n msh-e2e-srv link set br0 up
  nat_side a 192.0.2.2 10.1.0 cone
  nat_side b 192.0.2.3 10.2.0 "$1"
  COORD_NS=msh-e2e-srv
  COORD_IP=192.0.2.1
}

# nat_side <a 或 b> <路由器的公网地址> <内网前缀> <cone 或 symmetric>
nat_side() {
  local side=$1 public=$2 lan=$3 kind=$4
  local router=msh-e2e-nat-$side host=msh-e2e-$side
  new_ns "$router"
  new_ns "$host"

  # 路由器的公网口接到服务器的网桥上
  ip link add wan netns "$router" type veth peer name "to-$side" netns msh-e2e-srv
  ip -n msh-e2e-srv link set "to-$side" master br0
  ip -n msh-e2e-srv link set "to-$side" up
  ip -n "$router" addr add "$public/24" dev wan
  ip -n "$router" link set wan up

  # 路由器和它后面的机器
  ip link add lan netns "$router" type veth peer name eth0 netns "$host"
  ip -n "$router" addr add "$lan.1/24" dev lan
  ip -n "$host" addr add "$lan.2/24" dev eth0
  ip -n "$router" link set lan up
  ip -n "$host" link set eth0 up
  ip -n "$host" route add default via "$lan.1"

  ip netns exec "$router" sysctl -qw net.ipv4.ip_forward=1
  # --random-fully：每条新连接随机挑外部端口，也就是对称 NAT
  local masquerade=(-t nat -A POSTROUTING -o wan -j MASQUERADE)
  if [ "$kind" = symmetric ]; then
    masquerade+=(--random-fully)
  fi
  ip netns exec "$router" iptables "${masquerade[@]}"
  # 进来的只放行已经建立的连接。没被放行的报文在 conntrack 登记之前就丢了，
  # 不会占住对面打洞要用的那个端口映射
  for chain in INPUT FORWARD; do
    ip netns exec "$router" iptables -A "$chain" -i wan \
      -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
    ip netns exec "$router" iptables -A "$chain" -i wan -j DROP
  done
}

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

# 某台机器最后选定的路径：Direct、Relay，还没选过就是空的
last_path() {
  grep "切换路径" "$WORK/$1.log" | grep -o "path=[A-Za-z]*" | tail -n 1 | cut -d= -f2 || true
}

# 最后选定的路径的完整写法，带地址
last_path_detail() {
  grep "切换路径" "$WORK/$1.log" | tail -n 1 | sed 's/.*path=//' || true
}

# 启动协调服务（顺带跑中继）和两边的 meshorad，等隧道通。中继先行，所以不管最后走哪条路，
# 隧道都应该很快就通。参数原样传给两边的 meshorad
start_nodes() {
  ip netns exec "$COORD_NS" "$BIN/meshora-coord" --key "$WORK/coord.key" \
    --listen "$COORD_IP:7443" --probe "$COORD_IP:7443" --relay-listen "$COORD_IP:7444" \
    --node "$A" --node "$B" > "$WORK/coord.log" 2>&1 &
  PIDS+=($!)
  wait_for_port "$COORD_NS" "$COORD_IP" 7443

  for side in a b; do
    ip netns exec "msh-e2e-$side" "$BIN/meshorad" up --key "$WORK/$side.key" \
      --coord "$COORD_IP:7443" --coord-key "$COORD" --tun msh0 "$@" > "$WORK/$side.log" 2>&1 &
    PIDS+=($!)
  done

  for _ in $(seq 1 20); do
    if ip netns exec msh-e2e-a ping -c 1 -W 1 100.64.0.2 > /dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  fail "$SCENARIO：20 秒内没 ping 通"
}

show_paths() {
  echo "A 选的路径：$(last_path_detail a)"
  echo "B 选的路径：$(last_path_detail b)"
}

# wait_for_path <Direct 或 Relay> <最多等几秒>：等到两边最后选定的路径都是这种
wait_for_path() {
  local expected=$1 timeout=$2 started=$SECONDS path_a path_b
  while :; do
    path_a=$(last_path a)
    path_b=$(last_path b)
    if [ "$path_a" = "$expected" ] && [ "$path_b" = "$expected" ]; then
      break
    fi
    if [ $((SECONDS - started)) -ge "$timeout" ]; then
      fail "$SCENARIO：$timeout 秒内两边没都切到 $expected，A 选了 ${path_a:-无}，B 选了 ${path_b:-无}"
    fi
    sleep 1
  done
  echo "用了约 $((SECONDS - started)) 秒，两边都是 $expected"
  show_paths
}

# hold_path <Direct 或 Relay> <几秒>：这段时间里两边一直是这种路径。
# 用来确认"没切走"，所以时长得长过一轮打洞（探测 3 秒一轮，打洞请求 10 秒一次）
hold_path() {
  local expected=$1 duration=$2 started=$SECONDS
  while [ $((SECONDS - started)) -lt "$duration" ]; do
    assert_path "$expected"
    sleep 1
  done
  show_paths
}

assert_path() {
  local side path
  for side in a b; do
    path=$(last_path "$side")
    [ "$path" = "$1" ] || fail "$SCENARIO：$side 应该走 $1，实际是 ${path:-无}"
  done
}

ping_both() {
  ip netns exec msh-e2e-a ping -c 3 -W 2 100.64.0.2 || fail "$SCENARIO：A → B 丢包"
  ip netns exec msh-e2e-b ping -c 3 -W 2 100.64.0.1 || fail "$SCENARIO：B → A 丢包"
}

# 等到某台机器最后选定的路径里出现某个地址
wait_for_path_to() {
  local side=$1 addr=$2 timeout=$3 started=$SECONDS path
  while :; do
    path=$(last_path_detail "$side")
    case $path in
      "Direct($addr:"*) break ;;
    esac
    if [ $((SECONDS - started)) -ge "$timeout" ]; then
      fail "$SCENARIO：$timeout 秒内 $side 没走到直连 $addr，现在是 ${path:-无}"
    fi
    sleep 1
  done
  echo "用了约 $((SECONDS - started)) 秒，$side 直连到 $addr"
}

# 在 B 的 NAT 路由器上掐断（或恢复）两边公网地址之间的 UDP：直连断了，
# 到协调服务和中继的连接不受影响
direct_link() {
  local op
  case $1 in
    cut) op=-I ;;
    restore) op=-D ;;
  esac
  for match in -s -d; do
    ip netns exec msh-e2e-nat-b iptables "$op" FORWARD -p udp "$match" 192.0.2.2 -j DROP
  done
}

begin() {
  SCENARIO=$1
  echo "== $SCENARIO"
}

# 顺带报一下两边各切换了几次路径：来回抖动的话这个数会变大
pass() {
  local a b
  a=$(grep -c "切换路径" "$WORK/a.log" || true)
  b=$(grep -c "切换路径" "$WORK/b.log" || true)
  echo "== $SCENARIO：通过（切换路径 A $a 次，B $b 次）"
  stop_all
}

setup_lan

begin "同一网段：走直连"
start_nodes
wait_for_path Direct 20
ping_both
assert_path Direct
pass

begin "同一网段，--relay-only：全程经中继"
start_nodes --relay-only
wait_for_path Relay 5
ping_both
assert_path Relay
pass

del_namespaces
setup_nat cone

begin "两边都是普通 NAT：打洞成功走直连；直连断了回落中继，恢复后切回直连"
start_nodes
wait_for_path Direct 30
ping_both
echo "掐断两个 NAT 之间的 UDP"
direct_link cut
wait_for_path Relay 40
ping_both
echo "恢复两个 NAT 之间的 UDP"
direct_link restore
wait_for_path Direct 40
ping_both
assert_path Direct
pass

del_namespaces
setup_nat cone

begin "A 的路由器换了公网地址：重连、重新探测、重新打洞，回到直连"
start_nodes
wait_for_path Direct 30
ping_both
# 地址一删，MASQUERADE 按旧地址做的映射（conntrack）随之清掉：A 经这个路由器的
# 连接全断了，就像运营商重新分配地址
echo "把 A 的路由器的公网地址从 192.0.2.2 换成 192.0.2.12"
ip -n msh-e2e-nat-a addr del 192.0.2.2/24 dev wan
ip -n msh-e2e-nat-a addr add 192.0.2.12/24 dev wan
wait_for_path_to b 192.0.2.12 60
wait_for_path Direct 30
ping_both
assert_path Direct
grep -q "已重新连上协调服务" "$WORK/a.log" || fail "$SCENARIO：A 应该重连过协调服务"
pass

del_namespaces
setup_nat symmetric

begin "B 在对称 NAT 后面：打洞打不通，经中继照样通"
start_nodes
hold_path Relay 20
ping_both
assert_path Relay
pass

del_namespaces
echo "全部通过"
