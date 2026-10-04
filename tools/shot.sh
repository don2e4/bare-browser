#!/usr/bin/env bash
# Verifies the frameless window: runs Bare under Openbox in a headless X server and checks that it has
# no title bar. A decorated control window (st) must report a title bar, proving the check can fail.
# Also exercises the window mechanics that replace a title bar (resize edge, gutter drag, F11, Ctrl+W)
# and window-size persistence. Screenshots land in target/shots/.
#
#   tools/shot.sh
#   BARE_BIN=...  binary to test (default target/debug/bare)
#   XVFB=...      Xvfb to use (default: Xvfb on PATH, else Xephyr, which opens a window on your desktop)
#
# Needs: Openbox, xdotool, xprop, ImageMagick `import`, and a terminal (`st`) for the control window.
set -u
cd "$(dirname "$0")/.."
BIN=${BARE_BIN:-target/debug/bare}
OUT=target/shots
mkdir -p "$OUT"
DNUM=:77
FIXTURE="$PWD/tools/fixtures/hello.html"
TMP=$(mktemp -d)
fails=0
pids=()
HOST_DISPLAY=${DISPLAY:-}

ok()    { echo "  ok    $*"; }
bad()   { echo "  FAIL  $*"; fails=$((fails + 1)); }
check() { local desc=$1; shift; if "$@"; then ok "$desc"; else bad "$desc"; return 1; fi; }
log_has_line() { grep -qF "$1" "$TMP/bare.log"; }
wait_until() { local secs=$1; shift; for ((i = 0; i < secs * 10; i++)); do "$@" && return 0; sleep 0.1; done; return 1; }
cleanup() { for p in "${pids[@]:-}"; do kill "$p" 2>/dev/null; done; rm -rf "$TMP"; }
trap cleanup EXIT

# --- headless X + window manager ------------------------------------------------------------
XVFB_BIN=${XVFB:-$(command -v Xvfb || true)}
if [[ -n $XVFB_BIN ]]; then
  "$XVFB_BIN" $DNUM -screen 0 1280x800x24 +extension GLX +render -noreset -xkbdir /usr/share/X11/xkb >"$TMP/xvfb.log" 2>&1 &
else
  Xephyr $DNUM -screen 1280x800 -ac -nolisten tcp >"$TMP/xephyr.log" 2>&1 &
fi
pids+=($!)
export DISPLAY=$DNUM
wait_until 10 xdotool getdisplaygeometry >/dev/null 2>&1 || { echo "no X server on $DNUM"; exit 2; }
# Without Xvfb the nested server is a window on your own desktop; get it out of the way (it keeps working).
if [[ -z $XVFB_BIN && -n $HOST_DISPLAY ]]; then
  DISPLAY=$HOST_DISPLAY xdotool search --sync --name "Xephyr on $DNUM" windowminimize 2>/dev/null || true
fi
openbox --sm-disable --config-file /etc/xdg/openbox/rc.xml >"$TMP/openbox.log" 2>&1 &
pids+=($!)
sleep 1

# --- helpers ---------------------------------------------------------------------------------
find_window() { timeout 20 xdotool search --sync --onlyvisible --pid "$1" 2>/dev/null | head -1; }
frame_extents() { # "left right top bottom", waits for the WM to set them
  local w=$1 e
  for ((i = 0; i < 30; i++)); do
    e=$(xprop -id "$w" _NET_FRAME_EXTENTS 2>/dev/null | sed -nE 's/.*= *([0-9]+), *([0-9]+), *([0-9]+), *([0-9]+).*/\1 \2 \3 \4/p')
    [[ -n $e ]] && { echo "$e"; return; }
    sleep 0.1
  done
}
geom() { eval "$(xdotool getwindowgeometry --shell "$1")"; echo "$X $Y $WIDTH $HEIGHT"; }
title_is() { [[ $(xdotool getwindowname "$1" 2>/dev/null) == "$2" ]]; }
show_title() { echo "        (window title is now: $(xdotool getwindowname "$1" 2>/dev/null))"; tail -5 "$TMP/bare.log" | sed 's/^/        bare: /'; }
fullscreen() { xprop -id "$1" _NET_WM_STATE 2>/dev/null | grep -q _NET_WM_STATE_FULLSCREEN; }
go() { xdotool key ctrl+l; sleep 0.3; xdotool type --delay 20 "$1"; xdotool key Return; }
pixel() { import -window root -crop 1x1+"$1"+"$2" -depth 8 txt:- 2>/dev/null | tail -1 | grep -oE '#[0-9A-F]{6}' | head -1; }
drag() { # drag() x y dx dy: press at (x,y), move by (dx,dy) in two steps, release
  xdotool mousemove --sync "$1" "$2"; sleep 0.2
  xdotool mousedown 1; sleep 0.3
  xdotool mousemove_relative --sync -- "$(( $3 / 2 ))" "$(( $4 / 2 ))"; sleep 0.2
  xdotool mousemove_relative --sync -- "$(( $3 - $3 / 2 ))" "$(( $4 - $4 / 2 ))"; sleep 0.3
  xdotool mouseup 1
}


