#!/usr/bin/env bash
# Phase 5 build pipeline: cross-compile the shell (libmandala_shell.so) and all
# signed modules for aarch64 Android, then stage a ready-to-package tree:
#
#   dist/android/
#     jniLibs/arm64-v8a/libmandala_shell.so
#     AndroidManifest.xml
#     mandala-home/{modules,trust,themes}   (push this to the device)
#     themes/*.json
#
# Requirements: rustup target `aarch64-linux-android`, `cargo-ndk`, and
# ANDROID_NDK_HOME (or NDK_HOME) pointing at an installed NDK.
#
#   ANDROID_NDK_HOME=$HOME/Android/Sdk/ndk/26.1.10909125 scripts/android-build.sh
#
# Override the target/ABI with ABI=arm64-v8a TARGET=aarch64-linux-android.
set -euo pipefail
cd "$(dirname "$0")/.."

ABI="${ABI:-arm64-v8a}"
TARGET="${TARGET:-aarch64-linux-android}"
PROFILE="${PROFILE:-release}"
OUT="dist/android"
KEYS="${MANDALA_KEYS:-$PWD/dev-keys}"
FLAG=""
[ "$PROFILE" = "release" ] && FLAG="--release"

if ! command -v cargo-ndk >/dev/null 2>&1; then
  echo "error: cargo-ndk is not installed. Run: cargo install cargo-ndk" >&2
  exit 1
fi
if [ -z "${ANDROID_NDK_HOME:-${NDK_HOME:-}}" ]; then
  echo "error: set ANDROID_NDK_HOME to an installed Android NDK" >&2
  exit 1
fi
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-${NDK_HOME:-}}"
if ! rustup target list --installed | grep -qx "$TARGET"; then
  echo "error: rustup target $TARGET is missing. Run: rustup target add $TARGET" >&2
  exit 1
fi

echo "==> building libmandala_shell.so for $ABI ($PROFILE)"
cargo ndk -t "$ABI" build --lib $FLAG -p mandala-shell --features android

echo "==> building modules for $ABI"
cargo ndk -t "$ABI" build --lib $FLAG \
  -p app-notes -p app-calculator -p app-terminal \
  -p app-aegis -p app-vault -p app-localsend

echo "==> building host signing tool"
cargo build $FLAG -p mandala-sign

REL="target/$PROFILE"
OUTLIB="target/$TARGET/$PROFILE"
rm -rf "$OUT"
mkdir -p "$OUT/jniLibs/$ABI" "$OUT/mandala-home/trust" "$OUT/mandala-home/themes"
cp android/AndroidManifest.xml "$OUT/"
cp "$OUTLIB/libmandala_shell.so" "$OUT/jniLibs/$ABI/"
cp themes/*.json "$OUT/themes/"

if [ ! -f "$KEYS/host.key" ]; then
  echo "==> generating a signing key in $KEYS (keep host.key secret)"
  "$REL/mandala-sign" keygen "$KEYS"
fi
cp "$KEYS/host.pub" "$OUT/mandala-home/trust/host.pub"
cp themes/*.json "$OUT/mandala-home/themes/"

echo "==> signing + installing modules for the device"
"$REL/mandala-sign" install --home "$OUT/mandala-home" --key "$KEYS/host.key" \
  "$OUTLIB"/libapp_notes.so "$OUTLIB"/libapp_calculator.so \
  "$OUTLIB"/libapp_terminal.so "$OUTLIB"/libapp_aegis.so \
  "$OUTLIB"/libapp_vault.so "$OUTLIB"/libapp_localsend.so

cat <<EOF

Staged $OUT/.

Next steps (Android SDK required, not automated here):
  1. Put AndroidManifest.xml + jniLibs/ into a Gradle module (or use cargo-apk)
     and build the APK; the activity is android.app.NativeActivity with
     android.app.lib_name=mandala_shell.
  2. Install and push the module tree:
       adb push $OUT/mandala-home /sdcard/Android/data/org.mandala.launcher/files
     (or wherever the app's internal_data_path resolves).
  3. On first run grant overlay access with the \`overlay\` command and set a
     wallpaper with \`wallpaper /sdcard/Pictures/pic.png\`.

This script cross-compiles only. The JNI wallpaper/overlay paths have not been
exercised on a device in this repository's CI; treat them as needing a smoke
test before shipping.
EOF
