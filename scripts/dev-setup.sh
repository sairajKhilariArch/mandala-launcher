#!/usr/bin/env bash
# Build everything, create a dev signing key, install + sign the example
# modules and the trusted public key into a local Mandala home.
#
#   scripts/dev-setup.sh            # uses ./mandala-home and ./dev-keys
#   MANDALA_HOME=/path scripts/dev-setup.sh
set -euo pipefail
cd "$(dirname "$0")/.."

HOME_DIR="${MANDALA_HOME:-$PWD/mandala-home}"
KEY_DIR="${MANDALA_KEYS:-$PWD/dev-keys}"
PROFILE="${PROFILE:-release}"
FLAG=""; [ "$PROFILE" = "release" ] && FLAG="--release"

echo "==> building workspace ($PROFILE)"
cargo build $FLAG --workspace

TARGET="target/$PROFILE"
case "$(uname -s)" in
  Darwin) EXT=dylib ;;
  MINGW*|MSYS*|CYGWIN*) EXT=dll ;;
  *) EXT=so ;;
esac

if [ ! -f "$KEY_DIR/host.key" ]; then
  echo "==> generating dev signing key in $KEY_DIR (keep host.key secret; it is git-ignored)"
  "$TARGET/mandala-sign" keygen "$KEY_DIR"
fi

mkdir -p "$HOME_DIR/trust" "$HOME_DIR/modules" "$HOME_DIR/themes"
cp "$KEY_DIR/host.pub" "$HOME_DIR/trust/host.pub"
cp -n themes/*.json "$HOME_DIR/themes/" 2>/dev/null || true

echo "==> signing + installing modules into $HOME_DIR/modules"
# Every module-backed slot (14 of the 15; A3 All Apps is built into the host).
"$TARGET/mandala-sign" install --home "$HOME_DIR" --key "$KEY_DIR/host.key" \
  "$TARGET/libapp_notes.$EXT" "$TARGET/libapp_calculator.$EXT" \
  "$TARGET/libapp_terminal.$EXT" "$TARGET/libapp_aegis.$EXT" \
  "$TARGET/libapp_vault.$EXT" "$TARGET/libapp_localsend.$EXT" \
  "$TARGET/libapp_weather.$EXT" "$TARGET/libapp_rss.$EXT" \
  "$TARGET/libapp_tasks.$EXT" "$TARGET/libapp_reader.$EXT" \
  "$TARGET/libapp_gallery.$EXT" "$TARGET/libapp_chat.$EXT" \
  "$TARGET/libapp_ai.$EXT" "$TARGET/libapp_camera.$EXT"

cat <<EOF

Done. Run the launcher with:
  MANDALA_HOME="$HOME_DIR" $TARGET/mandala-shell
EOF