# BARE_FILTER_UPDATES=0: Bare must not fetch its filter lists behind the tests' backs (run_updates tests that).
export BARE_HOME="$TMP/bare" BARE_APP_ID="app.bare.Test$$" GSK_RENDERER=${GSK_RENDERER:-cairo} BARE_FILTER_UPDATES=0
start_bare() { "$BIN" "$@" >"$TMP/bare.log" 2>&1 & BARE_PID=$!; pids+=($BARE_PID); }

# Shared by the feature and filter-update sections: the local test server (find/blocking/download pages,
# a stand-in SearXNG, a small filter list), and short names for the checks that follow a page load.
serve_fixtures() {
  PORT=$((20000 + RANDOM % 20000))
  python3 tools/fixtures/server.py "$PORT" >/dev/null 2>&1 & pids+=($!)
  BASE="http://127.0.0.1:$PORT"
  wait_until 10 curl -sf "$BASE/find.html" -o /dev/null || { echo "test server did not start"; exit 2; }
}
tt() { wait_until 10 title_is "$W" "$1"; }
log_has() { grep -q "$1" "$TMP/bare.log"; }
begin() { start_bare "$@"; W=$(find_window "$BARE_PID"); sleep 1.2; xdotool windowfocus "$W"; }

run_basic() {
# --- 1. no title bar ---------------------------------------------------------------------------
echo "== no title bar"
start_bare
W=$(find_window "$BARE_PID")
[[ -n $W ]] || { echo "Bare never mapped a window"; cat "$TMP/bare.log"; exit 2; }
sleep 1.5
read -r flags _ decor _ <<<"$(xprop -id "$W" _MOTIF_WM_HINTS 2>/dev/null | sed 's/.*= *//; s/,//g')"
check "_MOTIF_WM_HINTS asks for no decorations (flags=$flags decorations=$decor)" \
  bash -c '(( ${1:-0} & 2 )) && (( ${2:-1} == 0 ))' _ "${flags:-0}" "${decor:-1}"
read -r l r t b <<<"$(frame_extents "$W")"
check "Openbox frame extents are all zero (l=$l r=$r t=$t b=$b)" [ "${l:-x}${r:-x}${t:-x}${b:-x}" = "0000" ]

st -e sleep 30 &
pids+=($!)
C=$(find_window $!)
read -r cl cr ct cb <<<"$(frame_extents "$C")"
check "control: a normal window DOES get a title bar (top extent=$ct)" [ "${ct:-0}" -gt 0 ]
kill $! 2>/dev/null

# --- 2. content and navigation ------------------------------------------------------------------
echo "== page, URL bar, title"
check "home page loads (window title is 'Bare')" wait_until 10 title_is "$W" "Bare"
import -window root "$OUT/1-home.png" 2>/dev/null
xdotool windowfocus "$W"; sleep 0.3
xdotool key ctrl+l; sleep 0.3
xdotool type --delay 20 "$FIXTURE"; xdotool key Return
check "typing a path in the URL bar loads it (title 'Fixture page')" wait_until 10 title_is "$W" "Fixture page"
sleep 0.8
import -window root "$OUT/2-page.png" 2>/dev/null
# The empty new-tab page is native (no web view), so it is not in the history: go somewhere else first.
go "$PWD/tools/fixtures/second.html"
check "a second page loads (title 'Second page')" wait_until 10 title_is "$W" "Second page"
xdotool key alt+Left
check "Alt+Left goes back (title 'Fixture page')" wait_until 10 title_is "$W" "Fixture page" || show_title "$W"
xdotool key alt+Right
check "Alt+Right goes forward (title 'Second page')" wait_until 10 title_is "$W" "Second page"

# --- 3. window mechanics that replace a title bar --------------------------------------------------
echo "== resize, move, fullscreen"
read -r x y w h <<<"$(geom "$W")"
check "default size is 1100x760 (got ${w}x${h})" [ "$w" = 1100 ] && [ "$h" = 760 ]
drag $((x + w - 2)) $((y + h / 2)) 120 0
sleep 0.5
read -r x2 y2 w2 h2 <<<"$(geom "$W")"
check "dragging the right edge resizes (width $w -> $w2)" [ "$w2" -ge $((w + 80)) ]
read -r x y w h <<<"$(geom "$W")"
drag $((x + 10)) $((y + 15)) 70 50
sleep 0.5
read -r x3 y3 w3 h3 <<<"$(geom "$W")"
check "dragging the URL bar gutter moves the window ($x,$y -> $x3,$y3)" [ $((x3 - x)) -ge 50 ] && [ $((y3 - y)) -ge 35 ]
xdotool key F11; sleep 0.8
check "F11 enters fullscreen" fullscreen "$W"
import -window root "$OUT/3-fullscreen.png" 2>/dev/null
xdotool key F11; sleep 0.8
check "F11 again leaves fullscreen" bash -c '! xprop -id "$1" _NET_WM_STATE | grep -q FULLSCREEN' _ "$W"
import -window root "$OUT/4-windowed.png" 2>/dev/null

# Kept after the resize checks on purpose: in headless Xvfb the first button-1 press after an
# xdotool click on buttons 8/9 is swallowed (even when the app ignores those buttons), which would
# make the edge-drag check fail for reasons unrelated to Bare.
echo "== mouse buttons, error page"
go "$PWD/tools/fixtures/second.html"
check "a second page loads (title 'Second page')" wait_until 10 title_is "$W" "Second page"
read -r x y w h <<<"$(geom "$W")"
xdotool mousemove $((x + 500)) $((y + 400)) click 8
check "mouse back button goes back exactly one page (title 'Fixture page')" wait_until 10 title_is "$W" "Fixture page" || show_title "$W"
xdotool click 9
check "mouse forward button goes forward (title 'Second page')" wait_until 10 title_is "$W" "Second page" || show_title "$W"
go "localhost:38917"
check "a refused connection shows an error page (title 'Can't load this page')" wait_until 10 title_is "$W" "Can't load this page" || show_title "$W"
import -window root "$OUT/5-error.png" 2>/dev/null
go "?just some words"
check "a search opens bare://search (window title becomes the query)" wait_until 20 title_is "$W" "just some words" || show_title "$W"
sleep 3   # let the engines answer
import -window root "$OUT/8-search.png" 2>/dev/null
xdotool key j; sleep 0.5; xdotool key j; sleep 0.5
import -window root "$OUT/9-search-j-j.png" 2>/dev/null
check "the browser survives a real search" kill -0 "$BARE_PID"

# --- 4. close + size memory -----------------------------------------------------------------------------
echo "== close and remember size"
read -r _ _ wbefore hbefore <<<"$(geom "$W")"
xdotool key ctrl+w
check "Ctrl+W closes the window and exits" wait_until 5 bash -c '! kill -0 "$1" 2>/dev/null' _ "$BARE_PID"
check "window size saved ($(tr '\n' ' ' <"$BARE_HOME/config/window" 2>/dev/null))" grep -q "width=$wbefore" "$BARE_HOME/config/window"
start_bare
W=$(find_window "$BARE_PID")
sleep 1
read -r _ _ wafter hafter <<<"$(geom "$W")"
check "relaunch restores the size (${wbefore}x${hbefore} -> ${wafter}x${hafter})" [ "$wafter" = "$wbefore" ] && [ "$hafter" = "$hbefore" ]
kill "$BARE_PID" 2>/dev/null

echo "== chrome = \"hidden\""
mkdir -p "$BARE_HOME/config"; printf 'chrome = "hidden"\nborder = true\n' >"$BARE_HOME/config/config.toml"
start_bare "$FIXTURE"
W=$(find_window "$BARE_PID"); sleep 1.5
read -r x y w h <<<"$(geom "$W")"
hidden_px=$(pixel $((x + w / 2)) $((y + 10)))
import -window root "$OUT/6-hidden.png" 2>/dev/null
xdotool windowfocus "$W"; xdotool key ctrl+l; sleep 0.8
shown_px=$(pixel $((x + w / 2)) $((y + 10)))
import -window root "$OUT/7-hidden-summoned.png" 2>/dev/null
check "hidden mode: page fills the top row until Ctrl+L ($hidden_px -> bar $shown_px)" [ "$hidden_px" != "$shown_px" ]
xdotool key Escape; sleep 0.8
back_px=$(pixel $((x + w / 2)) $((y + 10)))
check "Esc hides the bar again ($back_px)" [ "$back_px" = "$hidden_px" ]
kill "$BARE_PID" 2>/dev/null

}

