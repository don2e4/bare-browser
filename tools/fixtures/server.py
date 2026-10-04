#!/usr/bin/env python3
"""A small local HTTP server for tools/shot.sh: pages for the find, blocking, download and
geolocation tests. Geolocation needs a secure context, which http://127.0.0.1 is.   server.py PORT"""
import http.server
import json
import sys

PAGES = {
    "/find.html": "<!doctype html><title>find page</title><body style='font:24px sans-serif'>"
    "<p>one needle here</p><p>a second needle there</p><p>and a third needle, plus haystack.</p>",
    # Two scripts (one the test list blocks) and an element the test list hides. The title reports
    # what happened after load.
    "/blocktest.html": """<!doctype html><title>pending</title>
<script src="/blockme.js"></script><script src="/okay.js"></script>
<style>.promo{display:block}</style><div class="promo" id="promo">promo</div>
<script>window.addEventListener('load',function(){
 document.title='ok='+typeof okLoaded+' blocked='+typeof blockmeLoaded+
   ' promo='+getComputedStyle(document.getElementById('promo')).display;});</script>""",
    "/geo.html": """<!doctype html><title>geo pending</title><script>
navigator.geolocation.getCurrentPosition(function(){document.title='geo=allowed'},
  function(e){document.title='geo=error'+e.code},{timeout:4000});</script>""",
}
SCRIPTS = {"/blockme.js": "var blockmeLoaded=1;", "/okay.js": "var okLoaded=1;"}
# A filter list for Bare's self-update (BARE_FILTER_LISTS): the same script and element as above.
FILTER_LIST = "! test list\n/blockme.js\n##.promo\n"


class Handler(http.server.BaseHTTPRequestHandler):
    def send(self, status, ctype, body, headers=None):
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        for k, v in (headers or {}).items():
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        path = self.path.split("?")[0]
        if path in PAGES:
            self.send(200, "text/html; charset=utf-8", PAGES[path].encode())
        elif path in SCRIPTS:
            self.send(200, "application/javascript", SCRIPTS[path].encode())
        elif path == "/testlist.txt":
            self.send(200, "text/plain; charset=utf-8", FILTER_LIST.encode())
        elif path == "/download.bin":  # an attachment whose suggested name tries to climb out of the folder
            self.send(200, "application/octet-stream", b"x" * 10240,
                      {"Content-Disposition": 'attachment; filename="../../evil name.bin"'})
        elif path == "/search":  # stands in for a SearXNG instance (`!sx query`), so results are deterministic
            host = self.headers.get("Host", "127.0.0.1")
            results = [
                {"url": f"http://{host}/find.html", "title": "Needle page", "content": "A page with needles."},
                {"url": f"http://{host}/blocktest.html", "title": "Second result", "content": "Another page."},
            ]
            body = json.dumps({"query": "q", "results": results, "suggestions": [], "corrections": []})
            self.send(200, "application/json", body.encode())
        elif path == "/data.bin":  # no Content-Disposition, but a type no browser can show
            self.send(200, "application/x-bare-test", b"y" * 2048)
        else:
            self.send(404, "text/plain", b"not found")

    def log_message(self, *args):
        pass


http.server.ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
