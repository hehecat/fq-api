#!/usr/bin/env python3
"""FQ Full web: search + batch export TXT/EPUB with progress.

Uses ordered batch windows against /chapter (unidbg-fq prefetches via batch_full
internally with chapter-size≈30). Serves a simple SPA frontend.
"""
from __future__ import annotations

import html as html_lib
import json
import os
import queue
import re
import threading
import time
import traceback
import uuid
import zipfile
import urllib.error
import urllib.parse
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

UPSTREAM = os.environ.get("FQ_UPSTREAM", "http://127.0.0.1:18080")
PUBLIC = os.environ.get("FQ_PUBLIC", "https://fq-full.oyufen.com")
EXPORT_DIR = Path(os.environ.get("FQ_EXPORT_DIR", "/opt/fq/exports"))
STATIC_DIR = Path(os.environ.get("FQ_STATIC_DIR", "/opt/fq/web/static"))
HOST = os.environ.get("FQ_HOST", "127.0.0.1")
PORT = int(os.environ.get("FQ_PORT", "18081"))
UA = "fq-web/1.0"
BATCH_WINDOW = int(os.environ.get("FQ_BATCH_WINDOW", "30"))  # match unidbg prefetch
WINDOW_WORKERS = int(os.environ.get("FQ_WINDOW_WORKERS", "4"))

EXPORT_DIR.mkdir(parents=True, exist_ok=True)
STATIC_DIR.mkdir(parents=True, exist_ok=True)

_jobs: dict[str, dict] = {}
_jobs_lock = threading.Lock()


