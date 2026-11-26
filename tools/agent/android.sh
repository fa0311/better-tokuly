#!/usr/bin/env bash
# The local Android emulator (needs /dev/kvm; the container has it since 2026-11-26).
# Same device as CI: API 34, google_apis, x86_64, Pixel 6, headless, software GPU.
#   android.sh start | stop | status | env | locale <ja-JP|en-US> | shot <file.png>
# `eval "$(android.sh env)"` puts adb, emulator and maestro on PATH.
# Everything lives in the home volume (~/tokuly-dev/android, ~/.maestro), so it survives a
# container recreate; only one emulator runs at a time (port 5554), shared by everyone.
set -euo pipefail
A=${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/android
export ANDROID_HOME=$A ANDROID_SDK_ROOT=$A ANDROID_AVD_HOME=$A/avd JAVA_HOME=${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/jdk/current
PATH=$JAVA_HOME/bin:$A/platform-tools:$A/emulator:$HOME/.maestro/bin:$PATH
PID=${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/run/emulator.pid
running() { [ -f "$PID" ] && kill -0 "$(cat "$PID")" 2>/dev/null; }
case "${1:-status}" in
  env)
    printf 'export ANDROID_HOME=%s ANDROID_SDK_ROOT=%s ANDROID_AVD_HOME=%s/avd JAVA_HOME=%s\n' "$A" "$A" "$A" "$JAVA_HOME"
    printf 'export PATH=%s/bin:%s/platform-tools:%s/emulator:%s/.maestro/bin:$PATH\n' "$JAVA_HOME" "$A" "$A" "$HOME" ;;
  start)
    if running; then echo "emulator already running (pid $(cat "$PID"))"; exit 0; fi
    [ -w /dev/kvm ] || { echo "no /dev/kvm: the emulator would be far too slow" >&2; exit 1; }
    nohup emulator -avd tokuly -no-window -no-audio -no-boot-anim -gpu swiftshader_indirect \
      -camera-back none -no-snapshot-save >"${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/logs/emulator.log" 2>&1 &
    echo $! >"$PID"
    adb wait-for-device
    for _ in $(seq 1 100); do
      [ "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = 1 ] && { echo "emulator ready"; exit 0; }
      sleep 3
    done
    echo "emulator did not finish booting; see ${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/logs/emulator.log" >&2; exit 1 ;;
  stop)
    running && { adb emu kill >/dev/null 2>&1 || kill "$(cat "$PID")"; } ; rm -f "$PID"; echo stopped ;;
  status)
    if running; then echo "running (pid $(cat "$PID"))"; adb devices | tail -n +2; else echo "stopped"; fi ;;
  locale)
    # en-US (default, same as CI) for Maestro flows: with ja-JP, Gboard turns typed handles into kana.
    # For Japanese screenshots: sign in under en-US, then `locale ja-JP`; the app keeps its session.
    adb root >/dev/null 2>&1; sleep 2
    adb shell "setprop persist.sys.locale ${2:?ja-JP|en-US}; setprop ctl.restart zygote"
    sleep 20; adb wait-for-device
    for _ in $(seq 1 60); do [ "$(adb shell getprop sys.boot_completed | tr -d '\r')" = 1 ] && break; sleep 3; done
    echo "locale $(adb shell getprop persist.sys.locale | tr -d '\r')" ;;
  shot)
    adb exec-out screencap -p >"${2:?file.png}"; echo "${2}" ;;
  *) echo "usage: android.sh start|stop|status|env|locale <tag>|shot <file>" >&2; exit 2 ;;
esac
