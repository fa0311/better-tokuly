#!/usr/bin/env bash
# Installs what tools/agent/android.sh needs into $TOKULY_DEV_HOME (survives container recreates):
# a JDK 17, the Android SDK with the emulator and the CI's system image (API 34, google_apis, x86_64),
# the AVDs `tokuly` (Pixel 6) and `tokuly-small` (320x640 mdpi, for `android.sh small`) and Maestro. Needs /dev/kvm to be useful.
: "${TOKULY_DEV_HOME:=$HOME/tokuly-dev}"
set -euo pipefail
D=${TOKULY_DEV_HOME:-$HOME/tokuly-dev}
mkdir -p "$D/jdk" "$D/android/cmdline-tools" "$D/android/avd"
if [ ! -x "$D/jdk/current/bin/javac" ]; then
  curl -fsSL -o "$D/jdk/jdk.tgz" "https://api.adoptium.net/v3/binary/latest/17/ga/linux/x64/jdk/hotspot/normal/eclipse"
  tar xzf "$D/jdk/jdk.tgz" -C "$D/jdk" && rm "$D/jdk/jdk.tgz"
  ln -sfn "$(ls -d "$D"/jdk/jdk-17* | head -1)" "$D/jdk/current"
fi
export JAVA_HOME=$D/jdk/current ANDROID_HOME=$D/android ANDROID_AVD_HOME=$D/android/avd
if [ ! -x "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" ]; then
  curl -fsSL -o /tmp/clt.zip https://dl.google.com/android/repository/commandlinetools-linux-11076708_latest.zip
  unzip -q /tmp/clt.zip -d /tmp/clt && mv /tmp/clt/cmdline-tools "$ANDROID_HOME/cmdline-tools/latest" && rm -rf /tmp/clt /tmp/clt.zip
fi
sm=$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager
yes | "$sm" --licenses >/dev/null || true
"$sm" --install platform-tools emulator "system-images;android-34;google_apis;x86_64" >/dev/null
echo no | "$ANDROID_HOME/cmdline-tools/latest/bin/avdmanager" create avd -n tokuly \
  -k "system-images;android-34;google_apis;x86_64" -d pixel_6 --force >/dev/null
echo no | "$ANDROID_HOME/cmdline-tools/latest/bin/avdmanager" create avd -n tokuly-small \
  -k "system-images;android-34;google_apis;x86_64" --force >/dev/null
c=$ANDROID_AVD_HOME/tokuly-small.avd/config.ini
sed -i '/^hw\.\(lcd\.width\|lcd\.height\|lcd\.density\|ramSize\)=/d' "$c"
printf 'hw.lcd.width=320\nhw.lcd.height=640\nhw.lcd.density=160\nhw.ramSize=1536M\n' >>"$c"
[ -x "$HOME/.maestro/bin/maestro" ] || curl -fsSL https://get.maestro.mobile.dev | bash
echo "ready: $(dirname "$0")/../android.sh start"
