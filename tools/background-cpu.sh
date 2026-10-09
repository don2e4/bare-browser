#!/usr/bin/env bash
# CPU time Bare's web processes use while a busy page (tools/fixtures/busy.html: a timer doing a little
# work every 50 ms) sits in a background tab, with a static page in front. Prints one line.
#   tools/background-cpu.sh        (BARE_BIN=... for another binary, WAIT=seconds, default 300;
#                                   FRONT=1 keeps the busy page in front, as a control)
# Needs: Xvfb, Openbox, xdotool.
set -u
cd "$(dirname "$0")/.."; BIN=${BARE_BIN:-target/release/bare}; WAIT=${WAIT:-300}; DNUM=:81; TMP=$(mktemp -d); pids=()
cleanup() { for p in "${pids[@]:-}"; do kill "$p" 2>/dev/null; done; rm -rf "$TMP"; }; trap cleanup EXIT
Xvfb $DNUM -screen 0 1280x800x24 +extension GLX +render -noreset >/dev/null 2>&1 & pids+=($!)
export DISPLAY=$DNUM; for i in $(seq 50); do xdotool getdisplaygeometry >/dev/null 2>&1 && break; sleep 0.1; done
openbox --sm-disable >/dev/null 2>&1 & pids+=($!); sleep 1
BARE_HOME="$TMP/home" BARE_APP_ID="app.bare.Bg$$" BARE_DISCARD_SECS=0 GSK_RENDERER=cairo BARE_FILTER_UPDATES=0 \
  "$BIN" "$PWD/tools/fixtures/busy.html" "$PWD/tools/fixtures/hello.html" >/dev/null 2>&1 & bare=$!; pids+=($bare)
w=$(timeout 20 xdotool search --sync --onlyvisible --pid "$bare" | head -1); sleep 3; xdotool windowfocus "$w"
[[ -n ${FRONT:-} ]] || xdotool key alt+2   # the static page in front, the busy one behind it
sleep 2
descendants() { local c; for c in $(pgrep -P "$1"); do echo "$c"; descendants "$c"; done; }
ticks() { # user + system clock ticks of the web processes (not their sandbox wrappers)
  local t=0 p; for p in $(descendants "$bare"); do
    [[ $(cat /proc/$p/comm 2>/dev/null) == WebKitWebProces ]] && t=$((t + $(awk '{print $14 + $15}' /proc/$p/stat)))
  done; echo $t
}
a=$(ticks); sleep "$WAIT"; b=$(ticks)
awk -v a="$a" -v b="$b" -v hz="$(getconf CLK_TCK)" -v w="$WAIT" -v f="${FRONT:+in front}" \
  'BEGIN {printf "web processes used %.2f CPU seconds in %d s (busy page %s)\n", (b - a) / hz, w, f ? f : "in a background tab"}'
