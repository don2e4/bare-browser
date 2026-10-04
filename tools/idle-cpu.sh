#!/usr/bin/env bash
# Bare idle CPU with the same five local pages loaded as five tabs, which is how tools/measure.sh
# measures Firefox (measure.sh itself measures Bare idle on the home page). Prints one line; run it a few times.
#   tools/idle-cpu.sh        (BARE_BIN=... for another binary)  Needs: Xvfb, Openbox, xdotool, python3.
set -u
cd "$(dirname "$0")/.."; BIN=${BARE_BIN:-target/release/bare}; DNUM=:80; TMP=$(mktemp -d); pids=()
cleanup() { for p in "${pids[@]:-}"; do kill "$p" 2>/dev/null; done; rm -rf "$TMP"; }; trap cleanup EXIT
Xvfb $DNUM -screen 0 1280x800x24 +extension GLX +render -noreset >/dev/null 2>&1 & pids+=($!)
export DISPLAY=$DNUM; for i in $(seq 50); do xdotool getdisplaygeometry >/dev/null 2>&1 && break; sleep 0.1; done
openbox --sm-disable >/dev/null 2>&1 & pids+=($!); sleep 1
python3 - "$TMP" <<'PY'
import sys
for i in range(5):
    body = "".join(f"<p class='p{n % 7}'>Paragraph {n} of page {i}. " + "Lorem ipsum dolor sit amet, consectetur adipiscing elit. " * 6 + "</p>\n" for n in range(400))
    open(f"{sys.argv[1]}/page{i}.html", "w").write(
        f"<!doctype html><meta charset=utf-8><title>Page {i}</title><style>"
        + "".join(f".p{k}{{color:#{k}{k}{k};margin:.4em 2em;font:15px/1.5 sans-serif}}" for k in range(7))
        + f"</style><h1>Page {i}</h1>\n{body}")
PY
descendants() { local c; for c in $(pgrep -P "$1"); do echo "$c"; descendants "$c"; done; }
ticks() { local t=0 p s; for p in $1 $(descendants "$1"); do s=$(awk '{print $14 + $15}' /proc/$p/stat 2>/dev/null); t=$((t + ${s:-0})); done; echo $t; }
export BARE_HOME="$TMP/h" BARE_APP_ID="app.bare.I$RANDOM" GSK_RENDERER=cairo BARE_FILTER_UPDATES=0
"$BIN" "$TMP"/page{0..4}.html >/dev/null 2>&1 & pid=$!; pids+=($pid)
sleep 30   # load all five, and let GTK's caret stop blinking
n=$(( $(descendants $pid | wc -l) + 1 ))
a=$(ticks $pid); sleep 10; b=$(ticks $pid)
awk -v a=$a -v b=$b -v hz=$(getconf CLK_TCK) -v n=$n 'BEGIN {printf "idle CPU with 5 pages loaded: %.1f %% (%d processes)\n", (b-a)/hz/10*100, n}'
