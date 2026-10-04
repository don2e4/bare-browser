# Benchmarks

Every number here was measured on **2026-10-04** on one machine, from the commit that added this file.
The raw output is in [`docs/bench/`](docs/bench). To reproduce, see [the end of this file](#reproduce).

These numbers are rough. They come from one desktop, a headless X server and software rendering, and
the runs vary. Read them as orders of magnitude, not as a ranking of browsers.

## Machine

| | |
|---|---|
| CPU | Intel Core i3-12100 (4 cores / 8 threads, up to 4.3 GHz) |
| RAM | 16 GB |
| OS | CachyOS, Linux 7.2.6 |
| WebKitGTK | 2.52.6 (system package, dynamically linked) |
| GTK | 4.22.5 |
| Firefox | 156.0.1 (for comparison) |
| Rust | rustc 1.98.1; `cargo build --release` with the workspace profile (`opt-level = 3`, fat LTO, 1 codegen unit, `panic = "abort"`), no extra `RUSTFLAGS` |
| Display | Xvfb 21.1.24 at 1280×800, Openbox 3.6.1, `GSK_RENDERER=cairo` (software rendering, no GPU) |
| Binary | `target/release/bare`, 3.9 MB stripped |

The desktop session kept running during the measurements. Memory and CPU are summed over the
measured browser's own process tree only, so other programs don't count toward them, but they do
compete for the CPU.

## Memory, processes, idle CPU

`tools/measure.sh` opens five identical local HTML pages (400 paragraphs each), one per tab. Memory is
**PSS** (proportional set size) summed over the browser's whole process tree. PSS splits shared pages
between the processes that share them, so the sum counts each page once. "MB" here is MiB (kB / 1024), as the script prints it. The script ran three
separate times ([`measure-1`](docs/bench/measure-1.txt), [`2`](docs/bench/measure-2.txt),
[`3`](docs/bench/measure-3.txt)); the tables show the median and the range.

| Bare | median | range (3 runs) | processes |
|---|---|---|---|
| one window on the home page (no web process yet) | 109 MB | 108–109 MB | 1 |
| 5 tabs, all loaded | 541 MB | 540–561 MB | 26 |
| the same 5 tabs after the 4 background tabs were discarded | 297 MB | 277–298 MB | 8 |
| idle CPU on the home page, no web process (100 % = one core, over 10 s) | 0.0 % | 0.0–0.1 % | |
| idle CPU with the 5 pages loaded ([`tools/idle-cpu.sh`](tools/idle-cpu.sh), [output](docs/bench/idle-cpu-5-tabs.txt)) | 0.0 % | 0.0–0.3 % | |
| start-up: `main()` to first frame presented, Bare's own clock | 249 ms | 247–256 ms | |

| Firefox 156, fresh profile, the same 5 pages | median | range (3 runs) | processes |
|---|---|---|---|
| 5 tabs, all loaded, measured 90 s after launch | 671 MB | 668–689 MB | 13 |
| idle CPU with the 5 pages loaded | 0.4 % | 0.3–0.7 % | |

What this does and doesn't show:

- **With all five tabs live, Bare used about 130 MB (~20 %) less than Firefox.** That is one
  workload of static pages. Real sites with heavy JavaScript will look different, and nothing here
  measured them.
- **Most of the saving comes from discarding, which Firefox doesn't do by default.** Bare unloads
  background tabs after `discard_minutes` (default 10). The measurement shortens that to 24 s with
  `BARE_DISCARD_SECS=24`. Firefox also unloads tabs, but only under memory pressure, so the
  "after discard" row has no Firefox equivalent.
- **Idle CPU is a tie.** Both are near zero with the same five pages loaded. (`measure.sh` measures
  Bare's idle CPU on the home page, before any web page exists; that row alone would flatter Bare,
  hence the separate five-page measurement.) An earlier version of this README reported 17.4 % for
  Firefox, measured 30 s after a fresh-profile launch; that was Firefox's first-run work, not idle
  cost. With 90 s to settle it is 0.3–0.7 %.
- **Bare runs twice as many processes** (26 vs 13). WebKitGTK wraps every web process in two
  `bwrap` (bubblewrap sandbox) processes, and the sandbox adds an `xdg-dbus-proxy`. The helpers are
  small (about 0.15 MB PSS each, 0.7 MB for the proxy), but they show up in `ps`.
- **Start-up times are not comparable, so there is no Firefox row.** Bare's figure is its own
  in-process clock, from `main()` to GTK presenting the first frame. The script times Firefox from
  outside, to the first visible window, polled by `xdotool`; that came out at ~1,008 ms in all three
  runs, which looks like the polling interval, not Firefox.
- Everything ran with software rendering (`GSK_RENDERER=cairo` in Xvfb). On a GPU, absolute numbers
  differ for both browsers.
- These runs used Bare's small built-in filter list, not the full EasyList + EasyPrivacy (see below).

### Where Bare's memory goes

After discard, with one live tab ([`measure-1`](docs/bench/measure-1.txt), `SHOWTREE=1`):

| Process | PSS |
|---|---|
| `bare` (the Rust/GTK shell: window, URL bar, search, history) | 113 MB |
| `WebKitWebProcess` (the one live tab) | 133 MB |
| `WebKitNetworkProcess` | 30 MB |
| 4 × `bwrap`, 1 × `xdg-dbus-proxy` | 1.2 MB |

The `bare` process itself is the surprising number: a window on the home page, with no web process
at all, costs about 109 MB. One sample
of it ([`bare-process-memory.txt`](docs/bench/bare-process-memory.txt), with one tab showing
Wikipedia, 110 MB PSS in total) splits as 63 MB anonymous memory (heap and other private
allocations), 39 MB of mapped files (shared libraries such as GTK, WebKitGTK and JavaScriptCore,
plus fonts and caches) and 8 MB of shared memory. Nobody has profiled the anonymous part yet; that is
the obvious place to look.

## Content blocking cost

**Not re-measured for this file.** On 2026-10-03, the run in the previous README compared the full
EasyList + EasyPrivacy against the built-in baseline. It used five alternating pairs with one tab
open, and the converter emitted about 112,000 rules. The full lists added about 8 MB (+6 to +10 MB
in each pair), idle CPU stayed 0.0 %, and the weekly refresh child process peaked near 130 MB for
under two seconds. `FILTERS=full tools/measure.sh` (see below) repeats it.

## Search pipeline

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
is: in the screenshot in the README, the search took 1.75 s, waiting on the engines.

## URL-bar history search

`cargo run --release -p bare-core --example bench_history`
([`history-bench.txt`](docs/bench/history-bench.txt)) fills an in-memory SQLite history with 200,000
synthetic pages and times one suggestion query, which runs on every keystroke, on the UI thread:

| Typed | Time per keystroke |
|---|---|
| (nothing) | 0.21 ms |
| `a` | 0.20 ms |
| `site12` | 5.2 ms |
| `topic 77 site` | 1.8 ms |
| `zzzz-no-match` (worst case: scans everything) | 19.5 ms |

A query that matches nothing scans every row, and at 200,000 rows that costs a little more than one
frame at 60 Hz (16.7 ms) per keystroke. Typical histories are much smaller.

## Search engine availability

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
usually still returns something. The README screenshot ("2 of 4 engines, duckduckgo blocked") was
taken minutes after this probe. If you need reliable results, set `[search] instance` to a SearXNG
server you control.

## Reproduce

```sh
cargo build --release
FIREFOX=1 SHOWTREE=1 FF_SETTLE=90 tools/measure.sh     # memory / CPU / start-up, Bare and Firefox
tools/idle-cpu.sh                                        # Bare idle CPU with the five pages loaded
# full filter lists: fetch them once into a scratch home, then measure with them in force
BARE_HOME=/tmp/bh target/release/bare --update-filters
FILTERS=full FILTERS_DIR=/tmp/bh/data tools/measure.sh
cargo run --release -p bare-search --example bench       # search pipeline, from the workspace root
cargo run --release -p bare-core --example bench_history # URL-bar history search
cargo bs probe                                           # which engines answer from your network
```

`tools/measure.sh` needs Xvfb (or Xephyr), Openbox, xdotool and python3. It uses
`BARE_DISCARD_SECS=24` so the discard step does not take ten minutes.
