# Using Bare

`bare [address | search words | file]...`: one argument is opened as typed in the URL bar; several
addresses or files open one tab each; any other words are one search. Run while Bare is already open, it
hands the pages to that window as new tabs. `man bare` has the same in man-page form.

## Moving and resizing

There is no title bar, so:

- **Move:** drag the URL bar (until you've started editing the address: then a drag selects text), the
  empty new-tab page, the empty space under the tabs in the sidebar, or either end of the bar. Hold
  **Alt** and drag anywhere also works (and is the only way with `chrome = "hidden"`, or in a page that
  fills the window).
- **Resize:** drag any edge or corner.
- **Maximize:** double-click either end of the URL bar. Fullscreen is F11.

## Keys

| Key | Does |
|---|---|
| Ctrl+L, Ctrl+K, F6 | focus the URL bar (all selected); with nothing typed it lists your other tabs |
| ↓ ↑ (or Ctrl+N/P) in the bar | move through suggestions; Enter opens one, Alt+Enter in a new tab |
| Enter / Esc (in the bar) | go / give up and return to the page |
| Ctrl+T / Ctrl+W / Ctrl+Shift+T | new top-level tab / close tab / reopen the last closed tab (under its old parent, if that is still open) |
| Ctrl+Tab, Ctrl+Shift+Tab | next / previous tab, down / up the tab tree (folded tabs are skipped) |
| Alt+1 … Alt+8, Alt+9 | jump to line 1…8 of the tab tree / the last line |
| F1 | show / hide the tab tree on the left (remembered) |
| Ctrl+D | bookmark this page / remove the bookmark |
| Ctrl+F, F3 / Shift+F3 (or Ctrl+G) | find in page / next / previous match; Esc closes |
| Alt+← / Alt+→, mouse back/forward | back / forward |
| Ctrl+R, F5 / Ctrl+Shift+R | reload / reload bypassing cache |
| Ctrl+= / Ctrl+- / Ctrl+0 | zoom in / out / reset |
| F11 | fullscreen (hides the bar) |
| Ctrl+Q | quit |

## Tabs as a tree

There is no tab strip. A sidebar on the left lists the open tabs the way the `tree` command lists files:

```
Rust Programming Language
├── The Book
│   └── Ownership
└── Install Rust
GTK 4 docs
└── Gtk.Widget
```

A tab opened from a link in another tab is a **subtab** of it: links that open a new window
(`target=_blank`, `window.open` after a click), middle-click and Ctrl+click on a link, and results
opened from Bare Search with Shift+Enter or Ctrl+Enter. Middle-click and Ctrl+click open the subtab
behind the current tab; the others bring it forward. A new subtab goes after its older siblings.
Everything else starts a new top-level tab at the bottom: Ctrl+T, Alt+Enter in the URL bar, and pages
handed to Bare from the command line or another program.

In the sidebar: click a tab to switch to it, middle-click to close it, and double-click a tab that has
subtabs to fold them away. A folded tab shows how many tabs it hides (`+3`); click that to unfold, or
just switch to one of them (from the URL bar, say) and its branch opens. Closing a tab moves its subtabs
up a level, into its place, so nothing closes that you didn't close. Unloaded (discarded) tabs are
dimmed. Drag the sidebar's edge to make it wider or narrower.

F1 hides the sidebar for a page-only window (it is also hidden in fullscreen, and starts hidden with
`chrome = "hidden"`). While it is hidden, a small count at the right end of the URL bar shows that other
tabs exist; click it for the switcher. The URL bar is a tab switcher either way: with nothing typed it
lists your other tabs.

## The URL bar and Bare Search

What you type in the URL bar: an address (`example.com`, `localhost:3000`, `/path/to/file.html`)
opens it; anything else searches. Start with `?` to force a search (`?main.rs`).

Search prefixes (SearXNG's syntax): `!w rust` asks only Wikipedia, `!it tokio` the programming engines
(`!gh` GitHub, `!so` Stack Overflow, `!aw` Arch Wiki), `:de` sets the language. Default engines: Bing,
DuckDuckGo, Wikipedia, Marginalia. From a terminal: `cargo bs rust lifetimes`, `cargo bs engines`,
`cargo bs probe` (live health check of every engine).

The results page keys: `j`/`k` (or arrows) move, Enter opens, Shift+Enter opens in a new tab,
Ctrl+Enter in a background tab, Ctrl+L refines the query.

## Config

`~/.config/bare/config.toml`, every key optional:

```toml
chrome = "bar"        # "hidden": no bar until Ctrl+L summons it
border = false        # true: 1 px hairline around the window
home_hint = true      # the one line of key hints on the home page
discard_minutes = 10  # background tabs idle this long are unloaded (0 = never); they reload when you return
history = true        # remember visited pages for suggestions (false = keep none)
filters = true        # block ads and trackers (see below)
filter_updates = true # fetch the full filter lists by itself, and again when a week old (false = only `bare --update-filters`)

[search]
instance = "https://searx.example"   # optional: ask this SearXNG instance when the built-in engines come up short
```

Files live in `~/.config/bare` (`config.toml`, `bookmarks.txt`, `window`: size and sidebar), `~/.local/share/bare`
(`history.sqlite`, cookies, site data) and `~/.cache/bare`. **Bookmarks are a plain text file**, one
`url title` per line; edit it by hand (comments starting with `#` are kept). The SearXNG instance must
have its JSON format enabled, which many public instances turn off; Bare says so (`searxng blocked`)
if it can't use it. `BARE_HOME=<dir>` puts all three under one directory.

## Content blocking

Ads and trackers are blocked inside WebKit's own engine: filter lists are converted to WebKit's
content-blocker format, compiled once, cached, and shared by every tab, so blocking costs no scripts
and no extra process. Bare keeps the lists fresh itself: a few seconds after it starts, if it has no
lists yet or they are a week old, it runs `bare --update-filters` as a short-lived child process. That
downloads EasyList and EasyPrivacy, converts the subset of rules WebKit can express (unsupported rule
types are skipped and counted, never guessed at) and stores them, and the running browser then
compiles and swaps them in: pages you load from then on are blocked, with no restart. The browser
itself never holds the download or the converter's memory. Until the first fetch finishes (about ten
seconds on a first run) a small built-in list of well-known ad and tracking hosts is in force. The
first compile of the full lists takes a few seconds; after that they load from cache in milliseconds.
A failed attempt (no network) is not repeated for six hours. `filter_updates = false` stops Bare from
fetching anything: then only `bare --update-filters`, which you run yourself, updates the lists.
EasyList and EasyPrivacy are not bundled: they are downloaded from their publishers.

What the converter skips: regex filters, scriptlets, procedural cosmetic filters (`:has-text` and
friends), `$redirect`/`$csp`/`$removeparam`/`$badfilter`, and cosmetic exceptions. Ads that can only
be blocked with those rule types still show; this is not uBlock Origin.

## Downloads and permissions

**Downloads** (attachments, and anything the browser can't display) are saved straight into your
downloads folder, never over an existing file (`name (1).ext`), with a short message when they start and
finish. There is no download manager.

**Permissions:** a page asking for your location gets a bar under the URL bar (Allow this time / Block;
focus starts on Block, Esc blocks, and it is not remembered). Requests for notifications, camera,
microphone and the rest are refused without asking: Bare has nothing to hand them to. That means
video calls and other sites that need a camera or microphone do not work.

Third-party cookies are blocked, Intelligent Tracking Prevention is on, autoplay needs a click, and the
back/forward page cache is off (Back reloads from the HTTP cache) so closed pages don't linger in memory.

## Troubleshooting

- *Video won't play:* install the GStreamer plugins (`gst-plugins-good`; also `gst-plugins-bad`,
  `gst-libav` for more formats).
- *A blank or garbled window* (some GPU drivers): try `GSK_RENDERER=cairo bare`, or
  `WEBKIT_DISABLE_COMPOSITING_MODE=1 bare`.
- *Search says an engine is blocked or paused:* the public engines sometimes refuse automated traffic.
  Bare backs off from an engine that does and carries on with the others; you can also point it at a
  SearXNG instance (`[search] instance` in the config) as a fallback.
- *Ads still show:* on a first run the full lists arrive about ten seconds after start-up (or run
  `bare --update-filters` yourself); with `filter_updates = false` only that command fetches them.
  Some ad formats need rule types Bare does not convert (see above).
