#!/usr/bin/env bash
# Checks that the machine-wide build and safety set-up is actually in effect, not just
# configured somewhere. Prints one line per check; exits non-zero if any check fails.
# Run by the lead's hourly watchdog and after every restart.
: "${TOKULY_DEV_HOME:=$HOME/tokuly-dev}"
set -u
fail=0
ok()   { printf 'ok    %s\n' "$1"; }
bad()  { printf 'FAIL  %s\n' "$1"; fail=1; }
note() { printf 'note  %s\n' "$1"; }

# Every cargo build goes through the gate and sccache.
wrapper=$(cd "$(git -C "$(dirname "$(readlink -f "$0")")" rev-parse --show-toplevel)/backend" 2>/dev/null && cargo config get build.rustc-wrapper -Z unstable-options 2>/dev/null | tr -d '"' | sed 's/.*= *//')
[ -z "$wrapper" ] && wrapper=$(grep -E '^rustc-wrapper' "$HOME/.cargo/config.toml" 2>/dev/null | sed 's/.*= *"\(.*\)"/\1/')
if [ "$wrapper" = "$HOME/.cargo/bin/sccache" ]; then ok "cargo rustc-wrapper is the sccache gate"; else bad "cargo rustc-wrapper is '${wrapper:-unset}', expected $HOME/.cargo/bin/sccache"; fi
grep -q 'REAL=' "$HOME/.cargo/bin/sccache" 2>/dev/null && ok "~/.cargo/bin/sccache is the gate script" || bad "~/.cargo/bin/sccache is not the gate script"
[ -x "${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/libexec/sccache" ] && ok "real sccache present" || bad "real sccache missing at $TOKULY_DEV_HOME/libexec/sccache"
max=$("${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/libexec/sccache" --show-stats 2>/dev/null | awk '/Max cache size/{print $4,$5}')
case "$max" in "40 GiB") ok "sccache cache limit $max";; *) bad "sccache cache limit '${max:-unknown}', expected 40 GiB";; esac

# Is it used? Every real compilation (`rustc --crate-name ...`) must run under sccache: its
# parent is the sccache server (or the gate's flock). Probes from build scripts (`rustc -vV`,
# autocfg) have no --crate-name of a workspace crate and are ignored.
bypass=0
for pid in $(pgrep -x rustc 2>/dev/null); do
  args=$({ tr '\0' ' ' < "/proc/$pid/cmdline"; } 2>/dev/null) || continue   # the process may be gone
  case "$args" in *"--crate-name "*) ;; *) continue ;; esac
  case "$args" in *"/out/"*|*"build_script_"*|*"--crate-name ____"*) continue ;; esac
  parent=$(ps -o comm= -p "$(ps -o ppid= -p "$pid" | tr -d ' ')" 2>/dev/null)
  case "$parent" in sccache|flock) ;; *) bypass=$((bypass+1)); echo "      pid $pid parent '$parent': ${args:0:120}" ;; esac
done
if [ "$bypass" -eq 0 ]; then ok "every running rustc compilation goes through sccache"
else bad "$bypass rustc compilation(s) bypass sccache (see above)"; fi

# Guards.
pgrep -f 'memguard.sh' >/dev/null && ok "memguard running" || bad "memguard not running (see docs/agents/machine.md)"
pgrep -f 'diskguard.sh' >/dev/null && ok "diskguard running" || bad "diskguard not running"

# The prepaid API key (claude-api wrapper): private file, spending under the cap, never in the repo.
key="${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/secrets/anthropic-api-key"
if [ -f "$key" ]; then
  [ "$(stat -c %a "$key")" = 600 ] && ok "API key file is private (600)" || bad "API key file mode is $(stat -c %a "$key"), expected 600"
  spent=$(awk -F'\t' '{s+=$3} END {printf "%.2f", s+0}' "${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/secrets/api-ledger.tsv" 2>/dev/null)
  note "API credit used: \$${spent:-0} of the \$198 cap"
  if git -C "$(git -C "$(dirname "$(readlink -f "$0")")" rev-parse --show-toplevel)" grep -qF "$(cut -c1-24 "$key")" 2>/dev/null; then bad "the API key appears in the repository"; else ok "API key not in the repository"; fi
fi

# Facts about the machine that briefs and replies rely on.
free_gb=$(df -BG --output=avail /home | tail -1 | tr -dc 0-9)
[ "$free_gb" -ge 40 ] && ok "disk free ${free_gb} GB" || bad "disk free ${free_gb} GB (< 40)"
# KVM: the Android emulator and the Docker VM depend on it (compose: devices /dev/kvm, group 994).
if [ -r /dev/kvm ] && [ -w /dev/kvm ]; then ok "/dev/kvm usable (Android emulator, Docker VM)"
else bad "/dev/kvm missing or not writable: check the agent container's compose (devices: /dev/kvm, group_add: 994)"; fi
[ -x "${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/android/emulator/emulator" ] && [ -x "$HOME/.maestro/bin/maestro" ] \
  && ok "Android SDK, emulator and Maestro installed (tools/agent/android.sh)" \
  || bad "Android tooling missing ($TOKULY_DEV_HOME/android, ~/.maestro)"
note "emulator: $("${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/bin/android.sh" status 2>/dev/null | head -1)"
note "virtualisation: $(systemd-detect-virt 2>/dev/null || echo unknown)"
exit $fail