run_tabs() {
echo "== tabs, bookmarks, history, suggestions, idle discard"
export BARE_HOME="$TMP/bare-tabs" BARE_APP_ID="app.bare.Tabs$$" BARE_DISCARD_SECS=3 BARE_LOG=1
mkdir -p "$BARE_HOME/config"
SECOND="$PWD/tools/fixtures/second.html"
start_bare "$FIXTURE"
W=$(find_window "$BARE_PID"); sleep 1.5
xdotool windowfocus "$W"
tt() { wait_until 10 title_is "$W" "$1"; }
check "tab A loads the fixture" tt "Fixture page"
read -r x y w h <<<"$(geom "$W")"

xdotool key ctrl+t; sleep 0.5
check "Ctrl+T opens a new tab on the home page" tt "Bare"
go "$SECOND"
check "tab B loads the second page" tt "Second page"
xdotool key ctrl+Tab
check "Ctrl+Tab cycles to the next tab (wraps B -> A)" tt "Fixture page"
xdotool key alt+2
check "Alt+2 jumps to the second tab" tt "Second page"
xdotool key alt+1
check "Alt+1 jumps to the first tab" tt "Fixture page"
import -window root "$OUT/10-two-tabs.png" 2>/dev/null

# the tab switcher: the bar focused with nothing typed lists the other tabs (regression: it showed nothing)
before=$(grep -c "suggestion Tab(" "$TMP/bare.log")
xdotool key ctrl+l; sleep 0.6
import -window root "$OUT/13-switcher.png" 2>/dev/null
xdotool key Return; sleep 0.8
check "Ctrl+L then Enter with nothing selected just reloads (no tab switch)" bash -c '[ "$(grep -c "suggestion Tab(" "$1")" = "$2" ]' _ "$TMP/bare.log" "$before"
xdotool key ctrl+l; sleep 0.6; xdotool key Down; sleep 0.2; xdotool key Return
check "Ctrl+L, Down, Enter switches to the other tab" wait_until 5 bash -c '[ "$(grep -c "suggestion Tab(" "$1")" -gt "$2" ]' _ "$TMP/bare.log" "$before"
check "...and it is the second page" tt "Second page"
xdotool mousemove $((x + 1061)) $((y + 15)); sleep 0.3; xdotool click 1; sleep 0.6
xdotool key Down; sleep 0.2; xdotool key Return
check "clicking the tab count opens the switcher (and Down, Enter picks the other tab)" tt "Fixture page"

# a link with target=_blank opens a new tab (in front); ctrl+click opens it behind
xdotool mousemove $((x + 160)) $((y + 30 + 320)); sleep 0.3; xdotool click 1
check "a target=_blank link opens in a new tab" tt "Second page"
check "...and the new tab was logged" grep -q "open tab 3 file://.*second.html" "$TMP/bare.log"
xdotool key ctrl+w
check "Ctrl+W closes it and returns to the most recently used tab" tt "Fixture page"
xdotool mousemove $((x + 160)) $((y + 30 + 320)); sleep 0.3
xdotool keydown ctrl; xdotool click 1; xdotool keyup ctrl; sleep 1
check "ctrl+click opens in the background (title unchanged)" tt "Fixture page"
check "...but it was opened" grep -q "open tab 4 file://.*second.html" "$TMP/bare.log"
xdotool mousemove $((x + 600)) $((y + 600)); xdotool key alt+4 2>/dev/null; sleep 0.3
xdotool key alt+9
check "Alt+9 jumps to the last tab" tt "Second page"
xdotool key ctrl+w; sleep 0.5
xdotool key alt+1
check "back on tab A after closing the last one" tt "Fixture page"

# bookmarks
xdotool key ctrl+d; sleep 0.5
check "Ctrl+D bookmarks the page (bookmarks.txt)" grep -q "^file://$FIXTURE Fixture page$" "$BARE_HOME/config/bookmarks.txt"
xdotool key ctrl+d; sleep 0.5
check "Ctrl+D again removes it" bash -c '! grep -q "hello.html" "$1"' _ "$BARE_HOME/config/bookmarks.txt"
xdotool key ctrl+d; sleep 0.5

# suggestions. Tab A is the (bookmarked) fixture, tab B the second page. The log says which kind of
# suggestion was opened, so a check can't pass by landing on the right page the wrong way.
xdotool key alt+1; sleep 0.3
check "(setup) on tab A, the fixture" tt "Fixture page"
tabs_before=$(grep -c "suggestion Tab(" "$TMP/bare.log")
xdotool key ctrl+l; sleep 0.3; xdotool type --delay 30 "second"; sleep 0.6
import -window root "$OUT/11-dropdown.png" 2>/dev/null
xdotool key Down; sleep 0.2; xdotool key Return
check "a tab suggestion switches to that tab" tt "Second page"
check "...and it was a Tab suggestion (one more than before)" bash -c '[ "$(grep -c "suggestion Tab(" "$1")" -gt "$2" ]' _ "$TMP/bare.log" "$tabs_before"
xdotool key ctrl+w; sleep 0.8
check "(setup) closed it: one tab left, on the fixture" tt "Fixture page" || { show_title "$W"; grep -E "open tab|discard|restore|close|switch|suggestion|dropdown for" "$TMP/bare.log" | tail -30 | sed "s/^/        log: /"; }
go "$SECOND"
check "(setup) the lone tab shows the second page" tt "Second page"
xdotool key ctrl+l; sleep 0.3; xdotool type --delay 30 "hello"; sleep 0.6
xdotool key Down; sleep 0.2; xdotool key Return
check "a bookmark suggestion opens the bookmark" tt "Fixture page"
check "...and it was a Bookmark suggestion" grep -q "suggestion Bookmark" "$TMP/bare.log"
xdotool key ctrl+d; sleep 0.4        # un-bookmark it: now only history knows the page
go "$SECOND"
check "(setup) the second page again" tt "Second page"
xdotool key ctrl+l; sleep 0.3; xdotool type --delay 30 "hello"; sleep 0.6
xdotool key Down; sleep 0.2; xdotool key Return
check "a history suggestion opens the visited page" tt "Fixture page"
check "...and it was a History suggestion" grep -q "suggestion History" "$TMP/bare.log"
xdotool key ctrl+l; sleep 0.3; xdotool type --delay 30 "rust lifetimes"; sleep 0.5
xdotool key Return
check "Enter with nothing selected searches what was typed" tt "rust lifetimes"
check "...and it was not a suggestion" bash -c '[ "$(grep -c "suggestion Search" "$1")" = 0 ]' _ "$TMP/bare.log"
go "$FIXTURE"
check "(setup) back on the fixture" tt "Fixture page"

# reopen closed tab
xdotool key ctrl+t; sleep 0.4; go "$FIXTURE"
check "(setup) a throwaway tab on the fixture" tt "Fixture page"
xdotool key ctrl+w; sleep 0.8
xdotool key ctrl+shift+t
check "Ctrl+Shift+T reopens the closed tab" tt "Fixture page"
go "localhost:38917"
check "(setup) an error page, to prove it is not recorded" tt "Can't load this page"

# idle discard (BARE_DISCARD_SECS=3): the other tab is unloaded, then comes back with its page
sleep 5
check "an idle background tab is discarded" grep -q "discard tab" "$TMP/bare.log"
xdotool key alt+1
check "switching to it brings the page back (the fixture, not the error page in the other tab)" tt "Fixture page"
check "...by restoring the tab" grep -q "restore tab" "$TMP/bare.log"

# history
python3 - "$BARE_HOME/data/history.sqlite" "$FIXTURE" "$SECOND" <<'PY' && ok "history.sqlite holds the visited pages with titles" || bad "history.sqlite contents"
import sqlite3, sys
db, fx, sec = sys.argv[1:4]
rows = {u: (t, v) for u, t, v in sqlite3.connect(db).execute("select url,title,visits from history")}
assert rows["file://" + fx][0] == "Fixture page", rows
assert rows["file://" + sec][0] == "Second page", rows
assert all(not u.startswith("bare://home") and "38917" not in u for u in rows), rows   # no home page, no error page
PY
kill "$BARE_PID" 2>/dev/null

}

