#!/usr/bin/env python3
"""Build PUA->char map for fanqie awesome-font (x-tt-zhal).

Pipeline:
  1) Download official .otf for font_id
  2) FreeType-render glyphs vs SourceHanSansSC-Normal
  3) Optional supervised refine: align search raw fields to book detail pages

Usage:
  python tools/build_font_map.py --from-search '我不是戏神'
  python tools/build_font_map.py --font-id c207f68a84deae3
  python tools/build_font_map.py --font-id c207f68a84deae3 --no-supervise

Requires: fonttools pillow numpy freetype-py
"""
from __future__ import annotations

import argparse
import json
import re
import sys
import time
import urllib.parse
import urllib.request
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MAP_DIR = ROOT / "font_maps"
CACHE = ROOT / "data" / "font_cache"
REF_PATH = CACHE / "SourceHanSansSC-Normal.otf"
REF_URL = (
    "https://cdn.jsdelivr.net/gh/adobe-fonts/source-han-sans@release/"
    "OTF/SimplifiedChinese/SourceHanSansSC-Normal.otf"
)
SIZE, PX = 64, 48


def ensure_deps():
    missing = []
    for mod in ("fontTools", "numpy", "PIL", "freetype"):
        try:
            __import__(mod if mod != "PIL" else "PIL.Image")
        except ImportError:
            missing.append(mod if mod != "PIL" else "pillow")
    if missing:
        print("Install: pip install fonttools pillow numpy freetype-py", file=sys.stderr)
        print("missing:", missing, file=sys.stderr)
        sys.exit(1)


def download(url: str, dest: Path, timeout: int = 120):
    dest.parent.mkdir(parents=True, exist_ok=True)
    print(f"download {url} -> {dest}")
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        dest.write_bytes(r.read())


def ensure_ref() -> Path:
    if REF_PATH.exists() and REF_PATH.stat().st_size > 1_000_000:
        return REF_PATH
    download(REF_URL, REF_PATH)
    return REF_PATH


def http_get(url: str):
    req = urllib.request.Request(
        url, headers={"User-Agent": "Mozilla/5.0", "Referer": "https://fanqienovel.com/"}
    )
    with urllib.request.urlopen(req, timeout=25) as r:
        return r.read(), {k.lower(): v for k, v in r.headers.items()}


def fetch_zhal_and_font(query: str) -> tuple[str, str, Path]:
    q = urllib.parse.quote(query)
    url = (
        "https://fanqienovel.com/api/author/search/search_book/v0"
        f"?filter=127,127,127,127&page_count=1&page_index=0&query_type=0&query_word={q}"
    )
    body, headers = http_get(url)
    zhal = headers.get("x-tt-zhal", "")
    m = dict(re.findall(r"([a-z0-9]+)=([^;]+)", zhal, flags=re.I))
    font_id = m.get("f") or ""
    d1 = m.get("d1") or "lf6-awef.bytetos.com"
    if not font_id:
        raise SystemExit(f"no x-tt-zhal font id: {zhal!r}")
    otf = CACHE / f"{font_id}.otf"
    if not otf.exists():
        download(f"https://{d1}/obj/awesome-font/c/{font_id}.otf", otf)
    return font_id, zhal, otf


