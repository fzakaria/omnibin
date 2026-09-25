#!/usr/bin/env python3
"""Serve the built site out of $SITE_ROOT, for local testing.

Not a bare `python -m http.server`: every file in a store output carries the
epoch as its mtime, so If-Modified-Since would 304 a file from a *previous*
build, and the browser then shows a stale site across rebuilds whatever
changed. Ignore conditional requests and forbid caching outright; this server
exists only for testing. (GitHub Pages serves real validators, so the deployed
site is unaffected.)

    SITE_ROOT=<dir> serve-site.py [port]
"""

import functools
import http.server
import os
import sys


class NoCacheHandler(http.server.SimpleHTTPRequestHandler):
    def send_head(self):
        del self.headers["If-Modified-Since"]
        del self.headers["If-None-Match"]

        # GitHub Pages resolves an extensionless request onto <path>.html,
        # which is how /docs/cli serves cli.html. Doing the same here is what
        # keeps the docs' own links working in a local preview: without it they
        # 404 here and succeed once deployed, which is the worst way round to
        # find out.
        path, sep, query = self.path.partition("?")
        local = self.translate_path(path)
        if not os.path.exists(local) and os.path.isfile(local + ".html"):
            self.path = path + ".html" + sep + query

        return super().send_head()

    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()


port = int(sys.argv[1]) if len(sys.argv) > 1 else 8000
handler = functools.partial(NoCacheHandler, directory=os.environ["SITE_ROOT"])

# Name the store path being served, so a glance at the terminal settles which
# build the browser should be showing.
print(f"serving {os.environ['SITE_ROOT']}", flush=True)
print(f"     on http://127.0.0.1:{port}", flush=True)
try:
    http.server.ThreadingHTTPServer(("127.0.0.1", port), handler).serve_forever()
except KeyboardInterrupt:
    sys.exit(0)