run_features() {
echo "== find, downloads, permissions, content blocking"
serve_fixtures
export BARE_HOME="$TMP/bare-feat" BARE_APP_ID="app.bare.Feat$$" BARE_DISCARD_SECS=0 BARE_LOG=1
rm -rf "$BARE_HOME"; mkdir -p "$BARE_HOME/config" "$BARE_HOME/data"

# ---- content blocking: the built-in baseline alone leaves the test page untouched ------------------------
begin "$BASE/blocktest.html"
check "(baseline) the page loads its scripts and shows its element" tt "ok=number blocked=number promo=block"
check "the built-in filter list was compiled" wait_until 10 log_has "filters: bare-"
kill "$BARE_PID" 2>/dev/null; sleep 1
# ...and a downloaded list blocks one script and hides one element.
cat >"$BARE_HOME/data/filters.json" <<'JSON'
[{"trigger":{"url-filter":"/blockme\\.js"},"action":{"type":"block"}},
 {"trigger":{"url-filter":".*"},"action":{"type":"css-display-none","selector":".promo"}}]
JSON
begin "$BASE/blocktest.html"
check "a filter list blocks a script and hides an element (in the engine)" tt "ok=number blocked=undefined promo=none"
check "...and was compiled by WebKit" wait_until 10 log_has "filters: bare-.* compiled"
kill "$BARE_PID" 2>/dev/null; sleep 1
begin "$BASE/blocktest.html"
check "the second start loads the compiled list from cache" wait_until 10 log_has "loaded from cache"
check "...and blocking still applies" tt "ok=number blocked=undefined promo=none"
kill "$BARE_PID" 2>/dev/null; sleep 1
rm -f "$BARE_HOME/data/filters.json"
printf 'filters = false\n' >"$BARE_HOME/config/config.toml"
begin "$BASE/blocktest.html"
check "filters = false blocks nothing" tt "ok=number blocked=number promo=block"
kill "$BARE_PID" 2>/dev/null; sleep 1
rm -f "$BARE_HOME/config/config.toml"

# ---- find in page ------------------------------------------------------------------------------------------
begin "$BASE/find.html"
check "(setup) the find page loads" tt "find page"
xdotool key ctrl+f; sleep 0.4; xdotool type --delay 40 "needle"
check "Ctrl+F then typing finds all three matches" wait_until 5 log_has "find: 3 matches"
import -window root "$OUT/14-find.png" 2>/dev/null
xdotool key Return; sleep 0.3; xdotool key shift+Return; sleep 0.3
xdotool type --delay 40 "z"
check "a word that isn't there reports no matches" wait_until 5 log_has "find: 0 matches"
xdotool key Escape; sleep 0.4
import -window root "$OUT/15-find-closed.png" 2>/dev/null
check "Esc closes find and returns focus to the page (F5 reloads it)" bash -c 'xdotool key F5; sleep 1; true'

# (WebKit already turns the server's "../../evil name.bin" into "evil_name.bin"; bare-core::downloads
# sanitises again as a second line of defence, unit-tested there.)
# ---- downloads -----------------------------------------------------------------------------------------------
go "$BASE/download.bin"
check "an attachment is saved to the downloads folder" wait_until 10 log_has "download finished: evil_name.bin" || { grep -E "download|toast|open tab" "$TMP/bare.log" | tail -8 | sed "s/^/        log: /"; ls -la "$BARE_HOME/downloads" 2>&1 | sed "s/^/        ls: /"; }
check "...with a safe name (the ../../ in the suggested name is gone)" [ -f "$BARE_HOME/downloads/evil_name.bin" ]
check "...and nothing escaped the folder" bash -c '[ ! -e "$1/evil_name.bin" ] && [ ! -e "$2/evil_name.bin" ]' _ "$BARE_HOME" "$TMP"
check "...with all its bytes" [ "$(stat -c %s "$BARE_HOME/downloads/evil_name.bin" 2>/dev/null)" = 10240 ]
check "the page you were on is still there" tt "find page"
go "$BASE/download.bin"
check "a second download never overwrites the first" wait_until 10 [ -f "$BARE_HOME/downloads/evil_name (1).bin" ]
go "$BASE/data.bin"
check "a type the browser can't show is downloaded too" wait_until 10 [ -f "$BARE_HOME/downloads/data.bin" ]
kill "$BARE_PID" 2>/dev/null; sleep 1

# ---- permissions: only the location is asked about, and Block is the default ------------------------------
begin "$BASE/geo.html"
check "a page asking for the location is asked about first" wait_until 10 log_has "permission: asking about location"
check "...and the page waits for the answer" tt "geo pending"
import -window root "$OUT/16-permission.png" 2>/dev/null
xdotool key Return
check "pressing Enter (focus starts on Block) refuses" wait_until 10 log_has "permission: blocked"
check "...and the page is told it was denied (error 1)" tt "geo=error1"
xdotool key F5; sleep 0.3
check "reloading asks again" wait_until 10 bash -c '[ "$(grep -c "asking about location" "$1")" -ge 2 ]' _ "$TMP/bare.log"
xdotool key shift+Tab; sleep 0.2; xdotool key Return
check "Shift+Tab then Enter allows this time" wait_until 10 log_has "permission: allowed"
check "...and the page is not told 'denied'" bash -c '[ "$(xdotool getwindowname "$1")" != "geo=error1" ]' _ "$W"
kill "$BARE_PID" 2>/dev/null; sleep 1

# ---- Bare Search results page, through a SearXNG-compatible instance (the test server) -----------------------
printf '[search]\ninstance = "%s"\n' "$BASE" >"$BARE_HOME/config/config.toml"
begin
go "?!sx rust lifetimes"
check "(setup) a search through a configured SearXNG instance opens the results page" tt "rust lifetimes"
check "...with the instance's results" wait_until 10 log_has "search: 2 results from 1 of 1 engines"
import -window root "$OUT/17-results.png" 2>/dev/null
before=$(grep -c "open tab" "$TMP/bare.log")
xdotool key shift+Return
check "Shift+Enter opens the focused result in a new tab" tt "find page"
check "...and it was opened as a tab" bash -c '[ "$(grep -c "open tab" "$1")" -gt "$2" ]' _ "$TMP/bare.log" "$before"
check "...as an external page, not one sharing the results page's process" bash -c 'grep "open tab" "$1" | tail -1 | grep -q "http://127.0.0.1"' _ "$TMP/bare.log"
xdotool key alt+1; sleep 0.5
check "the results page is still in the first tab" tt "rust lifetimes"
rm -f "$BARE_HOME/config/config.toml"
kill "$BARE_PID" 2>/dev/null; sleep 1

# ---- --update-filters: download, convert, compile (needs the network) --------------------------------------
rm -rf "$BARE_HOME/data" "$BARE_HOME/cache"; mkdir -p "$BARE_HOME/data"
if "$BIN" --update-filters >"$TMP/update.log" 2>&1; then
  check "bare --update-filters converts the real lists" grep -qE "converted to [0-9]{5,} rules" "$TMP/update.log"
  check "...and writes them where the browser looks" [ -s "$BARE_HOME/data/filters.json" ] && [ -s "$BARE_HOME/data/filters.id" ]
  begin "$BASE/blocktest.html"
  check "WebKit accepts and compiles the full EasyList + EasyPrivacy" wait_until 90 log_has "filters: bare-.* compiled"
  check "...and the page still works" tt "ok=number blocked=number promo=block"
  kill "$BARE_PID" 2>/dev/null; sleep 1
else
  ok "(skipped: could not download the filter lists: $(tail -1 "$TMP/update.log"))"
fi
}

