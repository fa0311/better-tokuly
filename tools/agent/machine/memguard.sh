#!/usr/bin/env bash
# Keeps the machine from running out of memory (which reboots it and kills every agent).
# When available memory drops below the threshold, the largest compiler process is killed:
# the build that owned it fails and is simply run again, which is far cheaper than a reboot.
# Never touches infrastructure, editors or agent processes.
#   nohup $TOKULY_DEV_HOME/memguard.sh > $TOKULY_DEV_HOME/logs/memguard.log 2>&1 &
: "${TOKULY_DEV_HOME:=$HOME/tokuly-dev}"
set -u
THRESHOLD_MB=${MEMGUARD_THRESHOLD_MB:-8000}
while sleep 2; do
  available_mb=$(awk '/MemAvailable/ {print int($2/1024)}' /proc/meminfo)
  [ "$available_mb" -ge "$THRESHOLD_MB" ] && continue
  # Largest rustc first; if there is none, the largest linker or test browser.
  victim=$(ps -eo pid,rss,comm --no-headers --sort=-rss | awk '$3 == "rustc" {print $1; exit}')
  [ -z "$victim" ] && victim=$(ps -eo pid,rss,comm --no-headers --sort=-rss | awk '$3 == "mold" || $3 == "ld" || $3 == "chrome" {print $1; exit}')
  if [ -n "$victim" ]; then
    echo "$(date '+%F %T') available ${available_mb} MB < ${THRESHOLD_MB} MB: killing $(ps -o comm=,rss= -p "$victim" | awk '{printf "%s (%.1f GB)", $1, $2/1048576}') pid $victim"
    kill -9 "$victim" 2>/dev/null
    sleep 3
  fi
done
