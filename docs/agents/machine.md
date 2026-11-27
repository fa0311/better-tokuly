# The development machine

The lead agent works on a Linux container (16 cores, 60 GB RAM) with `/dev/kvm` and an NVIDIA GPU passed in,
no Docker daemon. State outside the repository lives under `$TOKULY_DEV_HOME` (default `~/tokuly-dev`) on
a persistent volume; anything installed with `apt` is lost when the container is recreated.

## After the container is recreated

```sh
tools/agent/machine/provision.sh                 # system packages (Postgres, Valkey, clang, mold, QEMU, …)
# first time on a new machine also: setup-build.sh, setup-android.sh, vm/create-vm.sh
TOKULY_DEV_MEDIAMTX=1 tools/dev/infra.sh up      # native services; NATS may take a while on a big store
nohup tools/agent/machine/memguard.sh  > $TOKULY_DEV_HOME/logs/memguard.log  2>&1 &
nohup tools/agent/machine/diskguard.sh > $TOKULY_DEV_HOME/logs/diskguard.log 2>&1 &
tools/agent/machine/dev.sh api && tools/agent/machine/dev.sh web    # dev API :58081, web :53000
tools/agent/machine/env-check.sh                 # every line must be ok
```

`dev.sh api fresh` recreates the dev database (needed after a migration was edited in place).
Map tiles for the web: `pmtiles serve $TOKULY_DEV_HOME/data/tiles --port 58090 --cors='*'`.

## What keeps parallel agents from breaking the machine

| Limit | Guard |
|---|---|
| Memory: one `rustc` on the generated API crate takes 3–6 GB | `rustc-gate` (installed as `~/.cargo/bin/sccache`, set as `rustc-wrapper`): at most 3 heavy and 14 other compilations machine-wide; `memguard.sh` kills the largest `rustc` under 8 GB free |
| Rebuilding the same dependencies per agent | sccache behind the gate, 40 GiB cache, needs `CARGO_INCREMENTAL=0` |
| Disk: an agent's build directory grows to 10–100 GB | one `CARGO_TARGET_DIR` per agent under `$TOKULY_DEV_HOME/target/`, deleted when its PR merges; `diskguard.sh` |
| Test leftovers (NATS streams, databases) | testkit removes them at exit and sweeps older runs (#196) |

## Emulator and VM (need `/dev/kvm`)

- Android: `tools/agent/machine/setup-android.sh` installs a JDK, the SDK, the API 34 x86_64 image, AVD `tokuly` and Maestro under `$TOKULY_DEV_HOME`;
  `tools/agent/android.sh start`; `android.sh small <command…>` runs a command on a 320x640 phone instead. Same device as CI, en-US by default (Gboard turns typed text into kana under ja-JP); for Japanese screenshots sign in first, then `android.sh locale ja-JP`. Release APK: `expo prebuild` then
  `./gradlew assembleRelease -PreactNativeArchitectures=x86_64`.
- Docker: a Debian VM with Docker Engine: `tools/agent/machine/vm/create-vm.sh` once, then `$TOKULY_DEV_HOME/vm/start-vm.sh` and `ssh-vm.sh '<cmd>'`.
- iOS needs macOS: that is the Mac agent's side (`agent:mac`).
