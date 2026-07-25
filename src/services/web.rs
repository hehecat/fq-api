use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

use crate::fontmap::{looks_obfuscated, FontMapStore, ZhalInfo};
use crate::models::{BookDetail, ChapterItem, DirectoryData, SearchBook};

#[derive(Clone)]
pub struct WebClient {
    http: reqwest::Client,
    fonts: Arc<FontMapStore>,
}

impl WebClient {
    pub fn new() -> Result<Self> {
        Self::with_fonts(Arc::new(FontMapStore::new()))
    }

    pub fn with_fonts(fonts: Arc<FontMapStore>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .timeout(Duration::from_secs(25))
            .gzip(true)
            .brotli(true)
            .deflate(true)
            .build()?;
        Ok(Self { http, fonts })
    }

    pub fn fonts(&self) -> &FontMapStore {
        &self.fonts
    }

    pub async fn search_books(&self, q: &str, page: u32, size: u32) -> Result<serde_json::Value> {
        let url = format!(
            "https://fanqienovel.com/api/author/search/search_book/v0?filter=127,127,127,127&page_count={}&page_index={}&query_type=0&query_word={}",
            size,
            page,
            urlencoding::encode(q)
        );
        let resp = self
            .http
            .get(&url)
            .header("Referer", "https://fanqienovel.com/")
            .header("Accept", "application/json")
            .send()
            .await?
            .error_for_status()?;

        let zhal_raw = resp
            .headers()
            .get("x-tt-zhal")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let zhal = zhal_raw.as_deref().and_then(ZhalInfo::parse);

        let v: Value = resp.json().await?;
        if v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1) != 0 {
            bail!("search failed: {}", v);
        }
        let list = v
            .pointer("/data/search_book_data_list")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();

        let mut books: Vec<SearchBook> = list
            .into_iter()
            .map(|b| SearchBook {
                book_id: s(&b, "book_id"),
                book_name: s(&b, "book_name"),
                author: s(&b, "author"),
                abstract_text: s(&b, "book_abstract"),
                thumb_url: s(&b, "thumb_url"),
                word_count: i64_opt(&b, "word_count").or_else(|| i64_opt(&b, "word_number")),
                read_count: i64_opt(&b, "read_count"),
                creation_status: i64_opt(&b, "creation_status"),
                first_chapter_id: opt_s(&b, "first_chapter_id"),
                last_chapter_id: opt_s(&b, "last_chapter_id"),
                last_chapter_title: opt_s(&b, "last_chapter_title"),
                category: opt_s(&b, "category"),
            })
            .collect();

        // 1) 字体映射直解（快路径）
        let mut font_decoded = 0usize;
        if let Some(ref z) = zhal {
            if self.fonts.try_load_font(&z.font_id).is_some() {
                for book in books.iter_mut() {
                    let (name, u1) = self.fonts.decode_optional(Some(z), &book.book_name);
                    let (author, u2) = self.fonts.decode_optional(Some(z), &book.author);
                    let (abs, u3) = self.fonts.decode_optional(Some(z), &book.abstract_text);
                    let (cat, u4) = match &book.category {
                        Some(c) => {
                            let (d, u) = self.fonts.decode_optional(Some(z), c);
                            (Some(d), u)
                        }
                        None => (None, false),
                    };
                    let (last, u5) = match &book.last_chapter_title {
                        Some(c) => {
                            let (d, u) = self.fonts.decode_optional(Some(z), c);
                            (Some(d), u)
                        }
                        None => (None, false),
                    };
                    book.book_name = name;
                    book.author = author;
                    book.abstract_text = abs;
                    book.category = cat;
                    book.last_chapter_title = last;
                    if u1 || u2 || u3 || u4 || u5 {
                        font_decoded += 1;
                    }
                }
            } else {
                tracing::warn!(
                    "no font map for id={} (dir={}); will fallback enrich if needed",
                    z.font_id,
                    self.fonts.map_dir().display()
                );
            }
        }