run_updates() {
echo "== filter lists refresh themselves (the lists come from the test server)"
serve_fixtures
export BARE_HOME="$TMP/bare-upd" BARE_APP_ID="app.bare.Upd$$" BARE_DISCARD_SECS=0 BARE_LOG=1 \
  BARE_FILTER_UPDATES=1 BARE_FILTER_LISTS="$BASE/testlist.txt"
rm -rf "$BARE_HOME"; mkdir -p "$BARE_HOME/config" "$BARE_HOME/data"
data="$BARE_HOME/data"
compiled_twice() { [ "$(grep -c 'filters: bare-.* compiled' "$TMP/bare.log")" -ge 2 ]; }
nothing_fetched() { ! grep -q 'update started' "$TMP/bare.log" && [ ! -e "$data/filters.json" ]; }

# ---- first run: only the baseline (which knows nothing of the test page) is in force at first; a few
# seconds after start the lists are fetched and swapped in, and the open window uses them at once.
begin "$BASE/blocktest.html"
check "(first run) the page loads with only the baseline in force" tt "ok=number blocked=number promo=block"
check "Bare fetches the lists by itself, in the background" wait_until 40 log_has "filters: updated"
check "...stores them where the browser looks" [ -s "$data/filters.json" ] && [ -s "$data/filters.id" ]
check "...and WebKit compiles them (after the baseline)" wait_until 20 compiled_twice
go "$BASE/blocktest.html"
check "...and the open window blocks with them at once, without a restart" tt "ok=number blocked=undefined promo=none"
kill "$BARE_PID" 2>/dev/null; sleep 1

# ---- a fresh list is left alone
begin "$BASE/blocktest.html"
check "(next start) the stored list blocks from the first page" tt "ok=number blocked=undefined promo=none"
check "...and, being under a week old, is not fetched again" wait_until 12 log_has "filters: no update needed"
check "...(no fetch was started)" bash -c '! grep -q "update started" "$1"' _ "$TMP/bare.log"
kill "$BARE_PID" 2>/dev/null; sleep 1

# ---- a week-old list is refreshed, unless an attempt was made recently
touch -d '8 days ago' "$data/filters.json"; touch -d '1 hour ago' "$data/filters.checked"
begin "$BASE/blocktest.html"
check "(old list, but tried an hour ago) no retry yet" wait_until 12 log_has "filters: no update needed"
kill "$BARE_PID" 2>/dev/null; sleep 1
touch -d '8 days ago' "$data/filters.json"; touch -d '7 hours ago' "$data/filters.checked"
begin "$BASE/blocktest.html"
check "(old list, last tried 7 hours ago) the lists are fetched again" wait_until 40 log_has "filters: updated"
check "...and the stored list is new" [ -n "$(find "$data/filters.json" -mmin -2)" ]
kill "$BARE_PID" 2>/dev/null; sleep 1

# ---- the opt-outs (config.toml decides from here on, so the test override is dropped)
unset BARE_FILTER_UPDATES
rm -rf "$data"; mkdir -p "$data"
printf 'filter_updates = false\n' >"$BARE_HOME/config/config.toml"
begin "$BASE/blocktest.html"; sleep 8
check "filter_updates = false: nothing is fetched" nothing_fetched
kill "$BARE_PID" 2>/dev/null; sleep 1
printf 'filters = false\n' >"$BARE_HOME/config/config.toml"
begin "$BASE/blocktest.html"; sleep 8
check "filters = false: nothing is fetched either" nothing_fetched
kill "$BARE_PID" 2>/dev/null; sleep 1
rm -f "$BARE_HOME/config/config.toml"
begin "$BASE/blocktest.html"
check "(control) the same start with the default config does fetch" wait_until 40 log_has "filters: updated"
kill "$BARE_PID" 2>/dev/null; sleep 1

# ---- unreachable: reported, not fatal, and not retried at the next start
export BARE_FILTER_UPDATES=1 BARE_FILTER_LISTS="http://127.0.0.1:9/none.txt"
rm -rf "$data"; mkdir -p "$data"
begin "$BASE/blocktest.html"
check "an unreachable list is reported as a failed update" wait_until 30 log_has "filters: update failed"
check "...nothing was stored" [ ! -e "$data/filters.json" ]
check "...the attempt was stamped" [ -e "$data/filters.checked" ]
check "...and the browser carries on" kill -0 "$BARE_PID"
kill "$BARE_PID" 2>/dev/null; sleep 1
begin "$BASE/blocktest.html"
check "the next start does not retry at once" wait_until 12 log_has "filters: no update needed"
kill "$BARE_PID" 2>/dev/null; sleep 1
export BARE_FILTER_UPDATES=0; unset BARE_FILTER_LISTS
}

