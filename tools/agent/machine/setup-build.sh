#!/usr/bin/env bash
# Shared Rust build set-up for many agents on one machine: sccache (60 GB cache) behind rustc-gate,
# which limits concurrent compilations machine-wide, wired into every cargo build via ~/.cargo/config.toml.
: "${TOKULY_DEV_HOME:=$HOME/tokuly-dev}"
set -euo pipefail
D=${TOKULY_DEV_HOME:-$HOME/tokuly-dev}
mkdir -p "$D/libexec" "$HOME/.cargo/bin" "$HOME/.config/sccache"
# The real binary must keep the name `sccache` (see rustc-gate).
[ -x "$D/libexec/sccache" ] || cargo install --locked --root "$D/libexec-build" sccache && \
  { [ -x "$D/libexec/sccache" ] || mv "$D/libexec-build/bin/sccache" "$D/libexec/sccache"; }
install -m 755 "$(dirname "$0")/rustc-gate" "$HOME/.cargo/bin/sccache"
printf '[cache.disk]\ndir = "%s/.cache/sccache"\nsize = 64424509440\n' "$HOME" > "$HOME/.config/sccache/config"
grep -q '^rustc-wrapper' "$HOME/.cargo/config.toml" 2>/dev/null || cat >> "$HOME/.cargo/config.toml" <<CFG

[build]
# A share per build; the machine-wide limit is the gate.
jobs = 6
rustc-wrapper = "$HOME/.cargo/bin/sccache"
CFG
echo "done: builds need CARGO_INCREMENTAL=0 for the cache to work"