def render_char(face, codepoint: int, size=SIZE, px=PX):
    import freetype
    import numpy as np

    face.set_pixel_sizes(0, px)
    face.load_char(codepoint, freetype.FT_LOAD_RENDER | freetype.FT_LOAD_TARGET_NORMAL)
    bitmap = face.glyph.bitmap
    w, h = bitmap.width, bitmap.rows
    img = np.zeros((size, size), dtype=np.uint8)
    if w == 0 or h == 0:
        return img
    buf = np.array(bitmap.buffer, dtype=np.uint8).reshape(h, w)
    x0 = max((size - w) // 2, 0)
    y0 = max((size - h) // 2, 0)
    ww = min(w, size - x0)
    hh = min(h, size - y0)
    img[y0 : y0 + hh, x0 : x0 + ww] = buf[:hh, :ww]
    return img


def ahash(arr, bits=16):
    from PIL import Image
    import numpy as np

    small = np.array(Image.fromarray(arr).resize((bits, bits), Image.BILINEAR), dtype=np.float32)
    avg = small.mean()
    h = 0
    for i, b in enumerate((small > avg).flatten()):
        if b:
            h |= 1 << i
    return h


def hamming(a, b):
    return (a ^ b).bit_count()


def allowed_cp(cp: int) -> bool:
    if 0x20 <= cp <= 0x7E:
        return True
    if 0x3000 <= cp <= 0x303F:
        return True
    if 0xFF01 <= cp <= 0xFF5E:
        return True
    if 0x4E00 <= cp <= 0x9FFF:
        return True
    if 0x3400 <= cp <= 0x4DBF:
        return True
    return False


def category(cp: int) -> int:
    if 0x4E00 <= cp <= 0x9FA5:
        return 0
    if 0x9FA6 <= cp <= 0x9FFF:
        return 1
    if 0x20 <= cp <= 0x7E or 0x3000 <= cp <= 0x303F or 0xFF01 <= cp <= 0xFF5E:
        return 0
    if 0x3400 <= cp <= 0x4DBF:
        return 4
    return 9


def build_map_freetype(otf_path: Path, ref_path: Path) -> dict[int, str]:
    import freetype
    import numpy as np
    from fontTools.ttLib import TTFont

    obf_face = freetype.Face(str(otf_path))
    ref_face = freetype.Face(str(ref_path))
    obf_cmap = TTFont(str(otf_path)).getBestCmap()
    ref_cmap = TTFont(str(ref_path)).getBestCmap()

    print("indexing reference (freetype)...")
    ref_by_cp = {}
    ref_by_hash = defaultdict(list)
    for cp in ref_cmap:
        if not allowed_cp(cp):
            continue
        try:
            arr = render_char(ref_face, cp)
        except Exception:
            continue
        if arr.sum() == 0 and cp > 0x7F:
            continue
        h = ahash(arr)
        ref_by_cp[cp] = (h, arr)
        ref_by_hash[h].append(cp)
    print(f"ref glyphs: {len(ref_by_cp)}")

    matched: dict[int, str] = {}
    for pua in sorted(obf_cmap):
        arr = render_char(obf_face, pua)
        h = ahash(arr)
        cands = set()
        for rh, cps in ref_by_hash.items():
            if hamming(h, rh) <= 32:
                cands.update(cps)
        if len(cands) < 8:
            for rh, cps in ref_by_hash.items():
                if hamming(h, rh) <= 56:
                    cands.update(cps)
        scored = []
        for cp in cands:
            _, rarr = ref_by_cp[cp]
            mse = float(np.mean((arr.astype(np.float32) - rarr.astype(np.float32)) ** 2))
            scored.append((mse, cp))
        if not scored:
            continue
        scored.sort()
        best_mse = scored[0][0]
        cjk = [
            x
            for x in scored
            if category(x[1]) == 0 and x[0] <= best_mse + max(800.0, best_mse * 0.5)
        ]
        pool = cjk if cjk else [x for x in scored if x[0] <= best_mse + max(300.0, best_mse * 0.25)]
        if not pool:
            pool = scored[:5]
        pool.sort(key=lambda x: (category(x[1]), x[0]))
        best_mse, best = pool[0]
        if best_mse < 6000:
            matched[pua] = chr(best)
    print(f"matched {len(matched)}/{len(obf_cmap)}")
    return matched


def parse_initial_state(html: str):
    for key in ("window.__INITIAL_STATE__=", "window.__INITIAL_STATE__ ="):
        i = html.find(key)
        if i >= 0:
            i += len(key)
            break
    else:
        return None
    while i < len(html) and html[i].isspace():
        i += 1
    if i >= len(html) or html[i] != "{":
        return None
    depth = 0
    begin = i
    while i < len(html):
        c = html[i]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return json.loads(html[begin : i + 1])
        elif c == '"':
            i += 1
            while i < len(html):
                if html[i] == "\\":
                    i += 2
                    continue
                if html[i] == '"':
                    break
                i += 1
        i += 1
    return None


def book_detail(book_id: str) -> dict:
    body, _ = http_get(f"https://fanqienovel.com/page/{book_id}")
    html = body.decode("utf-8", "ignore")
    st = parse_initial_state(html) or {}
    page = st.get("page") or {}
    return {
        "book_name": page.get("bookName") or "",
        "author": page.get("author") or page.get("authorName") or "",
        "abstract": page.get("abstract") or page.get("description") or "",
    }


def align(raw: str, clean: str):
    pairs = []
    if not raw or not clean:
        return pairs
    if len(raw) == len(clean):
        for a, b in zip(raw, clean):
            if 0xE000 <= ord(a) <= 0xF8FF:
                pairs.append((ord(a), b))
        return pairs
    i = j = 0
    while i < len(raw) and j < len(clean):
        rc, cc = raw[i], clean[j]
        if 0xE000 <= ord(rc) <= 0xF8FF:
            pairs.append((ord(rc), cc))
            i += 1
            j += 1
        elif rc == cc:
            i += 1
            j += 1
        else:
            k = clean.find(rc, j, min(len(clean), j + 4)) if ord(rc) < 0xE000 else -1
            if k >= 0:
                j = k + 1
                i += 1
            else:
                i += 1
    return pairs


def supervise_refine(mp: dict[int, str], queries: list[str]) -> dict[int, str]:
    votes: dict[int, Counter] = defaultdict(Counter)
    for q in queries:
        url = (
            "https://fanqienovel.com/api/author/search/search_book/v0"
            f"?filter=127,127,127,127&page_count=8&page_index=0&query_type=0"
            f"&query_word={urllib.parse.quote(q)}"
        )
        try:
            body, _ = http_get(url)
            payload = json.loads(body)
        except Exception as e:
            print("search fail", q, e)
            continue
        books = (payload.get("data") or {}).get("search_book_data_list") or []
        print(f"supervise {q}: {len(books)} books")
        for b in books:
            bid = str(b.get("book_id") or "")
            if not bid:
                continue
            try:
                det = book_detail(bid)
            except Exception:
                continue
            for raw_k, clean_k, w in [
                ("book_name", "book_name", 5),
                ("author", "author", 4),
                ("book_abstract", "abstract", 1),
            ]:
                raw = b.get(raw_k) or ""
                clean = det.get(clean_k) or ""
                if raw_k == "book_abstract" and abs(len(raw) - len(clean)) > 20:
                    continue
                for pua, ch in align(raw, clean):
                    if ch and not ch.isspace():
                        votes[pua][ch] += w
            time.sleep(0.03)

    corrections = 0
    for pua, ctr in votes.items():
        ch, n = ctr.most_common(1)[0]
        total = sum(ctr.values())
        if n >= 3 and n / total >= 0.5:
            if mp.get(pua) != ch:
                corrections += 1
            mp[pua] = ch
    print(f"supervised corrections: {corrections}")
    return mp


def main():
    ensure_deps()
    ap = argparse.ArgumentParser()
    ap.add_argument("--font-id")
    ap.add_argument("--otf")
    ap.add_argument("--from-search", default="我不是戏神")
    ap.add_argument("--out-dir", default=str(MAP_DIR))
    ap.add_argument("--no-supervise", action="store_true")
    args = ap.parse_args()

    MAP_DIR.mkdir(parents=True, exist_ok=True)
    CACHE.mkdir(parents=True, exist_ok=True)
    ref = ensure_ref()

    if args.otf and args.font_id:
        font_id = args.font_id
        otf = Path(args.otf)
    elif args.font_id:
        font_id = args.font_id
        otf = CACHE / f"{font_id}.otf"
        if not otf.exists():
            download(f"https://lf6-awef.bytetos.com/obj/awesome-font/c/{font_id}.otf", otf)
    else:
        font_id, zhal, otf = fetch_zhal_and_font(args.from_search)
        print("zhal", zhal)

    mp = build_map_freetype(otf, ref)
    if not args.no_supervise:
        queries = [
            args.from_search,
            "开局斗圣",
            "斗破苍穹",
            "遮天",
            "凡人修仙传",
            "道诡异仙",
            "深空彼岸",
            "十日终焉",
            "夜的命名术",
            "大奉打更人",
            "诡秘之主",
            "全职高手",
        ]
        # unique preserve order
        seen = set()
        qs = []
        for q in queries:
            if q not in seen:
                seen.add(q)
                qs.append(q)
        mp = supervise_refine(mp, qs)

    out = {
        "font_id": font_id,
        "count": len(mp),
        "map": {str(k): v for k, v in sorted(mp.items())},
    }
    out_path = Path(args.out_dir) / f"{font_id}.json"
    out_path.write_text(json.dumps(out, ensure_ascii=False, indent=0), encoding="utf-8")
    print("wrote", out_path, "count", out["count"])

    # live smoke
    try:
        body, _ = http_get(
            "https://fanqienovel.com/api/author/search/search_book/v0"
            f"?filter=127,127,127,127&page_count=5&page_index=0&query_type=0"
            f"&query_word={urllib.parse.quote(args.from_search)}"
        )
        payload = json.loads(body)

        def dec(s: str) -> str:
            return "".join(mp.get(ord(c), c) for c in s)

        for b in (payload.get("data") or {}).get("search_book_data_list") or []:
            print(dec(b.get("book_name", "")), "|", dec(b.get("author", "")))
    except Exception as e:
        print("live check skipped:", e)


if __name__ == "__main__":
    main()
