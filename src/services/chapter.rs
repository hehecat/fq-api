use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;

use crate::crypto::{html_to_text, CryptoStore};
use crate::models::ChapterData;
use crate::services::WebClient;

#[derive(Clone)]
pub struct ChapterService {
    web: WebClient,
    crypto: Arc<CryptoStore>,
    http: reqwest::Client,
    cache_dir: PathBuf,
    device_serial: String,
    enable_adb: bool,
    signer_url: Option<String>,
}

impl ChapterService {
    pub fn new(web: WebClient, crypto: Arc<CryptoStore>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent("com.dragon.read/7.2.9.32")
            .timeout(Duration::from_secs(30))
            .gzip(true)
            .build()?;
        let cache_dir = PathBuf::from(
            std::env::var("FQ_CHAPTER_CACHE")
                .unwrap_or_else(|_| "data/chapters/decoded_json".into()),
        );
        let device_serial = std::env::var("FQ_ADB_SERIAL").unwrap_or_else(|_| "127.0.0.1:16384".into());
        let enable_adb = std::env::var("FQ_ENABLE_ADB")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let signer_url = std::env::var("FQ_SIGNER_URL").ok().filter(|s| !s.is_empty());
        Ok(Self {
            web,
            crypto,
            http,
            cache_dir,
            device_serial,
            enable_adb,
            signer_url,
        })
    }

    pub async fn get_chapter(&self, book_id: &str, item_id: &str, include_html: bool) -> Result<ChapterData> {
        // 1) local encrypted cache
        if let Some(data) = self.from_local_cache(book_id, item_id, include_html)? {
            return Ok(data);
        }

        // 2) try signed app full if signer configured
        if self.signer_url.is_some() {
            match self.from_app_full(book_id, item_id, include_html).await {
                Ok(v) => return Ok(v),
                Err(e) => tracing::warn!("app full failed: {e:#}"),
            }
        }

        // 3) try adb pull after triggering reader deep link (optional)
        if self.enable_adb {
            match self.from_adb(book_id, item_id, include_html).await {
                Ok(v) => return Ok(v),
                Err(e) => tracing::warn!("adb chapter failed: {e:#}"),
            }
        }

        // 4) SSR fallback：预生成字体表 O(n) 查表还原，并落盘明文缓存
        let (bid, book_name, title, next, pre, content_html, font_id, font_decoded) =
            self.web.chapter_ssr(item_id).await?;
        let html = content_html.unwrap_or_default();
        // SSR content 已是纯文本（带 <p> 时再剥标签）
        let text = if html.contains('<') {
            html_to_text(&html)
        } else {
            html.clone()
        };
        let bid = if bid.is_empty() {
            book_id.to_string()
        } else {
            bid
        };
        if font_decoded || !crate::fontmap::looks_obfuscated(&text) {
            let _ = self.save_plaintext_cache(&bid, &book_name, item_id, &title, &text);
        }
        let source = if font_decoded {
            format!("web_ssr+font:{}", font_id.unwrap_or_else(|| "unknown".into()))
        } else {
            "web_ssr".into()
        };
        Ok(ChapterData {
            book_id: bid,
            book_name,
            item_id: item_id.to_string(),
            title,
            content: text,
            content_html: if include_html { Some(html) } else { None },
            next_item_id: next,
            pre_item_id: pre,
            word_number: None,
            source,
        })
    }

    fn plaintext_dir() -> PathBuf {
        PathBuf::from(
            std::env::var("FQ_PLAINTEXT_DIR")
                .unwrap_or_else(|_| "data/chapters/plaintext".into()),
        )
    }

    fn save_plaintext_cache(
        &self,
        book_id: &str,
        book_name: &str,
        item_id: &str,
        title: &str,
        text: &str,
    ) -> Result<()> {
        let dir = Self::plaintext_dir();
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{item_id}.txt"));
        let body = format!("{book_name}\n{title}\n{book_id}\n{item_id}\n{text}");
        std::fs::write(path, body)?;
        Ok(())
    }

