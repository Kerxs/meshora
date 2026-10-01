#!/usr/bin/env bash
# 在（模拟器或真机）上装 Meshora 的 APK、打开，看它起得来、不崩。CI 的 android-emulator job 用它。
#
#   scripts/android-smoke.sh <APK> <输出目录> [网络码文件]
#
# 给了网络码文件就再走一步：等文件出现（另一边的测试建好网络写进来），点界面走完引导、贴网络码加入，
# 看 VpnService 建好网卡、连上。ping 它、确认它回应的是另一边的测试（an_android_phone_joins_and_answers_a_ping）。
#
# 输出目录里留下截图、界面树和 logcat，出了问题拿来看。
set -euo pipefail

apk=$1
out=$2
code_file=${3:-}
package=io.github.kerxs.meshora
mkdir -p "$out"

finish() {
  adb exec-out screencap -p > "$out/last.png" 2>/dev/null || true
  adb logcat -d > "$out/logcat.txt" 2>/dev/null || true
}
trap finish EXIT

adb wait-for-device
adb install -r "$apk"
# 预先给 VPN 权限：模拟器上没人去点系统对话框
adb shell appops set "$package" ACTIVATE_VPN allow || true
adb logcat -c
adb shell monkey -p "$package" -c android.intent.category.LAUNCHER 1 >/dev/null

# 界面（WebView）起来要一会儿
sleep 20
adb exec-out screencap -p > "$out/launch.png"

pid=$(adb shell pidof "$package" || true)
if [[ -z $pid ]]; then
  echo "Meshora 没在跑：多半是起来就崩了，看 $out/logcat.txt" >&2
  exit 1
fi
if adb logcat -d | grep -qE "FATAL EXCEPTION|panic："; then
  echo "日志里有崩溃，看 $out/logcat.txt" >&2
  exit 1
fi
echo "Meshora 在跑（pid $pid）"

[[ -n $code_file ]] || exit 0

# ---------- 加入网络 ----------

# 按文字找界面上的东西，点它的中心。WebView 的内容在无障碍树里，uiautomator 拿得到
tap() {
  local wanted=$1 point
  for _ in $(seq 1 20); do
    # 被系统挪到后台（或者被结束）了就再打开：界面会接着上次的进度
    if ! adb shell dumpsys activity activities | grep -q "topResumedActivity.*$package"; then
      echo "Meshora 不在前台，重新打开"
      adb shell monkey -p "$package" -c android.intent.category.LAUNCHER 1 >/dev/null || true
      sleep 5
    fi
    adb shell uiautomator dump /sdcard/ui.xml >/dev/null 2>&1 || true
    adb shell cat /sdcard/ui.xml > "$out/ui.xml" 2>/dev/null || true
    point=$(python3 - "$wanted" "$out/ui.xml" <<'PY'
import re, sys
import xml.etree.ElementTree as ET
wanted, path = sys.argv[1], sys.argv[2]
try:
    root = ET.parse(path).getroot()
except Exception:
    sys.exit(0)
for node in root.iter("node"):
    label = (node.get("text") or node.get("content-desc") or "").strip()
    # 按钮里有标题和说明时，无障碍文字是两段连在一起的："加入朋友的网络 朋友已经发给我一个网络码"
    if label == wanted or label.split(" ")[0] == wanted:
        x1, y1, x2, y2 = map(int, re.findall(r"\d+", node.get("bounds", "")))
        if x2 > x1 and y2 > y1:
            print((x1 + x2) // 2, (y1 + y2) // 2)
            break
PY
)
    if [[ -n $point ]]; then
      echo "点「$wanted」：$point"
      adb shell input tap $point
      return 0
    fi
    sleep 2
  done
  echo "界面上一直找不到「$wanted」，看 $out/ui.xml" >&2
  return 1
}

echo "等网络码…"
for _ in $(seq 1 300); do
  [[ -s $code_file ]] && break
  sleep 2
done
[[ -s $code_file ]] || { echo "一直没等到网络码" >&2; exit 1; }
code=$(cat "$code_file")

tap "下一步"
sleep 2
tap "加入朋友的网络"
sleep 2
# 引导里选了"加入"，网络码的框已经拿到焦点
adb shell input text "'$code'"
sleep 1
# 收起软键盘（有的话），免得挡住按钮
adb shell input keyevent 111 || true
sleep 1
adb exec-out screencap -p > "$out/code.png"
tap "加入"

echo "等连上…"
for _ in $(seq 1 60); do
  if adb logcat -d -s Meshora | grep -q "网卡已建好"; then
    break
  fi
  sleep 2
done
adb exec-out screencap -p > "$out/joined.png"
adb logcat -d -s Meshora | tail -40
if ! adb logcat -d -s Meshora | grep -q "网卡已建好"; then
  echo "VpnService 的网卡没建起来" >&2
  exit 1
fi
# 留时间给另一边 ping
sleep 60
echo "安卓这边走完了"
