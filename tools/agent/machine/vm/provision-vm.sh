#!/usr/bin/env bash
# Waits for the VM's SSH, then installs Docker CE + compose plugin inside it.
cd "$(dirname "$0")"
for i in $(seq 1 120); do
  if ./ssh-vm.sh true 2>/dev/null; then break; fi
  sleep 10
done
./ssh-vm.sh true || { echo "SSH never came up"; tail -c 400 serial.log; exit 1; }
echo "ssh up after ~$((i*10))s"
./ssh-vm.sh 'bash -s' <<'REMOTE'
set -e
for i in $(seq 1 60); do [ -f /var/tmp/ready ] && break; sleep 5; done
export DEBIAN_FRONTEND=noninteractive
sudo apt-get update -qq
sudo apt-get install -y -qq ca-certificates curl rsync jq >/dev/null
sudo install -m 0755 -d /etc/apt/keyrings
sudo curl -fsSL https://download.docker.com/linux/debian/gpg -o /etc/apt/keyrings/docker.asc
echo "deb [arch=amd64 signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/debian trixie stable" | sudo tee /etc/apt/sources.list.d/docker.list >/dev/null
sudo apt-get update -qq
sudo apt-get install -y -qq docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin >/dev/null
sudo usermod -aG docker dev
sudo systemctl enable --now docker
sudo docker version --format 'engine {{.Server.Version}}'
sudo docker compose version
sudo docker buildx version
df -h / | tail -1; nproc; free -h | sed -n 2p
REMOTE
echo "provision exit=$?"
