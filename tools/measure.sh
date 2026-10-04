#!/usr/bin/env bash
# What Bare costs. Runs a release build under Openbox in a headless X server and reports:
#   - binary size
#   - start-up: exec until the window is up
#   - memory: PSS summed over the whole process tree (Bare + WebKit's web/network processes), for
#       one tab on the home page, five tabs of identical local pages, and the same five tabs after
#       the four background ones were discarded
#   - idle CPU: share of one core used over 10 s, Bare on its home page (no web process; see
#       tools/idle-cpu.sh for Bare with five pages loaded), Firefox with its five tabs loaded
# FIREFOX=1 also measures Firefox (fresh profile, same five local pages) for comparison, FF_SETTLE seconds
# (default 30) after its window appears: a fresh profile does first-run work for a while.
#
#   tools/measure.sh                 (BARE_BIN=... to measure another binary; RUNS=3 to repeat)
#   FILTERS=full FILTERS_DIR=<dir> tools/measure.sh
#       with the full EasyList + EasyPrivacy in force instead of the built-in baseline; <dir> holds the
#       filters.json and filters.id that `BARE_HOME=<home> bare --update-filters` leaves in <home>/data.
#       The list is compiled by a warm-up launch first, so the numbers are the steady state.
#
# PSS (proportional set size) splits shared pages between the processes sharing them, so summing it
# over a tree counts each page once. Needs: Openbox, xdotool, Xvfb or Xephyr, python3.
set -u
cd "$(dirname "$0")/.."
BIN=${BARE_BIN:-target/release/bare}
RUNS=${RUNS:-1}
DNUM=:78
TMP=$(mktemp -d)
pids=()
cleanup() { for p in "${pids[@]:-}"; do kill "$p" 2>/dev/null; done; rm -rf "$TMP"; }
trap cleanup EXIT

now_ms() { echo $(( $(date +%s%N) / 1000000 )); }
wait_until() { local secs=$1; shift; for ((i = 0; i < secs * 20; i++)); do "$@" && return 0; sleep 0.05; done; return 1; }

# --- headless X + window manager -------------------------------------------------------------------
XVFB_BIN=${XVFB:-$(command -v Xvfb || true)}
if [[ -n $XVFB_BIN ]]; then
  "$XVFB_BIN" $DNUM -screen 0 1280x800x24 +extension GLX +render -noreset -xkbdir /usr/share/X11/xkb >/dev/null 2>&1 &
else
  Xephyr $DNUM -screen 1280x800 -ac -nolisten tcp >/dev/null 2>&1 &
fi
pids+=($!)
export DISPLAY=$DNUM
wait_until 10 xdotool getdisplaygeometry >/dev/null 2>&1 || { echo "no X server"; exit 2; }
openbox --sm-disable --config-file /etc/xdg/openbox/rc.xml >/dev/null 2>&1 &
pids+=($!)
sleep 1

# --- five identical local pages (a few hundred KB of DOM each, so a tab has something to hold) -----
python3 - "$TMP" <<'PY'
import sys
for i in range(5):
    body = "".join(f"<p class='p{n % 7}'>Paragraph {n} of page {i}. " + "Lorem ipsum dolor sit amet, consectetur adipiscing elit. " * 6 + "</p>\n" for n in range(400))
    open(f"{sys.argv[1]}/page{i}.html", "w").write(
        f"<!doctype html><meta charset=utf-8><title>Page {i}</title><style>"
        + "".join(f".p{k}{{color:#{k}{k}{k};margin:.4em 2em;font:15px/1.5 sans-serif}}" for k in range(7))
        + f"</style><h1>Page {i}</h1>\n{body}")
PY
PAGES=("$TMP"/page{0..4}.html)

# --- measuring helpers ---------------------------------------------------------------------------------
descendants() { local p=$1 c; for c in $(pgrep -P "$p" 2>/dev/null); do echo "$c"; descendants "$c"; done; }
tree_of() { echo "$1"; descendants "$1"; }
pss_kb() { local t=0 v p; for p in $(tree_of "$1"); do v=$(awk '/^Pss:/ {print $2}' "/proc/$p/smaps_rollup" 2>/dev/null); t=$((t + ${v:-0})); done; echo $t; }
procs() { tree_of "$1" | wc -l; }
cpu_ticks() { local t=0 p s; for p in $(tree_of "$1"); do s=$(awk '{print $14 + $15}' "/proc/$p/stat" 2>/dev/null); t=$((t + ${s:-0})); done; echo $t; }
find_window() { timeout 30 xdotool search --sync --onlyvisible --pid "$1" 2>/dev/null | head -1; }
mb() { awk -v k="$1" 'BEGIN {printf "%.0f", k / 1024}'; }
go() { xdotool key ctrl+l; sleep 0.3; xdotool type --delay 12 "$1"; xdotool key Return; }
idle_cpu() { # percent of one core over 10 s
  local a b; a=$(cpu_ticks "$1"); sleep 10; b=$(cpu_ticks "$1")
  awk -v a="$a" -v b="$b" -v hz="$(getconf CLK_TCK)" 'BEGIN {printf "%.1f", (b - a) / hz / 10 * 100}'
}

median() { sort -n | awk '{a[NR]=$1} END {print a[int((NR+1)/2)]}'; }