run_moving() {
echo "== moving the window (it has no title bar, so the URL bar and the empty new-tab page are the handle)"
export BARE_HOME="$TMP/bare-move" BARE_APP_ID="app.bare.Move$$" BARE_DISCARD_SECS=0 BARE_LOG=1
rm -rf "$BARE_HOME"; mkdir -p "$BARE_HOME"
start_bare
W=$(find_window "$BARE_PID"); sleep 1.5; xdotool windowfocus "$W"
moved_by() { # moved_by "label" x0 y0 dx dy: did dragging from window-relative (x0,y0) move the window by about (dx,dy)?
  local label=$1 x0=$2 y0=$3 dx=$4 dy=$5 bx by
  read -r bx by _ _ <<<"$(geom "$W")"
  drag $((bx + x0)) $((by + y0)) "$dx" "$dy"; sleep 0.6
  read -r ax ay _ _ <<<"$(geom "$W")"
  MOVED_X=$((ax - bx)); MOVED_Y=$((ay - by))
}
# Moved by at least three quarters of the drag, in the direction of the drag (works for negative offsets).
check_moved()  { moved_by "$@"; [ $(( MOVED_X * $4 )) -ge $(( $4 * $4 * 3 / 4 )) ] && [ $(( MOVED_Y * $5 )) -ge $(( $5 * $5 * 3 / 4 )) ]; }
check_still()  { moved_by "$@"; [ "${MOVED_X#-}" -le 3 ] && [ "${MOVED_Y#-}" -le 3 ]; }

check "dragging the middle of the URL bar moves the window" check_moved bar-middle 500 15 80 60
check "dragging the empty new-tab page moves the window" check_moved home-page 500 400 -60 40
check "dragging the left gutter still moves the window" check_moved gutter 8 20 40 40
check "dragging the right end of the bar moves the window" check_moved bar-right 1000 15 -50 30
maximized() { xprop -id "$W" _NET_WM_STATE 2>/dev/null | grep -q "_NET_WM_STATE_MAXIMIZED_VERT"; }
read -r bx by _ _ <<<"$(geom "$W")"
xdotool mousemove $((bx + 8)) $((by + 20)) click --repeat 2 --delay 90 1; sleep 0.8
check "double-clicking an end of the bar maximizes the window" maximized
read -r bx by _ _ <<<"$(geom "$W")"   # the maximized window is somewhere else now
xdotool mousemove $((bx + 8)) $((by + 20)) click --repeat 2 --delay 90 1; sleep 0.8
check "double-clicking again restores it" bash -c '! xprop -id "$1" _NET_WM_STATE | grep -q MAXIMIZED_VERT' _ "$W"
check "a short click-wobble on the bar does not move the window" check_still wobble 500 15 3 2

# A plain click selects the address: typing replaces it (the log shows the text the bar saw).
read -r bx by _ _ <<<"$(geom "$W")"
xdotool mousemove $((bx + 500)) $((by + 15)); sleep 0.2; xdotool click 1; sleep 0.4
xdotool type --delay 40 "zz"; sleep 0.6
check "a plain click on the bar focuses it for typing" log_has_line 'dropdown for "zz"'
# While editing, dragging in the bar selects text instead of moving the window.
check "dragging inside the bar while editing does not move the window" check_still editing 300 15 120 0
xdotool key Escape; sleep 0.4

# Web content is not a handle.
go "$FIXTURE"
check "(setup) a page is showing" wait_until 10 title_is "$W" "Fixture page"
check "dragging a web page does not move the window" check_still page 500 400 80 50
check "dragging the bar moves the window while a page is showing (bar not focused)" check_moved bar-with-page 500 15 -40 30
kill "$BARE_PID" 2>/dev/null; sleep 1
}