def upstream_get(path_qs: str, timeout: int = 120):
    url = UPSTREAM.rstrip("/") + path_qs
    req = urllib.request.Request(url, headers={"User-Agent": UA, "Accept": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read().decode("utf-8"))


def j(data, status=200):
    return status, json.dumps(data, ensure_ascii=False).encode("utf-8")


def escape_html(s: str) -> str:
    return (
        s.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace('"', "&quot;")
    )


def strip_html(s: str) -> str:
    s = re.sub(r"</p\s*>", "\n", s, flags=re.I)
    s = re.sub(r"<br\s*/?\s*>", "\n", s, flags=re.I)
    s = re.sub(r"<[^>]+>", "", s)
    s = html_lib.unescape(s)
    return re.sub(r"\n{3,}", "\n\n", s).strip()


def safe_name(name: str) -> str:
    name = re.sub(r'[\\/:*?"<>|]+', "_", name).strip()
    return name or "book"


def fetch_one_chapter(book_id: str, item_id: str, title_hint: str = "", retries: int = 5):
    last = None
    for i in range(retries):
        try:
            d = upstream_get(f"/chapter/{book_id}/{item_id}", timeout=120)
            data = d.get("data") or {}
            text = data.get("txtContent") or data.get("content_text") or ""
            if not text and data.get("content"):
                text = strip_html(data["content"])
            title = data.get("title") or title_hint or item_id
            if not text:
                raise RuntimeError(f"empty {item_id} code={d.get('code')} msg={d.get('message')}")
            return {
                "item_id": item_id,
                "title": title,
                "text": text.strip(),
                "words": int(data.get("wordCount") or data.get("word_number") or len(text)),
            }
        except Exception as e:
            last = e
            time.sleep(1.0 * (i + 1))
    raise RuntimeError(f"chapter {item_id} failed: {last}")


def write_txt(path: Path, meta: dict, chapters: list):
    lines = [meta["book_name"], f"作者：{meta.get('author') or ''}", ""]
    if meta.get("abstract"):
        lines += [meta["abstract"], ""]
    lines += ["=" * 20, ""]
    for ch in chapters:
        lines += [ch["title"], "", ch["text"], "", ""]
    path.write_text("\n".join(lines), encoding="utf-8")


def write_epub(path: Path, meta: dict, chapters: list):
    book_id = meta["book_id"]
    title = meta["book_name"]
    author = meta.get("author") or "Unknown"
    abstract = meta.get("abstract") or ""

    def chap_fname(i: int) -> str:
        return f"chap_{i:04d}.xhtml"

    container = """<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>
"""
    manifest = [
        '<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>',
        '<item id="css" href="style.css" media-type="text/css"/>',
    ]
    spine, nav = [], []
    for i, ch in enumerate(chapters, 1):
        fid = f"chap{i}"
        manifest.append(
            f'<item id="{fid}" href="{chap_fname(i)}" media-type="application/xhtml+xml"/>'
        )
        spine.append(f'<itemref idref="{fid}"/>')
        nav.append(
            f"""    <navPoint id="nav{i}" playOrder="{i}">
      <navLabel><text>{escape_html(ch['title'])}</text></navLabel>
      <content src="{chap_fname(i)}"/>
    </navPoint>"""
        )

    opf = f"""<?xml version="1.0" encoding="UTF-8"?>
<package version="2.0" xmlns="http://www.idpf.org/2007/opf" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:title>{escape_html(title)}</dc:title>
    <dc:creator opf:role="aut">{escape_html(author)}</dc:creator>
    <dc:language>zh-CN</dc:language>
    <dc:identifier id="BookId">fq-{escape_html(book_id)}</dc:identifier>
    <dc:description>{escape_html(abstract[:2000])}</dc:description>
  </metadata>
  <manifest>
    {chr(10).join(manifest)}
  </manifest>
  <spine toc="ncx">
    {chr(10).join(spine)}
  </spine>
</package>
"""
    ncx = f"""<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE ncx PUBLIC "-//NISO//DTD ncx 2005-1//EN" "http://www.daisy.org/z3986/2005/ncx-2005-1.dtd">
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head>
    <meta name="dtb:uid" content="fq-{escape_html(book_id)}"/>
    <meta name="dtb:depth" content="1"/>
    <meta name="dtb:totalPageCount" content="0"/>
    <meta name="dtb:maxPageNumber" content="0"/>
  </head>
  <docTitle><text>{escape_html(title)}</text></docTitle>
  <navMap>
{chr(10).join(nav)}
  </navMap>
</ncx>
"""
    css = "body{font-family:serif;line-height:1.75;padding:1em;max-width:40em;margin:auto;}h1{font-size:1.25em;text-align:center;}p{margin:.65em 0;text-indent:2em;}"

    def chapter_xhtml(ch: dict) -> str:
        paras = [
            f"<p>{escape_html(line.strip())}</p>"
            for line in ch["text"].split("\n")
            if line.strip()
        ]
        return f"""<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.1//EN" "http://www.w3.org/TR/xhtml11/DTD/xhtml11.dtd">
<html xmlns="http://www.w3.org/1999/xhtml" xml:lang="zh-CN">
<head><title>{escape_html(ch['title'])}</title>
<link rel="stylesheet" type="text/css" href="style.css"/></head>
<body><h1>{escape_html(ch['title'])}</h1>
{chr(10).join(paras)}
</body></html>
"""

    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as z:
        z.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        z.writestr("META-INF/container.xml", container)
        z.writestr("OEBPS/content.opf", opf)
        z.writestr("OEBPS/toc.ncx", ncx)
        z.writestr("OEBPS/style.css", css)
        for i, ch in enumerate(chapters, 1):
            z.writestr(f"OEBPS/{chap_fname(i)}", chapter_xhtml(ch))


def set_job(job_id: str, **kwargs):
    with _jobs_lock:
        job = _jobs.setdefault(job_id, {})
        job.update(kwargs)
        job["updated_at"] = time.time()


def get_job(job_id: str):
    with _jobs_lock:
        return dict(_jobs.get(job_id) or {})


def run_export(job_id: str, book_id: str, formats: list[str]):
    try:
        set_job(job_id, status="running", stage="book", progress=0, message="获取书籍信息")
        book = upstream_get(f"/book/{book_id}")["data"]
        meta = {
            "book_id": str(book.get("bookId") or book_id),
            "book_name": book.get("bookName") or book.get("book_name") or book_id,
            "author": book.get("author") or "",
            "abstract": book.get("description") or book.get("abstract_text") or "",
        }
        set_job(job_id, meta=meta, message=f"{meta['book_name']} / {meta['author']}")

        set_job(job_id, stage="toc", message="获取目录")
        toc = upstream_get(f"/toc/{book_id}", timeout=180)["data"]
        items = toc.get("item_data_list") or toc.get("chapters") or []
        total = len(items)
        if not total:
            raise RuntimeError("empty toc")
        set_job(job_id, total=total, done=0, message=f"目录 {total} 章，开始批量拉取")

        # Ordered windows to leverage unidbg internal batch_full prefetch (~30)
        chapters = [None] * total
        done = 0
        t0 = time.time()
        fails = []

        for start in range(0, total, BATCH_WINDOW):
            window = list(enumerate(items[start : start + BATCH_WINDOW], start=start))
            # Within a window, fetch concurrently; first request triggers batch_full prefetch
            with ThreadPoolExecutor(max_workers=min(WINDOW_WORKERS, len(window))) as ex:
                futs = {}
                for idx, it in window:
                    iid = str(it.get("item_id") or it.get("itemId") or it.get("chapterId") or "")
                    title_hint = it.get("title") or ""
                    futs[ex.submit(fetch_one_chapter, book_id, iid, title_hint)] = idx
                for fut in as_completed(futs):
                    idx = futs[fut]
                    try:
                        chapters[idx] = fut.result()
                    except Exception as e:
                        fails.append(str(e))
                        chapters[idx] = {
                            "item_id": str(items[idx].get("item_id") or ""),
                            "title": items[idx].get("title") or f"第{idx+1}章",
                            "text": f"【拉取失败: {e}】",
                            "words": 0,
                        }
                    done += 1
                    if done % 10 == 0 or done == total:
                        elapsed = max(time.time() - t0, 0.001)
                        rate = done / elapsed
                        eta = (total - done) / rate if rate else 0
                        set_job(
                            job_id,
                            done=done,
                            progress=round(done * 100 / total, 1),
                            rate=round(rate, 2),
                            eta_sec=int(eta),
                            message=f"拉取章节 {done}/{total}",
                            fails=len(fails),
                        )

        if any(c is None for c in chapters):
            raise RuntimeError("some chapters missing")

        set_job(job_id, stage="write", message="写入文件", progress=99)
        safe = safe_name(meta["book_name"])
        files = {}
        if "txt" in formats:
            txt_path = EXPORT_DIR / f"{safe}-{book_id}.txt"
            write_txt(txt_path, meta, chapters)
            files["txt"] = {
                "name": txt_path.name,
                "path": str(txt_path),
                "size": txt_path.stat().st_size,
                "url": f"/download/{urllib.parse.quote(txt_path.name)}",
            }
        if "epub" in formats:
            epub_path = EXPORT_DIR / f"{safe}-{book_id}.epub"
            write_epub(epub_path, meta, chapters)
            files["epub"] = {
                "name": epub_path.name,
                "path": str(epub_path),
                "size": epub_path.stat().st_size,
                "url": f"/download/{urllib.parse.quote(epub_path.name)}",
            }

        set_job(
            job_id,
            status="done",
            stage="done",
            progress=100,
            done=total,
            message="完成",
            files=files,
            total_words=sum(c["words"] for c in chapters),
            elapsed_sec=round(time.time() - t0, 1),
            fails=len(fails),
        )
    except Exception as e:
        print(traceback.format_exc(), flush=True)
        set_job(job_id, status="error", stage="error", message=str(e))


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        print("[web]", self.address_string(), fmt % args, flush=True)

    def _send(self, status: int, body: bytes, content_type="application/json; charset=utf-8", extra_headers=None):
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET,POST,OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "*")
        self.send_header("Cache-Control", "no-store")
        if extra_headers:
            for k, v in extra_headers.items():
                self.send_header(k, v)
        self.end_headers()
        if status != 204 and body is not None:
            self.wfile.write(body)

    def do_OPTIONS(self):
        self._send(204, b"")

    def _read_json(self):
        n = int(self.headers.get("Content-Length") or 0)
        if n <= 0:
            return {}
        return json.loads(self.rfile.read(n).decode("utf-8"))

    def do_POST(self):
        try:
            parsed = urllib.parse.urlparse(self.path)
            path = parsed.path.rstrip("/") or "/"
            if path in ("/api/export", "/export"):
                body = self._read_json()
                book_id = str(body.get("book_id") or body.get("bookId") or "").strip()
                formats = body.get("formats") or ["txt", "epub"]
                if isinstance(formats, str):
                    formats = [formats]
                formats = [f.lower() for f in formats if f]
                if not book_id.isdigit():
                    return self._send(*j({"code": 400, "message": "invalid book_id"}, 400))
                if not formats:
                    formats = ["txt", "epub"]
                job_id = uuid.uuid4().hex[:12]
                set_job(
                    job_id,
                    id=job_id,
                    status="queued",
                    book_id=book_id,
                    formats=formats,
                    progress=0,
                    done=0,
                    total=0,
                    message="排队中",
                    created_at=time.time(),
                )
                threading.Thread(
                    target=run_export, args=(job_id, book_id, formats), daemon=True
                ).start()
                return self._send(*j({"code": 0, "data": {"job_id": job_id}}))
            return self._send(*j({"code": 404, "message": "not found"}, 404))
        except Exception as e:
            print(traceback.format_exc(), flush=True)
            self._send(*j({"code": 502, "message": str(e)}, 502))

    def do_GET(self):
        try:
            parsed = urllib.parse.urlparse(self.path)
            path = parsed.path
            qs = urllib.parse.parse_qs(parsed.query)

            # static / spa
            if path in ("/", "/index.html"):
                return self._serve_static("index.html", "text/html; charset=utf-8")
            if path.startswith("/static/"):
                rel = path[len("/static/") :]
                return self._serve_static(rel)

            path_r = path.rstrip("/") or "/"

            if path_r in ("/health", "/api/health"):
                return self._send(
                    *j(
                        {
                            "ok": True,
                            "service": "fq-full-web",
                            "upstream": UPSTREAM,
                            "public": PUBLIC,
                            "batch_window": BATCH_WINDOW,
                        }
                    )
                )

            if path_r in ("/search", "/api/search"):
                key = (qs.get("key") or qs.get("q") or qs.get("query") or [""])[0]
                page = int((qs.get("page") or ["1"])[0] or 1)
                size = min(10, max(1, int((qs.get("size") or ["10"])[0] or 10)))
                if page < 1:
                    page = 1
                if not key:
                    return self._send(*j({"code": 400, "message": "missing key"}, 400))
                data = upstream_get(
                    f"/search?key={urllib.parse.quote(key)}&page={page}&size={size}"
                )
                books = ((data.get("data") or {}).get("books") or [])
                out = []
                for b in books:
                    bid = str(b.get("bookId") or b.get("book_id") or "")
                    name = b.get("bookName") or b.get("book_name") or ""
                    desc = b.get("description") or b.get("abstract_text") or ""
                    cover = b.get("coverUrl") or b.get("thumb_url") or ""
                    out.append(
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
                            "totalChapters": b.get("totalChapters") or b.get("chapter_total"),
                        }
                    )
                return self._send(
                    *j(
                        {
                            "code": 0,
                            "message": "success",
                            "data": {
                                "books": out,
                                "query": key,
                                "page": page,
                                "size": size,
                                "total": len(out),
                            },
                        }
                    )
                )

            m = re.fullmatch(r"/book/(\d+)", path_r) or re.fullmatch(r"/api/book/(\d+)", path_r)
            if m:
                bid = m.group(1)
                data = upstream_get(f"/book/{bid}")
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
                    "status": b.get("status"),
                }
                return self._send(*j({"code": 0, "message": "success", "data": out}))

            m = re.fullmatch(r"/toc/(\d+)", path_r) or re.fullmatch(
                r"/api/book/(\d+)/directory", path_r
            )
            if m:
                bid = m.group(1)
                data = upstream_get(f"/toc/{bid}", timeout=180)
                items = ((data.get("data") or {}).get("item_data_list") or [])
                chapters = []
                for i, it in enumerate(items):
                    iid = str(it.get("item_id") or it.get("itemId") or "")
                    url = f"{PUBLIC}/chapter/{bid}/{iid}"
                    chapters.append(
                        {
                            "item_id": iid,
                            "itemId": iid,
                            "title": it.get("title") or f"第{i+1}章",
                            "order": str(i + 1),
                            "url": url,
                        }
                    )
                return self._send(
                    *j(
                        {
                            "code": 0,
                            "message": "success",
                            "data": {
                                "book_id": bid,
                                "chapters": chapters,
                                "item_data_list": chapters,
                                "total": len(chapters),
                            },
                        }
                    )
                )

            m = re.fullmatch(r"/chapter/(\d+)/(\d+)", path_r) or re.fullmatch(
                r"/api/book/(\d+)/chapter/(\d+)", path_r
            )
            if m:
                bid, iid = m.group(1), m.group(2)
                data = upstream_get(f"/chapter/{bid}/{iid}", timeout=120)
                c = data.get("data") or {}
                text = c.get("txtContent") or c.get("content_text") or ""
                html_body = "".join(
                    f"<p>{escape_html(line)}</p>" for line in text.split("\n") if line.strip()
                )
                out = {
                    "book_id": str(c.get("bookId") or bid),
                    "item_id": str(c.get("chapterId") or iid),
                    "title": c.get("title") or "",
                    "content": html_body if html_body else text,
                    "content_text": text,
                    "txtContent": text,
                    "word_number": c.get("wordCount") or len(text),
                    "source": "app_full+unidbg",
                    "is_complete": True,
                }
                return self._send(*j({"code": 0, "message": "success", "data": out}))

            m = re.fullmatch(r"/api/export/([a-f0-9]+)", path_r) or re.fullmatch(
                r"/export/([a-f0-9]+)", path_r
            )
            if m:
                job = get_job(m.group(1))
                if not job:
                    return self._send(*j({"code": 404, "message": "job not found"}, 404))
                # don't leak internal paths
                public = {k: v for k, v in job.items() if k != "files"}
                if job.get("files"):
                    public["files"] = {
                        k: {"name": v["name"], "size": v["size"], "url": v["url"]}
                        for k, v in job["files"].items()
                    }
                return self._send(*j({"code": 0, "data": public}))

            m = re.fullmatch(r"/download/(.+)", path)
            if m:
                name = urllib.parse.unquote(m.group(1))
                if "/" in name or ".." in name or name.startswith("."):
                    return self._send(*j({"code": 400, "message": "bad name"}, 400))
                fp = EXPORT_DIR / name
                if not fp.exists() or not fp.is_file():
                    return self._send(*j({"code": 404, "message": "file not found"}, 404))
                data = fp.read_bytes()
                ctype = (
                    "application/epub+zip"
                    if name.endswith(".epub")
                    else "text/plain; charset=utf-8"
                )
                # ASCII fallback + RFC5987 for CJK filenames
                ascii_name = re.sub(r"[^A-Za-z0-9._-]+", "_", name).strip("._") or "download.bin"
                disp = (
                    f'attachment; filename="{ascii_name}"; '
                    f"filename*=UTF-8''{urllib.parse.quote(name)}"
                )
                return self._send(
                    200,
                    data,
                    ctype,
                    extra_headers={"Content-Disposition": disp},
                )

            return self._send(*j({"code": 404, "message": f"not found: {path_r}"}, 404))
        except urllib.error.HTTPError as e:
            try:
                body = e.read()
            except Exception:
                body = str(e).encode()
            self._send(e.code if e.code else 502, body)
        except Exception as e:
            print(traceback.format_exc(), flush=True)
            self._send(*j({"code": 502, "message": str(e)}, 502))

    def _serve_static(self, rel: str, content_type: str | None = None):
        rel = rel.lstrip("/")
        if ".." in rel or rel.startswith("/"):
            return self._send(*j({"code": 400, "message": "bad path"}, 400))
        fp = STATIC_DIR / (rel or "index.html")
        if not fp.exists() or not fp.is_file():
            # fallback index
            if rel in ("", "index.html"):
                return self._send(200, INDEX_HTML.encode("utf-8"), "text/html; charset=utf-8")
            return self._send(*j({"code": 404, "message": "static not found"}, 404))
        data = fp.read_bytes()
        if content_type is None:
            if rel.endswith(".css"):
                content_type = "text/css; charset=utf-8"
            elif rel.endswith(".js"):
                content_type = "application/javascript; charset=utf-8"
            elif rel.endswith(".html"):
                content_type = "text/html; charset=utf-8"
            else:
                content_type = "application/octet-stream"
        return self._send(200, data, content_type)


