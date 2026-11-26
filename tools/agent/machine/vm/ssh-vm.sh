#!/usr/bin/env bash
exec ssh -q -i "$(dirname "$0")/id_vm" -p 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=10 -o ServerAliveInterval=30 dev@127.0.0.1 "$@"