    fn from_local_cache(&self, book_id: &str, item_id: &str, include_html: bool) -> Result<Option<ChapterData>> {
        // 0) already-decoded plaintext (fast path, no network / no crypto)
        let plain = Self::plaintext_dir().join(format!("{item_id}.txt"));
        if plain.exists() {
            let text = std::fs::read_to_string(&plain)?;
            let mut lines = text.lines();
            let book_name = lines.next().unwrap_or("").to_string();
            let title = lines.next().unwrap_or("").to_string();
            let _bid = lines.next().unwrap_or("").to_string();
            let _iid = lines.next().unwrap_or("").to_string();
            let body = lines.collect::<Vec<_>>().join("\n");
            return Ok(Some(ChapterData {
                book_id: book_id.to_string(),
                book_name,
                item_id: item_id.to_string(),
                title,
                content: body,
                content_html: None,
                next_item_id: None,
                pre_item_id: None,
                word_number: None,
                source: "plaintext_cache".into(),
            }));
        }

        let p = self.cache_dir.join(format!("{item_id}.json"));
        if !p.exists() {
            return Ok(None);
        }
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p)?)?;
        self.decode_item_json(&v, book_id, item_id, include_html, "local_cache")
            .map(Some)
    }

    fn decode_item_json(
        &self,
        v: &Value,
        book_id: &str,
        item_id: &str,
        include_html: bool,
        source: &str,
    ) -> Result<ChapterData> {
        let content_b64 = v
            .get("content")
            .and_then(|x| x.as_str())
            .ok_or_else(|| anyhow!("missing content"))?;
        let key_version = v
            .get("key_version")
            .and_then(|x| x.as_i64())
            .or_else(|| v.get("keyVersion").and_then(|x| x.as_i64()))
            .unwrap_or(0);
        let html = self.crypto.decrypt_chapter_content(content_b64, key_version)?;
        let text = html_to_text(&html);
        let title = v
            .get("name")
            .or_else(|| v.get("title"))
            .or_else(|| v.get("chapter_title"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let book_name = v.get("book_name").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let bid = v
            .get("book_id")
            .and_then(|x| x.as_str())
            .unwrap_or(book_id)
            .to_string();
        Ok(ChapterData {
            book_id: bid,
            book_name,
            item_id: item_id.to_string(),
            title,
            content: text,
            content_html: if include_html { Some(html) } else { None },
            next_item_id: None,
            pre_item_id: None,
            word_number: v.get("word_number").and_then(|x| x.as_i64()),
            source: source.into(),
        })
    }

    async fn from_app_full(&self, book_id: &str, item_id: &str, include_html: bool) -> Result<ChapterData> {
        let signer = self.signer_url.as_ref().unwrap();
        let base = std::env::var("FQ_API_BASE").unwrap_or_else(|_| "https://api5-normal-sinfonlineb.fqnovel.com".into());
        let device_id = std::env::var("FQ_DEVICE_ID").unwrap_or_else(|_| "3225306743964489".into());
        let iid = std::env::var("FQ_IID").unwrap_or_else(|_| "3225306743968585".into());
        let params = [
            ("item_id", item_id.to_string()),
            ("book_id", book_id.to_string()),
            ("key_register_ts", "0".into()),
            ("iid", iid),
            ("device_id", device_id.clone()),
            ("ac", "wifi".into()),
            ("channel", "vivo_1967_64".into()),
            ("aid", "1967".into()),
            ("app_name", "novelapp".into()),
            ("version_code", "72932".into()),
            ("version_name", "7.2.9.32".into()),
            ("device_platform", "android".into()),
            ("os", "android".into()),
            ("ssmix", "a".into()),
            ("device_type", "23117RK66C".into()),
            ("device_brand", "Redmi".into()),
            ("language", "zh".into()),
            ("os_api", "35".into()),
            ("os_version", "15".into()),
            ("manifest_version_code", "72932".into()),
            ("resolution", "1920*1080".into()),
            ("dpi", "280".into()),
            ("update_version_code", "72932".into()),
            ("host_abi", "arm64-v8a".into()),
            ("cdid", std::env::var("FQ_CDID").unwrap_or_else(|_| "cb827eb9-0fdf-4977-a94f-a8439c9328e3".into())),
        ];
        let qs = params
            .iter()
            .map(|(k, v)| format!("{k}={}", urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        let full_url = format!("{base}/reading/reader/full/v?{qs}");

        // ask signer service: POST {url, headers} -> map
        let sign_body = serde_json::json!({
            "url": full_url,
            "headers": {
                "user-agent": "com.dragon.read/7.2.9.32",
                "accept": "application/json; charset=utf-8,application/x-protobuf",
                "x-xs-from-web": "0"
            }
        });
        let signed: Value = self
            .http
            .post(format!("{}/sign", signer.trim_end_matches('/')))
            .json(&sign_body)
            .send()
            .await
            .context("signer request")?
            .error_for_status()
            .context("signer status")?
            .json()
            .await
            .context("signer json")?;

        let mut req = self
            .http
            .get(&full_url)
            .header("User-Agent", "com.dragon.read/7.2.9.32")
            .header("Accept", "application/json; charset=utf-8,application/x-protobuf")
            .header("X-Xs-From-Web", "0");
        if let Some(obj) = signed.as_object() {
            for (k, v) in obj {
                if let Some(val) = v.as_str() {
                    req = req.header(k, val);
                }
            }
        }
        let resp = req.send().await?.error_for_status()?;
        let body: Value = resp.json().await?;
        // expected shape: { code, data: ItemContent-like }
        let data = body.get("data").cloned().unwrap_or(body.clone());
        // if nested content field encrypted
        if data.get("content").and_then(|x| x.as_str()).is_some() {
            // ensure key
            if let Some(kv) = data.get("key_version").and_then(|x| x.as_i64()) {
                if self.crypto.get_key(kv).is_none() {
                    let _ = self.refresh_register_key(kv).await;
                }
            }
            return self.decode_item_json(&data, book_id, item_id, include_html, "app_full");
        }
        bail!("unexpected full response: {}", body)
    }

    async fn refresh_register_key(&self, required: i64) -> Result<()> {
        // optional: if signer available, call registerkey and parse
        // for now, no-op if we already have key
        if self.crypto.get_key(required).is_some() {
            return Ok(());
        }
        bail!("missing aes key for version {required}; set FQ_AES_KEY / FQ_KEY_FILE or provide signer+registerkey")
    }

    async fn from_adb(&self, book_id: &str, item_id: &str, include_html: bool) -> Result<ChapterData> {
        // open reading deep link to force cache
        let uri = format!("dragon1967://reading?bookId={book_id}&itemId={item_id}");
        let _ = self.adb(&["shell", "am", "start", "-a", "android.intent.action.VIEW", "-d", &uri]).await;
        tokio::time::sleep(Duration::from_secs(3)).await;
        // pull chapter cache file if present under prefix_public_{book_id}
        let remote_dir = format!("/data/data/com.dragon.read/files/0/prefix_public_{book_id}");
        let list = self.adb(&["shell", "ls", &remote_dir]).await.unwrap_or_default();
        if list.trim().is_empty() {
            bail!("no adb chapter cache dir");
        }
        // pull whole dir to temp
        let local = PathBuf::from(format!("/tmp/fq_adb_{book_id}"));
        let _ = std::fs::remove_dir_all(&local);
        std::fs::create_dir_all(&local)?;
        let _ = self
            .adb(&["shell", "cp", "-r", &remote_dir, &format!("/sdcard/fq_pull_{book_id}")])
            .await;
        let _ = self
            .adb(&[
                "pull",
                &format!("/sdcard/fq_pull_{book_id}"),
                local.to_str().unwrap(),
            ])
            .await;
        // scan files for chapter_id json
        let mut found = None;
        let search_root = if local.join(format!("fq_pull_{book_id}")).exists() {
            local.join(format!("fq_pull_{book_id}"))
        } else {
            local.clone()
        };
        for entry in walkdir_files(&search_root) {
            if let Ok(bytes) = std::fs::read(&entry) {
                if let Some(v) = extract_json_object(&bytes) {
                    let cid = v
                        .get("chapter_id")
                        .or_else(|| v.get("item_id"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("");
                    if cid == item_id {
                        // save to cache
                        let _ = std::fs::create_dir_all(&self.cache_dir);
                        let _ = std::fs::write(
                            self.cache_dir.join(format!("{item_id}.json")),
                            serde_json::to_vec_pretty(&v)?,
                        );
                        found = Some(v);
                        break;
                    }
                }
            }
        }
        let v = found.ok_or_else(|| anyhow!("chapter not found in adb cache"))?;
        // ensure key present
        if let Some(kv) = v.get("key_version").and_then(|x| x.as_i64()) {
            if self.crypto.get_key(kv).is_none() {
                // try pull crypt key kv
                let _ = self.pull_crypt_key_from_device().await;
            }
        }
        self.decode_item_json(&v, book_id, item_id, include_html, "adb_cache")
    }

    async fn pull_crypt_key_from_device(&self) -> Result<()> {
        let _ = self
            .adb(&[
                "shell",
                "cp",
                "/data/data/com.dragon.read/files/mmkv/prefix_public_crypt_key_kv_0",
                "/sdcard/crypt_key_kv_0",
            ])
            .await;
        let local = PathBuf::from("/tmp/crypt_key_kv_0");
        let _ = self.adb(&["pull", "/sdcard/crypt_key_kv_0", local.to_str().unwrap()]).await;
        if !local.exists() {
            bail!("crypt key file missing");
        }
        let bytes = std::fs::read(&local)?;
        // extract key_VERSION! HEX32
        let text = String::from_utf8_lossy(&bytes);
        let re = regex::Regex::new(r"key_(\d+)!\s*([0-9A-Fa-f]{32})").unwrap();
        for cap in re.captures_iter(&text) {
            let ver: i64 = cap[1].parse().unwrap_or(0);
            let key = cap[2].to_string();
            if ver > 0 {
                tracing::info!("adb crypt key version={ver}");
                self.crypto.put_key(ver, &key);
            }
        }
        Ok(())
    }

    async fn adb(&self, args: &[&str]) -> Result<String> {
        let mut cmd = Command::new("adb");
        cmd.arg("-s").arg(&self.device_serial);
        for a in args {
            cmd.arg(a);
        }
        let out = cmd.output().await.context("spawn adb")?;
        if !out.status.success() {
            bail!(
                "adb {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

fn walkdir_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn rec(dir: &Path, out: &mut Vec<PathBuf>) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    rec(&p, out);
                } else {
                    out.push(p);
                }
            }
        }
    }
    rec(root, &mut out);
    out
}

fn extract_json_object(bytes: &[u8]) -> Option<Value> {
    let text = String::from_utf8_lossy(bytes);
    let i = text.find('{')?;
    // try raw_decode from first {
    let s = &text[i..];
    // scan for last plausible }
    if let Ok(v) = serde_json::from_str::<Value>(s) {
        return Some(v);
    }
    // progressive shrink from end
    if let Some(j) = s.rfind('}') {
        let candidate = &s[..=j];
        if let Ok(v) = serde_json::from_str::<Value>(candidate) {
            return Some(v);
        }
    }
    // find by decoder
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    for (idx, &b) in bytes.iter().enumerate() {
        if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                if let Ok(v) = serde_json::from_str::<Value>(&s[..=idx]) {
                    return Some(v);
                }
                break;
            }
        }
    }
    None
}
