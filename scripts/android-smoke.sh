#!/usr/bin/env bash
# 在（模拟器或真机）上装 Meshora 的 APK、打开，看它起得来、不崩。CI 的 android-emulator job 用它。
#
#   scripts/android-smoke.sh <APK> <输出目录>
#
# 输出目录里留下截图和 logcat，出了问题拿来看。
set -euo pipefail

apk=$1
out=$2
package=io.github.kerxs.meshora
mkdir -p "$out"

adb wait-for-device
adb install -r "$apk"
# 预先给 VPN 权限：模拟器上没人去点系统对话框
adb shell appops set "$package" ACTIVATE_VPN allow || true
adb logcat -c
adb shell monkey -p "$package" -c android.intent.category.LAUNCHER 1 >/dev/null

# 界面（WebView）起来要一会儿
sleep 20
adb exec-out screencap -p > "$out/launch.png"
adb logcat -d > "$out/logcat.txt"
grep -E "Meshora|RustStdoutStderr|AndroidRuntime" "$out/logcat.txt" | tail -80 || true

pid=$(adb shell pidof "$package" || true)
if [[ -z $pid ]]; then
  echo "Meshora 没在跑：多半是起来就崩了，看 $out/logcat.txt" >&2
  exit 1
fi
if grep -qE "FATAL EXCEPTION|panic：" "$out/logcat.txt"; then
  echo "日志里有崩溃，看 $out/logcat.txt" >&2
  exit 1
fi
echo "Meshora 在跑（pid $pid）"