# --- Bare ---------------------------------------------------------------------------------------------------
bare_run() {
  export BARE_HOME="$TMP/bare-home-$RANDOM" BARE_APP_ID="app.bare.M$RANDOM" GSK_RENDERER=${GSK_RENDERER:-cairo} BARE_DISCARD_SECS=24 BARE_FILTER_UPDATES=0
  local t0 t1 pid w
  if [[ ${FILTERS:-baseline} == full ]]; then
    mkdir -p "$BARE_HOME/data"; cp "$FILTERS_DIR/filters.json" "$FILTERS_DIR/filters.id" "$BARE_HOME/data/" || exit 2
    BARE_LOG=1 "$BIN" >"$TMP/warm.log" 2>&1 & local warm=$!
    wait_until 60 grep -q 'filters: bare-.* compiled' "$TMP/warm.log" || { echo "the full list was not compiled"; exit 2; }
    sleep 1; kill "$warm" 2>/dev/null; wait "$warm" 2>/dev/null
  fi
  t0=$(now_ms)
  BARE_LOG=1 "$BIN" >"$TMP/bare.log" 2>&1 & pid=$!
  pids+=($pid)
  w=$(find_window "$pid"); title_is_bare() { [[ $(xdotool getwindowname "$w" 2>/dev/null) == "Bare" ]]; }
  wait_until 30 title_is_bare; t1=$(now_ms)
  sleep 3
  # The app's own clock (launch to window presented); timing from outside mostly measures this script.
  START_MS=$(grep -o 'window presented [0-9]*' "$TMP/bare.log" | grep -o '[0-9]*$'); START_MS=${START_MS:-$((t1 - t0))}
  HOME_KB=$(pss_kb "$pid"); HOME_PROCS=$(procs "$pid")
  sleep 9   # GTK stops blinking the caret 10 s after focus; measure idle after that
  IDLE=$(idle_cpu "$pid")
  xdotool windowfocus "$w"
  go "${PAGES[0]}"; sleep 1.5
  local i; for i in 1 2 3 4; do xdotool key ctrl+t; sleep 0.4; go "${PAGES[$i]}"; sleep 1.5; done
  sleep 2; FIVE_KB=$(pss_kb "$pid"); FIVE_PROCS=$(procs "$pid")
  sleep 40; DISC_KB=$(pss_kb "$pid"); DISC_PROCS=$(procs "$pid")   # the four background tabs are now discarded
  if [[ ${SHOWTREE:-0} == 1 ]]; then for p in $(tree_of "$pid"); do printf '   %7s kB PSS  %s\n' "$(awk '/^Pss:/ {print $2}' /proc/$p/smaps_rollup 2>/dev/null)" "$(cat /proc/$p/comm 2>/dev/null)"; done; fi
  grep -cE "discard tab" "$TMP/bare.log" | sed "s/^/discard events logged: /"; kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null
}

if [[ ! -x $BIN ]]; then echo "no binary at $BIN (cargo build --release)"; exit 2; fi
echo "binary: $BIN  ($(du -h "$BIN" | cut -f1))"
if [[ ${FILTERS:-baseline} == full ]]; then
  [[ -s ${FILTERS_DIR:-/nonexistent}/filters.json ]] || { echo "FILTERS=full needs FILTERS_DIR=<dir with filters.json and filters.id>"; exit 2; }
  echo "content blocking: full lists ($(du -h "$FILTERS_DIR/filters.json" | cut -f1) of rules)"
else
  echo "content blocking: built-in baseline"
fi
declare -a S H F D I
for ((r = 0; r < RUNS; r++)); do
  bare_run; S+=($START_MS); H+=($HOME_KB); F+=($FIVE_KB); D+=($DISC_KB); I+=($IDLE)
done
printf '\n%-44s %10s\n' "Bare" "(median of $RUNS)"
printf '%-44s %8s ms\n' "start-up (launch to window presented)" "$(printf '%s\n' "${S[@]}" | median)"
printf '%-44s %8s MB  (%s processes)\n' "memory, 1 tab on the home page" "$(mb "$(printf '%s\n' "${H[@]}" | median)")" "$HOME_PROCS"
printf '%-44s %8s MB  (%s processes)\n' "memory, 5 tabs of local pages" "$(mb "$(printf '%s\n' "${F[@]}" | median)")" "$FIVE_PROCS"
printf '%-44s %8s MB  (%s processes)\n' "memory, same 5 tabs after idle discard" "$(mb "$(printf '%s\n' "${D[@]}" | median)")" "$DISC_PROCS"
printf '%-44s %8s %%\n' "idle CPU on the home page (one core = 100%)" "$(printf '%s\n' "${I[@]}" | median)"

# --- Firefox (optional) -----------------------------------------------------------------------------------
if [[ ${FIREFOX:-0} == 1 ]]; then
  command -v firefox >/dev/null || { echo "no firefox"; exit 0; }
  mkdir -p "$TMP/ff"
  t0=$(now_ms)
  firefox --new-instance --no-remote --profile "$TMP/ff" "file://${PAGES[0]}" "file://${PAGES[1]}" "file://${PAGES[2]}" "file://${PAGES[3]}" "file://${PAGES[4]}" >"$TMP/ff.log" 2>&1 &
  fpid=$!; pids+=($fpid)
  fw=$(find_window "$fpid"); t1=$(now_ms); sleep "${FF_SETTLE:-30}"   # let Firefox finish its start-up work before measuring
  printf '\n%-44s\n' "Firefox (fresh profile, the same 5 pages as 5 tabs, measured after ${FF_SETTLE:-30} s)"
  printf '%-44s %8s ms\n' "start-up to first window" "$((t1 - t0))"
  printf '%-44s %8s MB  (%s processes)\n' "memory, 5 tabs" "$(mb "$(pss_kb "$fpid")")" "$(procs "$fpid")"
  printf '%-44s %8s %%\n' "idle CPU" "$(idle_cpu "$fpid")"
  kill "$fpid" 2>/dev/null
fi
