#!/usr/bin/env bash
# Keeps the disk from filling up while many agents build: under the threshold, removes test
# executables in the build directories that nothing has touched for a while (cargo relinks them
# when they are needed again). Start after a reboot:
#   nohup $TOKULY_DEV_HOME/diskguard.sh > $TOKULY_DEV_HOME/logs/diskguard.log 2>&1 &
THRESHOLD_GB=${DISKGUARD_THRESHOLD_GB:-25}
while sleep 60; do
  free_gb=$(df --output=avail -BG /home | tail -1 | tr -dc 0-9)
  [ "$free_gb" -ge "$THRESHOLD_GB" ] && continue
  for age in 45 15 5; do
    find "$HOME"/tokuly-dev/target/*/debug/deps -maxdepth 1 -type f -perm -u+x ! -name '*.so' ! -name '*.rlib' -size +20M -mmin +"$age" -delete 2>/dev/null
    now_gb=$(df --output=avail -BG /home | tail -1 | tr -dc 0-9)
    echo "$(date '+%F %T') ${free_gb} GB free < ${THRESHOLD_GB} GB: removed test binaries older than ${age} min, now ${now_gb} GB"
    [ "$now_gb" -ge "$THRESHOLD_GB" ] && break
  done
done
