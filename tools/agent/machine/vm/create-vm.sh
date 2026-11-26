#!/usr/bin/env bash
# Creates the Docker verification VM from scratch in $TOKULY_DEV_HOME/vm (Debian 13 cloud image,
# a fresh SSH key, cloud-init seed), boots it and installs Docker Engine. Run once per machine.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
vm=${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/vm
mkdir -p "$vm" && cd "$vm"
cp "$here"/start-vm.sh "$here"/ssh-vm.sh "$here"/provision-vm.sh .
[ -f id_vm ] || ssh-keygen -q -t ed25519 -N '' -C tokuly-vm -f id_vm
[ -f debian13.qcow2 ] || curl -fsSL -o debian13.qcow2 \
  https://cloud.debian.org/images/cloud/trixie/latest/debian-13-genericcloud-amd64.qcow2
[ -f disk.qcow2 ] || { qemu-img create -q -f qcow2 -F qcow2 -b debian13.qcow2 disk.qcow2 60G; }
cat > user-data <<CLOUD
#cloud-config
hostname: tokuly-vm
users:
  - name: dev
    sudo: ALL=(ALL) NOPASSWD:ALL
    shell: /bin/bash
    ssh_authorized_keys:
      - $(cat id_vm.pub)
ssh_pwauth: false
growpart:
  mode: auto
  devices: ['/']
runcmd:
  - [ sh, -c, "echo cloud-init-done > /var/tmp/ready" ]
CLOUD
printf 'instance-id: tokuly-vm\nlocal-hostname: tokuly-vm\n' > meta-data
cloud-localds seed.iso user-data meta-data
./start-vm.sh && ./provision-vm.sh
