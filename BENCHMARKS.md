# Benchmarks

Bare 0.2.0's numbers were measured on **2026-10-09** on one machine. Bare 0.1.0 and Firefox were
measured again that same day, so every comparison here is like for like. That matters: the same 0.1.0
binary measured 7 to 30 MB more on 2026-10-09 than on 2026-10-04, because the desktop session around the
tests was different. Sections that were not re-run say so, with their date. The raw output is in
[`docs/bench/`](docs/bench). To reproduce, see [the end of this file](#reproduce).

These numbers are rough. They come from one desktop, a headless X server and software rendering, and
the runs vary. Read them as orders of magnitude, not as a ranking of browsers.

## Machine

| | |
|---|---|
| CPU | Intel Core i3-12100 (4 cores / 8 threads, up to 4.3 GHz) |
| RAM | 16 GB |
| OS | CachyOS, Linux 7.2.7 |
| WebKitGTK | 2.52.6 (system package, dynamically linked) |
| GTK | 4.22.5, icon theme Papirus-Light (it matters for start-up; see below) |
| Firefox | 156.0.1 (for comparison) |
| Rust | rustc 1.98.1; `cargo build --release` with the workspace profile (`opt-level = 3`, fat LTO, 1 codegen unit, `panic = "abort"`), no extra `RUSTFLAGS` |
| Display | Xvfb 21.1.24 at 1280×800, Openbox 3.6.1, `GSK_RENDERER=cairo` (software rendering, no GPU) |
| Binary | `target/release/bare`, 4.0 MB stripped (0.1.0: 3.9 MB) |

The desktop session kept running during the measurements. Memory and CPU are summed over the
measured browser's own process tree only, so other programs don't count toward them, but they do
compete for the CPU.

## Memory, processes, idle CPU

`tools/measure.sh` opens five identical local HTML pages (400 paragraphs each), one per tab. Memory is
**PSS** (proportional set size) summed over the browser's whole process tree. PSS splits shared pages
between the processes that share them, so the sum counts each page once. "MB" here is MiB (kB / 1024),
as the script prints it. For 0.2.0 the script ran three separate times with Firefox
([`measure-0.2.0-1`](docs/bench/measure-0.2.0-1.txt), [`2`](docs/bench/measure-0.2.0-2.txt),
[`3`](docs/bench/measure-0.2.0-3.txt)); 0.1.0 ran three times in one go
([`measure-0.1.0-same-day`](docs/bench/measure-0.1.0-same-day.txt)). The tables show the median and the
range.

| Bare | 0.2.0 median | range (3 runs) | 0.1.0, same day | processes |
|---|---|---|---|---|
| one window on the home page (no web process yet) | 109 MB | 109–109 MB | 116 MB (116–118) | 1 |
| 5 tabs, all loaded | 535 MB | 531–548 MB | 571 MB (570–572) | 26 |
| the same 5 tabs after the 4 background tabs were discarded | 295 MB | 295–298 MB | 309 MB (307–309) | 8 |
| idle CPU on the home page, no web process (100 % = one core, over 10 s) | 0.0 % | 0.0–0.0 % | 0.0 % | |
| idle CPU with the 5 pages loaded ([`tools/idle-cpu.sh`](tools/idle-cpu.sh), [output](docs/bench/idle-cpu-5-tabs-0.2.0.txt)) | 0.0 % | 0.0–0.1 % | | 32¹ |
| start-up: `main()` to the window presented, Bare's own clock | 75 ms | 71–76 ms | 254 ms (249–256) | |

| Firefox 156, fresh profile, the same 5 pages | median | range (3 runs) | processes |
|---|---|---|---|
| 5 tabs, all loaded, measured 90 s after launch | 772 MB | 737–776 MB | 13 |
| idle CPU with the 5 pages loaded | 1.4 % | 1.0–6.6 % | |

What this does and doesn't show:

- **With all five tabs live, Bare 0.2.0 used about 240 MB (~30 %) less than Firefox.** That is one
  workload of static pages. Real sites with heavy JavaScript will look different, and nothing here
  measured them. Firefox's figure also moves from day to day: it was 671 MB on 2026-10-04
  ([`measure-1`](docs/bench/measure-1.txt), [`2`](docs/bench/measure-2.txt),
  [`3`](docs/bench/measure-3.txt)).
- **0.2.0 uses 7 to 36 MB less than 0.1.0.** It doesn't set up WebKit's context (which starts
  JavaScriptCore) or the search engines until a page or a search needs them, and it hands memory
  that a closed or discarded tab freed back to the system (`malloc_trim`). The five-tab figure varies
  by more than that between runs, so read it as "a little less", not as a precise saving. The tab
  sidebar, new in 0.2.0, is included in these numbers.
- **Discarding saves more than anything else, and Firefox doesn't do it by default.** Bare unloads
  background tabs after `discard_minutes` (default 10). The measurement shortens that to 24 s with
  `BARE_DISCARD_SECS=24`. Firefox also unloads tabs, but only under memory pressure, so the
  "after discard" row has no Firefox equivalent.
- **Idle CPU is near zero for both.** Firefox's three runs ranged from 1.0 to 6.6 % this time; Bare's
  stayed at 0.0–0.1 %. (`measure.sh` measures Bare's idle CPU on the home page, before any web page
  exists; that row alone would flatter Bare, hence the separate five-page measurement.)

  ¹ `idle-cpu.sh` passes the five pages as command-line arguments instead of opening tabs one by
  one with Ctrl+T as `measure.sh` does. That run has 32 processes, not 26, with both versions;
  nobody has looked into why yet.
- **Bare runs twice as many processes** (26 vs 13). WebKitGTK wraps every web process in two
  `bwrap` (bubblewrap sandbox) processes, and the sandbox adds an `xdg-dbus-proxy`. The helpers are
  small (about 0.15 MB PSS each, 0.7 MB for the proxy), but they show up in `ps`.
- **Start-up times are not comparable, so there is no Firefox row.** Bare's figure is its own
  in-process clock, from `main()` to GTK presenting the window. The script times Firefox from
  outside, to the first visible window, polled by `xdotool`; that came out at ~1,010 ms in all three
  runs, which looks like the polling interval, not Firefox.
- Everything ran with software rendering (`GSK_RENDERER=cairo` in Xvfb). On a GPU, absolute numbers
  differ for both browsers.
- These runs used Bare's small built-in filter list, not the full EasyList + EasyPrivacy (see below).

## Start-up

Most of 0.1.0's quarter of a second was not Bare's. When a GtkApplication starts, it asks the icon
theme whether it has an icon named after the application, and that question waits until the whole
theme has loaded. With Papirus-Light, which this machine uses, that took about 190 ms. Bare's own
set-up took about 10 ms. GTK skips the question when a default icon is already named, so 0.2.0 names
one before GTK starts, unnames it so that showing the window doesn't wait either, and gives the window
its icon a second later; by then the theme has loaded in the background
([`main.rs`](crates/bare/src/main.rs)).

How much that saves depends on the icon theme. Five launches each, in one session
([`startup-icon-theme.txt`](docs/bench/startup-icon-theme.txt)):

| Window presented after | Papirus-Light | Adwaita (GTK's own) |
|---|---|---|
| Bare 0.1.0 | 270–280 ms | 75–83 ms |
| Bare 0.2.0 | 73–77 ms | 67–72 ms |

With a large icon theme, 0.2.0 shows its window in about a quarter of the time. With Adwaita the gain
is a few milliseconds, from not setting up WebKit and the search engines for a window that only shows
the new-tab page. (The session in this table ran a little slower than the one in the memory table, so
its 0.1.0 Papirus figures are higher than 249–256 ms.)

### Where Bare's memory goes

After discard, with one live tab ([`measure-0.2.0-1`](docs/bench/measure-0.2.0-1.txt), `SHOWTREE=1`):

| Process | PSS |
|---|---|
| `bare` (the Rust/GTK shell: window, URL bar, tab sidebar, search, history) | 132 MB |
| `WebKitWebProcess` (the one live tab) | 137 MB |
| `WebKitNetworkProcess` | 31 MB |
| 4 × `bwrap`, 1 × `xdg-dbus-proxy` | 1.3 MB |

The `bare` process itself is the surprising number: a window on the home page, with no web process
at all, costs about 109 MB, and loading pages adds over 20 MB more to it. One sample from 0.1.0 on
2026-10-04 ([`bare-process-memory.txt`](docs/bench/bare-process-memory.txt), with one tab showing
Wikipedia, 110 MB PSS in total) split as 63 MB anonymous memory (heap and other private
allocations), 39 MB of mapped files (shared libraries such as GTK, WebKitGTK and JavaScriptCore,
plus fonts and caches) and 8 MB of shared memory. Nobody has profiled the anonymous part yet; it is
still the obvious place to look.

### Old web processes after cross-site navigation

Each navigation from one site to another in the same tab leaves web processes behind. On a page
opened from a local file, typing four addresses on four different hosts into the URL bar of that one
tab took Bare from 8 processes and 279 MB to 50 processes and 546 MB (8 web processes, each with
its two `bwrap`). Three of those web processes ended about five minutes later; the rest stayed for as
long as the test watched.

That is WebKitGTK's doing, not Bare's: a 40-line Python program with one WebKitGTK view, and no
Bare code, does exactly the same ([`cross-site-processes.txt`](docs/bench/cross-site-processes.txt),
which includes the program). Neither of the settings meant for this changed it: the "document
browser" cache model, which should leave WebKit no spare processes to keep, and turning off
`ProcessSwapOnCrossSiteNavigation`. So Bare 0.2.0 doesn't set either. Discarding or closing a tab
still ends the web process it is using. Browsing many sites in one long-lived tab is where this
shows.

## Background tabs

A page in a background tab still runs its timers. WebKit already makes them fire at most once a
second; 0.2.0 also turns on `HiddenPageDOMTimerThrottlingAutoIncreases`, WebKit's own rule (used by
Safari) for stretching that interval out the longer a page stays hidden. Measured with
[`tools/background-cpu.sh`](tools/background-cpu.sh), which puts a page whose timer does a little
work every 50 ms (`tools/fixtures/busy.html`) in a background tab
([`background-timers.txt`](docs/bench/background-timers.txt)):

| Web processes' CPU time | 120 s | 300 s |
|---|---|---|
| WebKit's defaults (0.1.0) | 0.11 s | 0.24 s |
| with the stretching (0.2.0) | 0.00 s | 0.00 s |
| control: the same page in front | 0.26 s in 30 s | |

That page is light, so the absolute numbers are small; a heavier page in the background saves
proportionally more. Pages in front are not affected.

## Content blocking cost

**Not re-measured since 2026-10-03.** An earlier, unpublished measurement compared the full
EasyList + EasyPrivacy against the built-in baseline. It used five alternating pairs with one tab
open, and the converter emitted about 112,000 rules. The full lists added about 8 MB (+6 to +10 MB
in each pair), idle CPU stayed 0.0 %, and the weekly refresh child process peaked near 130 MB for
under two seconds. `FILTERS=full tools/measure.sh` (see below) repeats it.

## Search pipeline

**Measured on 2026-10-04, with 0.1.0; the search code has not changed since.**
`cargo run --release -p bare-search --example bench`
([`search-bench.txt`](docs/bench/search-bench.txt)) times the pure parts of a search on the recorded
engine responses in `crates/bare-search/tests/fixtures`:

| Step | Time |
|---|---|
| parse Bing results page (15 KB HTML) | 258 µs |
| parse DuckDuckGo results page (32 KB HTML) | 379 µs |
| parse Wikipedia API response (3 KB JSON) | 49 µs |
| parse Marginalia API response (4 KB JSON) | 26 µs |
| merge and score 4 engines' results | 24 µs |
| render the results page (27 results, 22 KB of HTML) | 32 µs |

All of Bare's own work on a search takes under a millisecond. The network decides how fast a search
is. 0.2.0 sets the engines up on the first search, on the search's own thread, instead of at
start-up.

## URL-bar history search

`cargo run --release -p bare-core --example bench_history`
([`history-bench.txt`](docs/bench/history-bench.txt)) fills an in-memory SQLite history with 200,000
synthetic pages and times one suggestion query, which runs on every keystroke, on the UI thread. The
same program ran against both versions' history code, one after the other:

| Typed | 0.1.0 | 0.2.0 |
|---|---|---|
| (nothing) | 0.22 ms | 0.22 ms |
| `a` | 0.20 ms | 0.21 ms |
| `article` (in every page) | 0.23 ms | 1.0 ms |
| `site12` (about 2,200 pages) | 6.4 ms | 6.1 ms |
| `topic 77 site` | 2.1 ms | 2.7 ms |
| `page-123456` (one page) | **25.3 ms** | 1.2 ms |
| `zzzz-no-match` (no page) | **22.2 ms** | 0.17 ms |
| recording a visit, in a database file (every page load) | 0.042 ms | 0.039 ms |

0.1.0 looked through pages newest first and stopped at 500 matches, so a rare word read the whole
history: 22–25 ms per keystroke, more than a frame at 60 Hz (16.7 ms). 0.2.0 adds a trigram index
(SQLite's FTS5): a word of three or more letters finds its few pages directly. When a word is in
thousands of pages, scanning newest first is still quicker, so it does that, after asking the index
how many there are; that costs common words up to a millisecond. The slowest query here went from
25 ms to 6 ms. The index makes filling a history in bulk about three times slower (6 s → 18 s for
the benchmark's 200,000 pages), but recording one visit, which happens on every page load, got a
little quicker, because 0.2.0 also writes through SQLite's write-ahead log.

A history kept by 0.1.0 has no index yet. Building it took 7 s for 200,000 pages, so 0.2.0 does that
once, on another thread, two seconds after start-up, and keeps scanning until it is done. With such a
history in place, the window still came up after 101 ms, and URL-bar suggestions worked during the
build; visits recorded in those seconds may be lost (the database is busy). Histories of up to 1,000
pages are indexed as they are opened, in tens of milliseconds.

## Search engine availability

**Measured on 2026-10-04, with 0.1.0.**
Bare Search depends on public engines, and the engines decide whether to answer.
`cargo bs probe` sends every engine three canned queries. Run from the machine above at 21:42
(UTC+8) on 2026-10-04 ([`probe.txt`](docs/bench/probe.txt)):

| Engine | ok | empty | blocked | failed | Notes |
|---|---|---|---|---|---|
| bing | 2 | 0 | 0 | 1 | timed out once |
| duckduckgo | 1 | 0 | **2** | 0 | bot-challenge page |
| wikipedia | 3 | 0 | 0 | 0 | |
| marginalia | 0 | 0 | 0 | **3** | timed out |
| archwiki | 0 | 1 | 0 | 2 | timed out |
| stackoverflow | 3 | 0 | 0 | 0 | |
| github | 1 | 0 | 0 | 2 | timed out |

That is one snapshot from one IP address, but it is typical of the problem. Scraped engines push back
with captchas, and free APIs time out. Bare suspends engines that fail and shows the rest, so a search
usually still returns something. If you need reliable results, set `[search] instance` to a SearXNG
server you control.

## Reproduce

```sh
cargo build --release
FIREFOX=1 SHOWTREE=1 FF_SETTLE=90 tools/measure.sh     # memory / CPU / start-up, Bare and Firefox
RUNS=3 tools/measure.sh                                  # Bare only, three runs, each run's figures too
tools/idle-cpu.sh                                        # Bare idle CPU with the five pages loaded
tools/background-cpu.sh                                  # a busy page in a background tab (FRONT=1: in front)
# start-up with GTK's own icon theme instead of yours
mkdir -p /tmp/adw/gtk-4.0 && printf '[Settings]\ngtk-icon-theme-name=Adwaita\n' >/tmp/adw/gtk-4.0/settings.ini
XDG_CONFIG_HOME=/tmp/adw tools/measure.sh
# full filter lists: fetch them once into a scratch home, then measure with them in force
BARE_HOME=/tmp/bh target/release/bare --update-filters
FILTERS=full FILTERS_DIR=/tmp/bh/data tools/measure.sh
cargo run --release -p bare-search --example bench       # search pipeline, from the workspace root
cargo run --release -p bare-core --example bench_history # URL-bar history search
cargo bs probe                                           # which engines answer from your network
```

`tools/measure.sh` needs Xvfb (or Xephyr), Openbox, xdotool and python3. It uses
`BARE_DISCARD_SECS=24` so the discard step does not take ten minutes.
