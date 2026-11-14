#!/bin/bash
# Build "Lumbridge Voice.app" into mac/build/. It only builds: nothing is installed, loaded or
# launched, and nothing outside this folder is written. Installing is install.sh.
#
#   ./build.sh                          ad-hoc signed
#   LUMBRIDGE_VOICE_SIGN=dev ./build.sh sign with the Apple Development identity in the keychain
#                                       (LUMBRIDGE_VOICE_SIGN_IDENTITY names one; needs an unlocked keychain)
#   LUMBRIDGE_VOICE_BUNDLE_ID=...       another bundle id. macOS ties the microphone, accessibility and
#                                       screen recording grants to it: keep it the same from build to build.
set -euo pipefail
cd "$(dirname "$0")"

BUNDLE_ID="${LUMBRIDGE_VOICE_BUNDLE_ID:-com.lumbridgecorp.lumbridge-voice.app}"
APP_NAME="Lumbridge Voice"
EXE="LumbridgeVoice"
B="build/$APP_NAME.app"

rm -rf build
mkdir -p "$B/Contents/MacOS" "$B/Contents/Resources/fonts"
cp Info.plist "$B/Contents/"
/usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier $BUNDLE_ID" "$B/Contents/Info.plist"
cp fonts/*.ttf fonts/OFL.txt "$B/Contents/Resources/fonts/"
swiftc -O -o "$B/Contents/MacOS/$EXE" main.swift pill.swift \
  -framework AppKit -framework AVFoundation -framework Speech

# Ad-hoc with an explicit requirement on the identifier: macOS then treats every rebuild as the same
# app, so the grants survive.
adhoc() { codesign --force --sign - --identifier "$BUNDLE_ID" -r="designated => identifier \"$BUNDLE_ID\"" "$B"; }
if [ "${LUMBRIDGE_VOICE_SIGN:-adhoc}" = "dev" ]; then
  ID="${LUMBRIDGE_VOICE_SIGN_IDENTITY:-$(security find-identity -v -p codesigning | sed -n 's/.*"\(Apple Development: [^"]*\)".*/\1/p' | head -1)}"
  if [ -z "$ID" ] || ! codesign --force --sign "$ID" --identifier "$BUNDLE_ID" "$B"; then
    echo "build.sh: signing with the development identity failed (keychain locked?); signing ad-hoc instead" >&2
    adhoc
  fi
else
  adhoc
fi
codesign --verify --strict "$B"
echo "built: $PWD/$B"