run_launch() {
echo "== launching: several addresses, and handing pages to a running Bare"
export BARE_HOME="$TMP/bare-launch" BARE_APP_ID="app.bare.Launch$$" BARE_DISCARD_SECS=0 BARE_LOG=1
rm -rf "$BARE_HOME"; mkdir -p "$BARE_HOME"
SECOND="$PWD/tools/fixtures/second.html"
start_bare "$FIXTURE" "file://$SECOND"
W=$(find_window "$BARE_PID"); sleep 1.5; xdotool windowfocus "$W"
check "several addresses open one tab each; the first is in front" wait_until 10 title_is "$W" "Fixture page"
xdotool key alt+2
check "...and the second is behind it" wait_until 10 title_is "$W" "Second page"
check "...as two tabs, not one search" bash -c '[ "$(grep -c "open tab" "$1")" = 2 ] && ! grep -q "bare://search" "$1"' _ "$TMP/bare.log"
cp "$TMP/bare.log" "$TMP/bare-first.log"
# A second launch while it runs (what clicking a link in another program does): a new tab in this window.
"$BIN" "$PWD/tools/fixtures/hello.html" >/dev/null 2>&1; code=$?
check "launching Bare again hands over and exits at once" [ "$code" = 0 ]
check "...opening the page as a third tab" wait_until 10 bash -c 'grep -c "open tab" "$1" | grep -qx 3' _ "$TMP/bare.log"
check "...and bringing it to the front" wait_until 10 title_is "$W" "Fixture page"
kill "$BARE_PID" 2>/dev/null; sleep 1
}

if [[ ${ONLY:-all} == all ]]; then run_basic; fi
if [[ ${ONLY:-all} == all || ${ONLY:-} == tabs ]]; then run_tabs; fi
if [[ ${ONLY:-all} == all || ${ONLY:-} == features ]]; then run_features; fi
if [[ ${ONLY:-all} == all || ${ONLY:-} == updates ]]; then run_updates; fi
if [[ ${ONLY:-all} == all || ${ONLY:-} == moving ]]; then run_moving; fi
if [[ ${ONLY:-all} == all || ${ONLY:-} == launch ]]; then run_launch; fi

echo
if ((fails)); then echo "$fails check(s) FAILED (screenshots: $OUT/)"; exit 1; fi
echo "all checks passed (screenshots: $OUT/)"
