#!/usr/bin/env bash
# Re-installs the system packages this sandbox needs after the container is recreated
# (the home directory survives a restart; anything installed with apt does not).
#   $TOKULY_DEV_HOME/provision.sh && tools/dev/infra.sh up
: "${TOKULY_DEV_HOME:=$HOME/tokuly-dev}"
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive

sudo -E apt-get update -qq
sudo -E apt-get install -y -qq --no-install-recommends \
  postgresql-18 postgresql-18-postgis-3 postgresql-18-pgvector postgresql-18-h3 postgresql-18-cron postgresql-18-partman \
  valkey-server docker.io docker-compose-v2 docker-buildx \
  clang lld mold pkg-config libssl-dev cmake ninja-build jq unzip rsync tini \
  qemu-system-x86 qemu-utils cloud-image-utils genisoimage ovmf openssh-client

# The packaged services must not grab the default ports; tools/dev/infra.sh runs its own instances.
sudo service postgresql stop >/dev/null 2>&1 || true
sudo service valkey-server stop >/dev/null 2>&1 || true

# buf lives in the persistent tool directory; the proto crate's build script needs it on PATH.
BUF_VERSION=1.75.0
BUF="${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/bin/buf"
if [ ! -x "$BUF" ] || [ "$("$BUF" --version 2>/dev/null)" != "$BUF_VERSION" ]; then
  curl -fsSL -o "$BUF" "https://github.com/bufbuild/buf/releases/download/v${BUF_VERSION}/buf-Linux-x86_64"
  chmod 0755 "$BUF"
fi
sudo ln -sf "$BUF" /usr/local/bin/buf

# The Rust version of backend/rust-toolchain.toml: RUSTUP_HOME belongs to root here, so cargo
# cannot install it on first use.
RUST_VERSION=$(sed -n 's/^channel = "\(.*\)"/\1/p' "$(dirname "$(readlink -f "$0")")/../../../backend/rust-toolchain.toml")
sudo env RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" "$(command -v rustup)" toolchain install "$RUST_VERSION" \
  --profile minimal -c clippy -c rustfmt >/dev/null

echo "provisioned: $(/usr/lib/postgresql/18/bin/postgres --version), $(valkey-server --version | cut -d' ' -f1-3), buf $(buf --version), rust $RUST_VERSION, $(qemu-system-x86_64 --version | head -1)"
