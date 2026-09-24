#!/usr/bin/env bash
#
# Build "Peek RS.app" and install it, so Peek can be started from Spotlight or the Dock
# instead of from a terminal.
#
#   ./scripts/package.sh                 build, then install to /Applications
#   ./scripts/package.sh --no-install    build the bundle and leave it in target/
#   ./scripts/package.sh --sign "ID"     sign with this identity instead of the one it finds
#   ./scripts/package.sh --adhoc         sign ad-hoc
#
# The name and bundle id differ from the Tauri app's (Peek.app, com.getpeek.dev) so the two
# install side by side. Both read the same ~/peek.

set -euo pipefail

readonly APP_NAME="Peek RS"
readonly BUNDLE_ID="com.getpeek.rs"
readonly EXECUTABLE="peek"
readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly BUNDLE="$ROOT/target/$APP_NAME.app"
readonly ICON_SOURCE="$ROOT/crates/peek-ui/src/dock_icon/midnight.png"

install_app=1
identity=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-install) install_app=0; shift ;;
    --sign) identity="${2:?--sign needs an identity}"; shift 2 ;;
    --adhoc) identity="-"; shift ;;
    -h|--help) sed -n '2,12p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

[[ "$(uname -s)" == "Darwin" ]] || { echo "this builds a macOS app bundle" >&2; exit 1; }

# The first quoted `version = "…"` in the root manifest is `[workspace.package]`'s; the
# package's own `version.workspace = true` does not match.
version="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"

# ---------------------------------------------------------------------------------------------
# The binary.
# ---------------------------------------------------------------------------------------------

echo "▸ building peek $version"
cargo build --release --manifest-path "$ROOT/Cargo.toml" --bin "$EXECUTABLE"

# ---------------------------------------------------------------------------------------------
# The bundle.
#
# A Dock launch cannot pass `--write`, so PEEK_PERSISTENCE turns saving on for the installed
# app; `cargo run` stays read-only unless asked. There is no PATH here: a Dock launch gets a
# bare one, but the one thing Peek spawns is the ACP agent, and peek-acp already resolves it
# against the login shell's PATH at runtime.
# No `peek://` scheme either: this app does not handle deep links, and claiming the scheme
# would take it from the Tauri app.
# ---------------------------------------------------------------------------------------------

echo "▸ assembling $BUNDLE"
rm -rf "$BUNDLE"
mkdir -p "$BUNDLE/Contents/MacOS" "$BUNDLE/Contents/Resources"

cp "$ROOT/target/release/$EXECUTABLE" "$BUNDLE/Contents/MacOS/$EXECUTABLE"
printf 'APPL????' > "$BUNDLE/Contents/PkgInfo"

cat > "$BUNDLE/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$APP_NAME</string>
  <key>CFBundleDisplayName</key><string>$APP_NAME</string>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleExecutable</key><string>$EXECUTABLE</string>
  <key>CFBundleIconFile</key><string>$APP_NAME</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>15.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSEnvironment</key>
  <dict>
    <key>PEEK_PERSISTENCE</key><string>write</string>
  </dict>
</dict>
</plist>
PLIST

# ---------------------------------------------------------------------------------------------
# The icon: the same image the Dock icon override uses. sips scales it and iconutil packs the
# sizes; both ship with macOS.
# ---------------------------------------------------------------------------------------------

echo "▸ rendering the icon"
iconset="$(mktemp -d)/$APP_NAME.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  sips -s format png -z "$size" "$size" "$ICON_SOURCE" \
    --out "$iconset/icon_${size}x${size}.png" > /dev/null
  sips -s format png -z "$((size * 2))" "$((size * 2))" "$ICON_SOURCE" \
    --out "$iconset/icon_${size}x${size}@2x.png" > /dev/null
done
iconutil --convert icns "$iconset" --output "$BUNDLE/Contents/Resources/$APP_NAME.icns"
rm -rf "$(dirname "$iconset")"

# ---------------------------------------------------------------------------------------------
# Signing.
#
# Take whichever identity is on this keychain: a Developer ID travels to other machines, an
# Apple Development certificate does not, and ad-hoc is the last resort.
#
# A real identity opts into the hardened runtime with a secure timestamp, because notarization
# refuses anything less. The hardened runtime needs no entitlements here: Peek neither JITs
# nor loads third-party libraries, and spawning the agent's own binaries is unaffected by it.
# ---------------------------------------------------------------------------------------------

find_identity() {
  security find-identity -v -p codesigning 2>/dev/null |
    awk -F\" -v want="$1" '$0 ~ want { print $2; exit }'
}

if [[ -z "$identity" ]]; then
  identity="${PEEK_SIGN_IDENTITY:-}"
fi
if [[ -z "$identity" ]]; then
  identity="$(find_identity "Developer ID Application:")"
fi
if [[ -z "$identity" ]]; then
  identity="$(find_identity "(Apple Development|Mac Developer):")"
fi
if [[ -z "$identity" ]]; then
  identity="-"
fi

if [[ "$identity" == "-" ]]; then
  sign_args=(--force --sign - --timestamp=none)
else
  sign_args=(--force --sign "$identity" --timestamp --options runtime)
fi

echo "▸ signing ($([[ "$identity" == "-" ]] && echo ad-hoc || echo "$identity"))"
codesign "${sign_args[@]}" "$BUNDLE" 2>&1 | sed 's/^/  /'

requirement="$(codesign --display --requirements - "$BUNDLE" 2>&1 | sed -n 's/^#* *designated => //p')"
echo "▸ identity: ${requirement:-none}"

if [[ "$identity" == "-" ]]; then
  echo "  ! signed ad-hoc: fine on this machine, Gatekeeper will refuse it anywhere else" >&2
fi

# ---------------------------------------------------------------------------------------------
# Installing.
# ---------------------------------------------------------------------------------------------

if (( install_app )); then
  target="/Applications"
  [[ -w "$target" ]] || target="$HOME/Applications"
  mkdir -p "$target"

  echo "▸ installing to $target"
  rm -rf "${target:?}/$APP_NAME.app"
  cp -R "$BUNDLE" "$target/$APP_NAME.app"

  # Launch Services caches Info.plist; without this the old registration can linger.
  /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister \
    -f "$target/$APP_NAME.app" 2>/dev/null || true

  echo
  echo "Installed $target/$APP_NAME.app"
  echo "Open it from Spotlight, or: open -a \"$APP_NAME\""
else
  echo
  echo "Built $BUNDLE"
fi
