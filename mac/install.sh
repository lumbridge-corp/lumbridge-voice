#!/bin/bash
# Install Lumbridge Voice on this Mac.
#
#   mac/install.sh              build, check, install, load the agents
#   mac/install.sh --no-build   use the build already in mac/build/
#   mac/install.sh --force      skip the "is it busy right now?" check
#
# What it does, in order:
#   1. Stops if a dictation or read-aloud is in progress, or if a name it needs is taken by something else.
#   2. Builds (build.sh) and checks the build. Stops if the build is missing or wrongly signed.
#   3. Installs "/Applications/Lumbridge Voice.app", ~/.local/bin/lumbridge-voice and three agents, loads them.
#
# Safe to run again: it replaces the app and the command with the new build.
# LUMBRIDGE_VOICE_BUNDLE_ID must be the same as for build.sh (see there).
set -euo pipefail

FORCE=0; BUILD=1
for a in "$@"; do
  case "$a" in
    --force) FORCE=1 ;;
    --no-build) BUILD=0 ;;
    -h|--help) sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "install.sh: unknown option $a (see --help)" >&2; exit 2 ;;
  esac
done

stop() { echo "install.sh: STOPPED: $*" >&2; exit 1; }
say() { echo "install.sh: $*"; }

[ "$(uname -s)" = "Darwin" ] || stop "this installs a Mac app; run it on the Mac."
[ "$(id -u)" != "0" ] || stop "run it as yourself, not with sudo."
cd "$(dirname "$0")"
HERE="$PWD"
UID_N="$(id -u)"
DOMAIN="gui/$UID_N"

# ---- Names -------------------------------------------------------------------------------------
BUNDLE_ID="${LUMBRIDGE_VOICE_BUNDLE_ID:-com.lumbridgecorp.lumbridge-voice.app}"
APP_NAME="Lumbridge Voice"
EXE="LumbridgeVoice"
APP="/Applications/$APP_NAME.app"
BUILT="$HERE/build/$APP_NAME.app"
CLI="$HOME/.local/bin/lumbridge-voice"
AGENTS_DIR="$HOME/Library/LaunchAgents"
LOG_DIR="$HOME/Library/Logs/$APP_NAME"
LABELS=(com.lumbridgecorp.lumbridge-voice com.lumbridgecorp.lumbridge-voice-learn com.lumbridgecorp.lumbridge-voice-files)
ANNOUNCE_HEALTH="http://127.0.0.1:8796/health"

on_exit() {
  local rc=$?
  [ "$rc" = "0" ] || echo "install.sh: did not finish (exit $rc)." >&2
}
trap on_exit EXIT

loaded() { launchctl print "$DOMAIN/$1" >/dev/null 2>&1; }
pid_of() { pgrep -f "^$1( |\$)" 2>/dev/null | head -1 || true; }   # $1 = full path of the executable

unload() {   # unload one agent and wait until launchd has let go of it
  local label="$1" i
  loaded "$label" || return 0
  launchctl bootout "$DOMAIN/$label" 2>/dev/null || true
  for i in $(seq 1 50); do loaded "$label" || return 0; sleep 0.2; done
  stop "launchd did not unload $label within 10 seconds."
}

wait_gone() {   # $1 = full path of an executable
  local i
  for i in $(seq 1 50); do [ -z "$(pid_of "$1")" ] && return 0; sleep 0.2; done
  stop "$1 is still running after 10 seconds. Nothing more was changed."
}

# Why the app (old or new) should not be interrupted right now; empty when it is idle.
# There is no "are you recording" question the app answers, so this looks from outside:
#  - the microphone is in use on this Mac (a power assertion with an audio-in resource), or
#  - the system says this app's process is playing or recording audio, or
#  - the app wrote to its log in the last 20 seconds (it logs each phrase, transcript and announcement).
busy_reason() {
  local exe="$1" log="$2" pid assertions now mtime
  pid="$(pid_of "$exe")"
  [ -n "$pid" ] || return 0
  assertions="$(pmset -g assertions 2>/dev/null || true)"
  if grep -i 'audio-in' <<<"$assertions" >/dev/null; then
    echo "the microphone is in use (a dictation, or a call in another app)"; return 0
  fi
  if grep -i -B3 -A3 'audio' <<<"$assertions" | grep -E "Created for PID: $pid([^0-9]|\$)|pid $pid\\(" >/dev/null; then
    echo "it is playing audio (read-aloud or an announcement)"; return 0
  fi
  if [ -f "$log" ]; then
    now="$(date +%s)"; mtime="$(stat -f %m "$log")"
    if [ $((now - mtime)) -lt 20 ]; then echo "it wrote to its log $((now - mtime)) seconds ago"; return 0; fi
  fi
}