INDEX_HTML = r"""<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width,initial-scale=1"/>
<title>番茄全文下载</title>
<style>
  :root {
    --bg:#0b1020; --card:#141b2f; --line:#243049; --text:#e8eefc; --muted:#93a0bd;
    --accent:#6ea8fe; --accent2:#3ddc97; --danger:#ff6b6b; --warn:#ffd166;
  }
  *{box-sizing:border-box}
  body{
    margin:0; font-family: ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif;
    background: radial-gradient(1200px 600px at 10% -10%, #1a2748 0%, transparent 50%),
                radial-gradient(900px 500px at 100% 0%, #13253a 0%, transparent 45%), var(--bg);
    color:var(--text); min-height:100vh;
  }
  .wrap{max-width:980px;margin:0 auto;padding:28px 16px 60px}
  h1{margin:0 0 6px;font-size:1.6rem;letter-spacing:.02em}
  .sub{color:var(--muted);margin-bottom:22px;font-size:.95rem}
  .card{
    background:linear-gradient(180deg, rgba(255,255,255,.03), transparent 40%), var(--card);
    border:1px solid var(--line); border-radius:16px; padding:16px; margin-bottom:16px;
    box-shadow:0 10px 40px rgba(0,0,0,.25);
  }
  .search{display:flex; gap:10px}
  input[type=text]{
    flex:1; border-radius:12px; border:1px solid var(--line); background:#0e1528;
    color:var(--text); padding:12px 14px; font-size:1rem; outline:none;
  }
  input[type=text]:focus{border-color:var(--accent)}
  button{
    border:0; border-radius:12px; padding:12px 16px; font-weight:600; cursor:pointer;
    background:linear-gradient(135deg, #6ea8fe, #5b8def); color:#06101f;
  }
  button.secondary{background:#1c2740;color:var(--text);border:1px solid var(--line)}
  button:disabled{opacity:.5;cursor:not-allowed}
  .row{display:flex;gap:12px;align-items:flex-start}
  .cover{width:72px;height:96px;border-radius:8px;object-fit:cover;background:#0e1528;border:1px solid var(--line);flex:none}
  .meta{flex:1;min-width:0}
  .title{font-weight:700;font-size:1.05rem;margin-bottom:4px}
  .author{color:var(--muted);font-size:.9rem;margin-bottom:6px}
  .intro{color:#c5cee6;font-size:.88rem;line-height:1.5;display:-webkit-box;-webkit-line-clamp:3;-webkit-box-orient:vertical;overflow:hidden}
  .actions{display:flex;gap:8px;margin-top:10px;flex-wrap:wrap}
  .chip{font-size:.78rem;color:var(--muted);background:#0e1528;border:1px solid var(--line);padding:3px 8px;border-radius:999px}
  .progress{
    height:10px;background:#0e1528;border-radius:999px;overflow:hidden;border:1px solid var(--line);margin:10px 0
  }
  .bar{height:100%;width:0;background:linear-gradient(90deg,var(--accent),var(--accent2));transition:width .25s}
  .status{color:var(--muted);font-size:.9rem}
  .ok{color:var(--accent2)} .err{color:var(--danger)}
  .links a{color:var(--accent);margin-right:12px}
  .empty{color:var(--muted);padding:18px 0;text-align:center}
  .fmt label{margin-right:12px;color:var(--muted);font-size:.9rem;user-select:none}
</style>
</head>
<body>
<div class="wrap">
  <h1>番茄全文下载</h1>
  <div class="sub">App full + 签名解密 · 按批窗口拉取（对齐 batch_full 预取）· 导出 TXT / EPUB</div>

  <div class="card">
    <div class="search">
      <input id="q" type="text" placeholder="搜索书名，例如：我不是戏神" value="我不是戏神"/>
      <button id="btnSearch">搜索</button>
    </div>
    <div class="fmt" style="margin-top:12px">
      <label><input type="checkbox" id="fmtTxt" checked/> TXT</label>
      <label><input type="checkbox" id="fmtEpub" checked/> EPUB</label>
    </div>
  </div>

  <div id="results" class="card"><div class="empty">输入关键词搜索</div></div>
  <div id="job" class="card" style="display:none"></div>
</div>
<script>
const $ = (id) => document.getElementById(id);
const results = $("results");
const jobBox = $("job");
let pollTimer = null;

async function api(path, opts) {
  const r = await fetch(path, opts);
  const t = await r.text();
  let data;
  try { data = JSON.parse(t); } catch { throw new Error(t || r.statusText); }
  if (!r.ok || (data.code && data.code !== 0 && data.ok !== true)) {
    throw new Error(data.message || data.msg || ("HTTP " + r.status));
  }
  return data;
}

function fmtWords(n) {
  if (n == null) return "-";
  n = Number(n);
  if (n >= 10000) return (n/10000).toFixed(1) + " 万字";
  return n + " 字";
}

function renderBooks(books) {
  if (!books || !books.length) {
    results.innerHTML = '<div class="empty">没有结果</div>';
    return;
  }
  results.innerHTML = books.map(b => `
    <div class="row" style="padding:12px 0;border-bottom:1px solid var(--line)">
      <img class="cover" src="${b.coverUrl || b.thumb_url || ''}" onerror="this.style.opacity=.2"/>
      <div class="meta">
        <div class="title">${escapeHtml(b.book_name || b.bookName || "")}</div>
        <div class="author">${escapeHtml(b.author || "")}
          <span class="chip">${escapeHtml(b.category || "未分类")}</span>
          <span class="chip">${fmtWords(b.wordNumber || b.word_count)}</span>
        </div>
        <div class="intro">${escapeHtml(b.description || b.abstract_text || "")}</div>
        <div class="actions">
          <button onclick="startExport('${b.book_id || b.bookId}')">下载全文</button>
          <button class="secondary" onclick="window.open('/book/${b.book_id || b.bookId}','_blank')">详情 JSON</button>
        </div>
      </div>
    </div>
  `).join("");
}

function escapeHtml(s) {
  return String(s||"").replace(/[&<>"']/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;","\"":"&quot;","'":"&#39;"}[c]));
}

async function doSearch() {
  const q = $("q").value.trim();
  if (!q) return;
  $("btnSearch").disabled = true;
  results.innerHTML = '<div class="empty">搜索中…</div>';
  try {
    const data = await api("/search?key=" + encodeURIComponent(q) + "&page=1&size=10");
    renderBooks(data.data.books || []);
  } catch (e) {
    results.innerHTML = `<div class="empty err">搜索失败：${escapeHtml(e.message)}</div>`;
  } finally {
    $("btnSearch").disabled = false;
  }
}

function selectedFormats() {
  const f = [];
  if ($("fmtTxt").checked) f.push("txt");
  if ($("fmtEpub").checked) f.push("epub");
  return f.length ? f : ["txt","epub"];
}

async function startExport(bookId) {
  jobBox.style.display = "block";
  jobBox.innerHTML = `<div class="status">创建任务…</div><div class="progress"><div class="bar" id="bar"></div></div>`;
  try {
    const data = await api("/api/export", {
      method: "POST",
      headers: {"Content-Type":"application/json"},
      body: JSON.stringify({book_id: bookId, formats: selectedFormats()})
    });
    const jobId = data.data.job_id;
    pollJob(jobId);
  } catch (e) {
    jobBox.innerHTML = `<div class="err">创建失败：${escapeHtml(e.message)}</div>`;
  }
}

function pollJob(jobId) {
  if (pollTimer) clearInterval(pollTimer);
  const tick = async () => {
    try {
      const data = await api("/api/export/" + jobId);
      const j = data.data;
      const pct = j.progress || 0;
      const bar = document.getElementById("bar");
      if (bar) bar.style.width = pct + "%";
      let links = "";
      if (j.files) {
        links = '<div class="links" style="margin-top:10px">' +
          Object.entries(j.files).map(([k,v]) =>
            `<a href="${v.url}">下载 ${k.toUpperCase()} (${(v.size/1024/1024).toFixed(2)} MB)</a>`
          ).join("") + "</div>";
      }
      const meta = j.meta ? `${escapeHtml(j.meta.book_name)} / ${escapeHtml(j.meta.author||"")}` : "";
      const extra = j.status === "done"
        ? `<div class="ok">完成 · ${j.total||0} 章 · ${fmtWords(j.total_words)} · ${j.elapsed_sec||0}s</div>`
        : `<div class="status">${escapeHtml(j.message||"")} · ${j.done||0}/${j.total||0}` +
          (j.rate ? ` · ${j.rate} 章/s` : "") +
          (j.eta_sec ? ` · ETA ${Math.ceil(j.eta_sec/60)} 分` : "") + `</div>`;
      jobBox.innerHTML = `
        <div style="font-weight:700;margin-bottom:6px">${meta || "导出任务 " + jobId}</div>
        <div class="progress"><div class="bar" style="width:${pct}%"></div></div>
        ${extra}
        ${j.status==="error" ? `<div class="err">${escapeHtml(j.message||"error")}</div>` : ""}
        ${links}`;
      if (j.status === "done" || j.status === "error") {
        clearInterval(pollTimer);
        pollTimer = null;
      }
    } catch (e) {
      jobBox.innerHTML += `<div class="err">轮询失败：${escapeHtml(e.message)}</div>`;
    }
  };
  tick();
  pollTimer = setInterval(tick, 1500);
}

$("btnSearch").onclick = doSearch;
$("q").addEventListener("keydown", e => { if (e.key === "Enter") doSearch(); });
doSearch();
</script>
</body>
</html>
"""


def main():
    # ensure index on disk too
    (STATIC_DIR / "index.html").write_text(INDEX_HTML, encoding="utf-8")
    httpd = ThreadingHTTPServer((HOST, PORT), Handler)
    print(f"fq-full-web http://{HOST}:{PORT} upstream={UPSTREAM} batch_window={BATCH_WINDOW}", flush=True)
    httpd.serve_forever()


if __name__ == "__main__":
    main()
