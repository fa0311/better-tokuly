#!/usr/bin/env bash
# Boots the Docker verification VM (Debian 13 + Docker Engine; this container has no Docker daemon).
# With /dev/kvm (the container gets it since 2026-11-26) the VM runs at near-native speed;
# without it, QEMU falls back to software emulation (3-10x slower).
#   VM_CPUS=6 VM_MEM=12G ~/tokuly-dev/vm/start-vm.sh
set -euo pipefail
cd "$(dirname "$0")"
if [ -f qemu.pid ] && kill -0 "$(cat qemu.pid)" 2>/dev/null; then echo "VM already running (pid $(cat qemu.pid))"; exit 0; fi
FWD="hostfwd=tcp:127.0.0.1:2222-:22"
# tcp:host-guest pairs for the app; extended as the stack grows
for p in 8080:80 8443:443 11935:1935 18888:8888 18889:8889 13000:3000 19000:9000; do FWD="$FWD,hostfwd=tcp:127.0.0.1:${p%%:*}-:${p##*:}"; done
for p in 8443:443 4433:4433 18189:8189; do FWD="$FWD,hostfwd=udp:127.0.0.1:${p%%:*}-:${p##*:}"; done
if [ -r /dev/kvm ] && [ -w /dev/kvm ]; then
  ACCEL=(-accel kvm -cpu host); CPUS=${VM_CPUS:-6}; MEM=${VM_MEM:-12G}
else
  ACCEL=(-accel tcg,thread=multi,tb-size=4096 -cpu max); CPUS=${VM_CPUS:-14}; MEM=${VM_MEM:-20G}
fi
exec qemu-system-x86_64 \
  -machine q35 "${ACCEL[@]}" -smp "$CPUS" -m "$MEM" \
  -drive if=virtio,file=disk.qcow2,format=qcow2,cache=unsafe,discard=unmap \
  -drive if=virtio,file=seed.iso,format=raw,readonly=on \
  -netdev "user,id=n0,$FWD" -device virtio-net-pci,netdev=n0 \
  -device virtio-rng-pci \
  -display none -serial file:serial.log -daemonize -pidfile qemu.pid
