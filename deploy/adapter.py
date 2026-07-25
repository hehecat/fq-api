#!/usr/bin/env python3
"""Thin adapter in front of unidbg-fq for Legado book sources.

Upstream: http://127.0.0.1:18080
Public:   https://fq-full.oyufen.com  (via cloudflared -> :18081)
"""
from __future__ import annotations

import json
import re
import traceback
import urllib.error
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

UPSTREAM = "http://127.0.0.1:18080"
PUBLIC = "https://fq-full.oyufen.com"
UA = "Mozilla/5.0 (Linux; Android 13) AppleWebKit/537.36 Chrome/120.0.0.0 Mobile Safari/537.36"


def upstream_get(path_qs: str, timeout: int = 90):
    url = UPSTREAM + path_qs
    req = urllib.request.Request(url, headers={"User-Agent": UA, "Accept": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return r.status, r.read(), dict(r.headers)


def j(data, status=200):
    body = json.dumps(data, ensure_ascii=False).encode("utf-8")
    return status, body


def escape_html(s: str) -> str:
    return (
        s.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace('"', "&quot;")
    )


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        print("[adapter]", self.address_string(), fmt % args, flush=True)

    def _send(self, status: int, body: bytes, content_type="application/json; charset=utf-8"):
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET,OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "*")
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        if status != 204:
            self.wfile.write(body)

    def do_OPTIONS(self):
        self._send(204, b"")

    def do_GET(self):
        try:
            parsed = urllib.parse.urlparse(self.path)
            path = parsed.path.rstrip("/") or "/"
            qs = urllib.parse.parse_qs(parsed.query)

            if path in ("/", "/health"):
                status, body = j(
                    {
                        "ok": True,
                        "service": "fq-full-adapter",
                        "upstream": UPSTREAM,
                        "public": PUBLIC,
                    }
                )
                return self._send(status, body)

            if path in ("/search", "/api/search"):
                key = (qs.get("key") or qs.get("q") or qs.get("query") or [""])[0]
                page = int((qs.get("page") or ["1"])[0] or 1)
                size = min(10, max(1, int((qs.get("size") or ["10"])[0] or 10)))
                if page < 1:
                    page = 1
                if not key:
                    return self._send(*j({"code": 400, "message": "missing key", "data": None}, 400))
                up = f"/search?key={urllib.parse.quote(key)}&page={page}&size={size}"
                _st, raw, _ = upstream_get(up)
                data = json.loads(raw)
                books = ((data.get("data") or {}).get("books") or [])
                out_books = []
                for b in books:
                    bid = str(b.get("bookId") or b.get("book_id") or "")
                    name = b.get("bookName") or b.get("book_name") or ""
                    desc = b.get("description") or b.get("abstract_text") or ""
                    cover = b.get("coverUrl") or b.get("thumb_url") or ""
                    out_books.append(
                        {
                            "book_id": bid,
                            "bookId": bid,
                            "book_name": name,
                            "bookName": name,
                            "author": b.get("author") or "",
                            "abstract_text": desc,
                            "description": desc,
                            "thumb_url": cover,
                            "coverUrl": cover,
                            "category": b.get("category") or "",
                            "word_count": b.get("wordNumber") or b.get("word_count"),
                            "wordNumber": b.get("wordNumber") or b.get("word_count"),
                            "last_chapter_title": b.get("lastChapterTitle")
                            or b.get("last_chapter_title"),
                            "lastChapterTitle": b.get("lastChapterTitle")
                            or b.get("last_chapter_title"),
                            "book_url": f"{PUBLIC}/book/{bid}",
                            "bookUrl": f"{PUBLIC}/book/{bid}",
                        }
                    )
                return self._send(
                    *j(
                        {
                            "code": 0,
                            "message": "success",
                            "data": {
                                "books": out_books,
                                "query": key,
                                "page": page,
                                "size": size,
                                "total": len(out_books),
                                "hasMore": (data.get("data") or {}).get("hasMore"),
                                "searchId": (data.get("data") or {}).get("searchId"),
                            },
                            "success": True,
                        }
                    )
                )

            m = re.fullmatch(r"/book/(\d+)", path) or re.fullmatch(r"/api/book/(\d+)", path)
            if m:
                bid = m.group(1)
                _st, raw, _ = upstream_get(f"/book/{bid}")
                data = json.loads(raw)
                b = data.get("data") or {}
                book_id = str(b.get("bookId") or bid)
                out = {
                    "book_id": book_id,
                    "bookId": book_id,
                    "book_name": b.get("bookName") or "",
                    "bookName": b.get("bookName") or "",
                    "author": b.get("author") or "",
                    "abstract_text": b.get("description") or "",
                    "description": b.get("description") or "",
                    "thumb_url": b.get("coverUrl") or "",
                    "coverUrl": b.get("coverUrl") or "",
                    "category": b.get("category") or "",
                    "word_number": b.get("wordNumber"),
                    "wordNumber": b.get("wordNumber"),
                    "chapter_total": b.get("totalChapters"),
                    "totalChapters": b.get("totalChapters"),
                    "last_chapter_title": b.get("lastChapterTitle"),
                    "lastChapterTitle": b.get("lastChapterTitle"),
                    "toc_url": f"{PUBLIC}/toc/{book_id}",
                    "tocUrl": f"{PUBLIC}/toc/{book_id}",
                    "status": b.get("status"),
                }
                return self._send(*j({"code": 0, "message": "success", "data": out, "success": True}))

            m = re.fullmatch(r"/toc/(\d+)", path) or re.fullmatch(
                r"/api/book/(\d+)/directory", path
            )
            if m:
                bid = m.group(1)
                _st, raw, _ = upstream_get(f"/toc/{bid}", timeout=120)
                data = json.loads(raw)
                items = ((data.get("data") or {}).get("item_data_list") or [])
                chapters = []
                for i, it in enumerate(items):
                    iid = str(
                        it.get("item_id")
                        or it.get("itemId")
                        or it.get("chapterId")
                        or ""
                    )
                    title = it.get("title") or f"第{i+1}章"
                    url = f"{PUBLIC}/chapter/{bid}/{iid}"
                    chapters.append(
                        {
                            "item_id": iid,
                            "itemId": iid,
                            "chapterId": iid,
                            "title": title,
                            "order": str(i + 1),
                            "url": url,
                            "chapter_url": url,
                        }
                    )
                return self._send(
                    *j(
                        {
                            "code": 0,
                            "message": "success",
                            "data": {
                                "book_id": bid,
                                "bookId": bid,
                                "chapters": chapters,
                                "item_data_list": chapters,
                                "total": len(chapters),
                                "serial_count": len(chapters),
                            },
                            "success": True,
                        }
                    )
                )

            m = re.fullmatch(r"/chapter/(\d+)/(\d+)", path) or re.fullmatch(
                r"/api/book/(\d+)/chapter/(\d+)", path
            )
            if m:
                bid, iid = m.group(1), m.group(2)
                _st, raw, _ = upstream_get(f"/chapter/{bid}/{iid}", timeout=120)
                data = json.loads(raw)
                c = data.get("data") or {}
                text = c.get("txtContent") or c.get("content") or c.get("content_text") or ""
                html = "".join(
                    f"<p>{escape_html(line)}</p>" for line in text.split("\n") if line.strip()
                )
                out = {
                    "book_id": str(c.get("bookId") or bid),
                    "bookId": str(c.get("bookId") or bid),
                    "item_id": str(c.get("chapterId") or iid),
                    "chapterId": str(c.get("chapterId") or iid),
                    "title": c.get("title") or "",
                    "author": c.get("authorName") or "",
                    "content": html if html else text,
                    "content_text": text,
                    "txtContent": text,
                    "word_number": c.get("wordCount") or len(text),
                    "wordCount": c.get("wordCount") or len(text),
                    "is_complete": True,
                    "source": "app_full+unidbg",
                }
                return self._send(*j({"code": 0, "message": "success", "data": out, "success": True}))

            return self._send(*j({"code": 404, "message": f"not found: {path}", "data": None}, 404))
        except urllib.error.HTTPError as e:
            try:
                body = e.read()
            except Exception:
                body = str(e).encode()
            self._send(e.code if e.code else 502, body)
        except Exception as e:
            print(traceback.format_exc(), flush=True)
            self._send(*j({"code": 502, "message": str(e), "data": None}, 502))


def main():
    host, port = "127.0.0.1", 18081
    httpd = ThreadingHTTPServer((host, port), Handler)
    print(f"fq-full-adapter on http://{host}:{port} -> {UPSTREAM}", flush=True)
    httpd.serve_forever()


if __name__ == "__main__":
    main()
