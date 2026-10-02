"""本地看界面用的静态服务器：和 `python -m http.server` 一样，只是每个响应都带 no-store，改了文件刷新就生效。

用法：python scripts/serve-nocache.py <端口> <目录>
"""

import functools
import http.server
import sys


class NoCache(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()


port, root = int(sys.argv[1]), sys.argv[2]
handler = functools.partial(NoCache, directory=root)
http.server.ThreadingHTTPServer(("127.0.0.1", port), handler).serve_forever()