check_idle() {
  [ "$FORCE" = "1" ] && return 0
  local why
  why="$(busy_reason "$APP/Contents/MacOS/$EXE" "$LOG_DIR/app.log")"
  [ -z "$why" ] || stop "Lumbridge Voice is busy: $why. Finish, wait 20 seconds and run this again (or --force). Nothing was changed."
}

# ---- 1. Checks that change nothing ------------------------------------------------------------
for l in "${LABELS[@]}"; do
  f="$AGENTS_DIR/$l.plist"
  if [ -f "$f" ] && ! grep -q -e "Lumbridge Voice" -e "lumbridge-voice" "$f"; then
    stop "$f exists and is not a Lumbridge Voice agent. Not overwriting it."
  fi
done
if [ -d "$APP" ]; then
  have="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$APP/Contents/Info.plist" 2>/dev/null || true)"
  [ "$have" = "$BUNDLE_ID" ] || stop "$APP exists with bundle id '$have', not '$BUNDLE_ID'. Not overwriting it."
fi
check_idle

# ---- 2. Build and check the build -------------------------------------------------------------
if [ "$BUILD" = "1" ]; then
  say "building…"
  ./build.sh || stop "the build failed. Nothing was changed."
fi
[ -x "$BUILT/Contents/MacOS/$EXE" ] || stop "the build is missing ($BUILT). Run mac/build.sh first, or drop --no-build. Nothing was changed."
codesign --verify --strict "$BUILT" 2>/dev/null || stop "the build in $BUILT is not properly signed. Run mac/build.sh again. Nothing was changed."
built_id="$(codesign -dv "$BUILT" 2>&1 | sed -n 's/^Identifier=//p')"
[ "$built_id" = "$BUNDLE_ID" ] || stop "the build is signed as '$built_id', expected '$BUNDLE_ID' (permissions would be asked again). Nothing was changed."
for l in "${LABELS[@]}"; do [ -f "$HERE/$l.plist" ] || stop "missing $HERE/$l.plist. Nothing was changed."; done
[ -f "$HERE/lumbridge-voice" ] || stop "missing $HERE/lumbridge-voice. Nothing was changed."
check_idle      # again: the build took a while

# ---- 3. Install Lumbridge Voice ---------------------------------------------------------------
for l in "${LABELS[@]}"; do unload "$l"; done          # a run after the first: stop our own agents
wait_gone "$APP/Contents/MacOS/$EXE"
mkdir -p "$HOME/.local/bin" "$AGENTS_DIR" "$LOG_DIR"
rm -rf "$APP.installing"
ditto "$BUILT" "$APP.installing"
rm -rf "$APP"
mv "$APP.installing" "$APP"
install -m 755 "$HERE/lumbridge-voice" "$CLI"
for l in "${LABELS[@]}"; do
  sed "s|__HOME__|$HOME|g" "$HERE/$l.plist" > "$AGENTS_DIR/$l.plist"
  plutil -lint "$AGENTS_DIR/$l.plist" >/dev/null
  launchctl bootstrap "$DOMAIN" "$AGENTS_DIR/$l.plist"
done

# ---- 4. Report --------------------------------------------------------------------------------
up=0
for i in $(seq 1 30); do
  if curl -fsS -m 2 "$ANNOUNCE_HEALTH" >/dev/null 2>&1; then up=1; break; fi
  sleep 0.5
done
say "installed $APP ($(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP/Contents/Info.plist"))"
if [ "$up" = "1" ]; then
  say "Lumbridge Voice is running (its announce server answers on :8796)."
else
  say "WARNING: Lumbridge Voice did not answer on :8796 within 15 seconds. Look at: $LOG_DIR/app.log"
fi
say "log: $LOG_DIR/app.log   data: $HOME/Library/Application Support/$APP_NAME"
say "If macOS asks for Microphone, Accessibility, Input Monitoring or Screen Recording again, allow"
say "\"Lumbridge Voice\" in System Settings > Privacy & Security; for Accessibility, then run:"
say "  launchctl kickstart -k $DOMAIN/${LABELS[0]}"