        // 2) 仍有混淆字时，详情页回填兜底
        let still_obfuscated = books.iter().any(|b| {
            looks_obfuscated(&b.book_name)
                || looks_obfuscated(&b.author)
                || looks_obfuscated(&b.abstract_text)
        });
        let mut enriched = false;
        if still_obfuscated {
            enriched = true;
            let futs = books.iter().map(|b| {
                let id = b.book_id.clone();
                async move { (id.clone(), self.book_detail(&id).await) }
            });
            let details = futures_util::future::join_all(futs).await;
            for (book, (id, det)) in books.iter_mut().zip(details) {
                match det {
                    Ok(d) => {
                        if book.book_id.is_empty() {
                            book.book_id = id;
                        }
                        if !d.book_name.is_empty() {
                            book.book_name = d.book_name;
                        }
                        if !d.author.is_empty() {
                            book.author = d.author;
                        }
                        if !d.abstract_text.is_empty() {
                            book.abstract_text = d.abstract_text;
                        }
                        if book.thumb_url.is_empty() && !d.thumb_url.is_empty() {
                            book.thumb_url = d.thumb_url;
                        }
                        if d.word_number.is_some() {
                            book.word_count = d.word_number;
                        }
                        if book.read_count.is_none() {
                            book.read_count = d.read_count;
                        }
                        if book.creation_status.is_none() {
                            book.creation_status = d.creation_status;
                        }
                        if let Some(t) = d.last_chapter_title {
                            if !t.is_empty() {
                                book.last_chapter_title = Some(t);
                            }
                        }
                        if let Some(t) = d.last_chapter_id {
                            if !t.is_empty() {
                                book.last_chapter_id = Some(t);
                            }
                        }
                        if book.category.as_ref().map(|c| looks_obfuscated(c)).unwrap_or(true) {
                            if let Some(c) = d.complete_category.or(d.category) {
                                if !c.is_empty() {
                                    book.category = Some(c);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("enrich search book {} failed: {e:#}", book.book_id);
                    }
                }
            }
        }

        Ok(serde_json::json!({
            "query": q,
            "page": page,
            "size": size,
            "total": books.len(),
            "books": books,
            "font_decoded": font_decoded > 0,
            "font_id": zhal.as_ref().map(|z| z.font_id.clone()),
            "zhal": zhal_raw,
            "enriched": enriched,
        }))
    }

    pub async fn book_detail(&self, book_id: &str) -> Result<BookDetail> {
        let url = format!("https://fanqienovel.com/page/{book_id}");
        let html = self
            .http
            .get(&url)
            .header("Referer", "https://fanqienovel.com/")
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let state = extract_initial_state(&html).context("book page INITIAL_STATE")?;
        let page = state.get("page").cloned().unwrap_or(Value::Null);
        Ok(BookDetail {
            book_id: s(&page, "bookId").if_empty(book_id),
            book_name: s(&page, "bookName"),
            author: first_nonempty(&[s(&page, "author"), s(&page, "authorName")]),
            author_id: opt_s(&page, "authorId"),
            abstract_text: first_nonempty(&[s(&page, "abstract"), s(&page, "description")]),
            thumb_url: first_nonempty(&[s(&page, "thumbUrl"), s(&page, "thumbUri")]),
            word_number: i64_opt(&page, "wordNumber"),
            read_count: i64_opt(&page, "readCount"),
            creation_status: i64_opt(&page, "creationStatus"),
            chapter_total: i64_opt(&page, "chapterTotal"),
            last_chapter_id: opt_s(&page, "lastChapterItemId"),
            last_chapter_title: opt_s(&page, "lastChapterTitle"),
            category: opt_s(&page, "category"),
            complete_category: opt_s(&page, "completeCategory"),
            genre: opt_s(&page, "genre").or_else(|| i64_opt(&page, "genre").map(|x| x.to_string())),
            volume_name_list: page
                .get("volumeNameList")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
                .unwrap_or_default(),
        })
    }

    pub async fn directory(&self, book_id: &str) -> Result<DirectoryData> {
        let url = format!("https://fanqienovel.com/api/reader/directory/detail?bookId={book_id}");
        let v: Value = self
            .http
            .get(&url)
            .header("Referer", &format!("https://fanqienovel.com/page/{book_id}"))
            .header("Accept", "application/json")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1) != 0 {
            bail!("directory failed: {}", v);
        }
        let data = v.get("data").cloned().unwrap_or(Value::Null);
        let all_item_ids: Vec<String> = data
            .get("allItemIds")
            .and_then(|x| x.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default();
        let volume_name_list: Vec<String> = data
            .get("volumeNameList")
            .and_then(|x| x.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default();

        let mut chapters = Vec::new();
        if let Some(vols) = data.get("chapterListWithVolume").and_then(|x| x.as_array()) {
            for vol in vols {
                if let Some(arr) = vol.as_array() {
                    for ch in arr {
                        chapters.push(ChapterItem {
                            item_id: first_nonempty(&[s(ch, "itemId"), s(ch, "item_id")]),
                            title: s(ch, "title"),
                            volume_name: opt_s(ch, "volume_name").or_else(|| opt_s(ch, "volumeName")),
                            need_pay: i64_opt(ch, "needPay"),
                            order: opt_s(ch, "realChapterOrder")
                                .or_else(|| i64_opt(ch, "realChapterOrder").map(|x| x.to_string())),
                        });
                    }
                }
            }
        }
        if chapters.is_empty() {
            for (i, id) in all_item_ids.iter().enumerate() {
                chapters.push(ChapterItem {
                    item_id: id.clone(),
                    title: format!("第{}章", i + 1),
                    volume_name: None,
                    need_pay: Some(0),
                    order: Some((i + 1).to_string()),
                });
            }
        }
        let total = if !all_item_ids.is_empty() {
            all_item_ids.len()
        } else {
            chapters.len()
        };
        Ok(DirectoryData {
            book_id: book_id.to_string(),
            all_item_ids,
            chapters,
            volume_name_list,
            total,
        })
    }

    /// 章节 SSR：优先用预生成字体映射表 O(n) 查表还原；不跑 FreeType。
    /// 返回 (book_id, book_name, title, next, pre, content, font_id, font_decoded)
    pub async fn chapter_ssr(
        &self,
        item_id: &str,
    ) -> Result<(
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        bool,
    )> {
        let url = format!("https://fanqienovel.com/reader/{item_id}");
        let html = self
            .http
            .get(&url)
            .header("Referer", "https://fanqienovel.com/")
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        let font_id = extract_reader_font_id(&html);
        let state = extract_initial_state(&html).context("reader INITIAL_STATE")?;
        let ch = state
            .pointer("/reader/chapterData")
            .cloned()
            .unwrap_or(Value::Null);
        let title = s(&ch, "title");
        let book_name = s(&ch, "bookName");
        let book_id = s(&ch, "bookId");
        let mut content = s(&ch, "content");
        let next = opt_s(&ch, "nextItemId");
        let pre = opt_s(&ch, "preItemId");
        if content.is_empty() {
            bail!("empty chapter content from SSR");
        }

        let mut font_decoded = false;
        if looks_obfuscated(&content) {
            if let Some(ref fid) = font_id {
                if let Some(map) = self.fonts.try_load_font(fid) {
                    content = crate::fontmap::decode_text(&content, &map);
                    font_decoded = !looks_obfuscated(&content);
                } else {
                    tracing::warn!(
                        "reader font map missing: {} (dir={})",
                        fid,
                        self.fonts.map_dir().display()
                    );
                }
            }
        }

        Ok((
            book_id,
            book_name,
            title,
            next,
            pre,
            Some(content),
            font_id,
            font_decoded,
        ))
    }
}

fn extract_reader_font_id(html: &str) -> Option<String> {
    // awesome-font/c/<font_id>.(woff2|woff|otf)
    const MARK: &str = "awesome-font/c/";
    let mut i = 0;
    while let Some(pos) = html[i..].find(MARK) {
        let start = i + pos + MARK.len();
        let rest = &html[start..];
        let end = rest
            .find(|c: char| !c.is_ascii_hexdigit())
            .unwrap_or(rest.len());
        let id = &rest[..end];
        if id.len() >= 8 {
            return Some(id.to_string());
        }
        i = start + end.max(1);
    }
    None
}

fn extract_initial_state(html: &str) -> Result<Value> {
    let markers = [
        "window.__INITIAL_STATE__=",
        "window.__INITIAL_STATE__ =",
    ];
    let mut start = None;
    for m in markers {
        if let Some(i) = html.find(m) {
            start = Some(i + m.len());
            break;
        }
    }
    let start = start.ok_or_else(|| anyhow!("INITIAL_STATE not found"))?;
    let bytes = html.as_bytes();
    let mut i = start;
    while i < bytes.len() && (bytes[i] as char).is_whitespace() {
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b'{' {
        bail!("INITIAL_STATE not object");
    }
    let mut depth = 0i32;
    let begin = i;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c == '{' {
            depth += 1;
        } else if c == '}' {
            depth -= 1;
            if depth == 0 {
                let raw = &html[begin..=i];
                return Ok(serde_json::from_str(raw)?);
            }
        } else if c == '"' {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' {
                    break;
                }
                i += 1;
            }
        }
        i += 1;
    }
    bail!("unbalanced INITIAL_STATE")
}

fn s(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(|x| {
            x.as_str()
                .map(|s| s.to_string())
                .or_else(|| x.as_i64().map(|n| n.to_string()))
                .or_else(|| x.as_f64().map(|n| n.to_string()))
                .or_else(|| x.as_bool().map(|b| b.to_string()))
        })
        .unwrap_or_default()
}

fn opt_s(v: &Value, k: &str) -> Option<String> {
    let t = s(v, k);
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

fn i64_opt(v: &Value, k: &str) -> Option<i64> {
    v.get(k).and_then(|x| {
        x.as_i64()
            .or_else(|| x.as_f64().map(|f| f as i64))
            .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
    })
}

fn first_nonempty(xs: &[String]) -> String {
    xs.iter()
        .find(|s| !s.is_empty())
        .cloned()
        .unwrap_or_default()
}

trait IfEmpty {
    fn if_empty(self, fallback: &str) -> String;
}
impl IfEmpty for String {
    fn if_empty(self, fallback: &str) -> String {
        if self.is_empty() {
            fallback.to_string()
        } else {
            self
        }
    }
}
