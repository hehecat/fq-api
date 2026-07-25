use aes::Aes128;
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use flate2::read::GzDecoder;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

type Aes128CbcDec = cbc::Decryptor<Aes128>;

/// 固定 REG_KEY（registerkey 响应二次解密用）
#[allow(dead_code)]
pub const REG_KEY_HEX: &str = "ac25c67ddd8f38c1b37a2348828e222e";

#[derive(Default)]
pub struct CryptoStore {
    /// key_version -> 32 hex chars (16 bytes)
    keys: RwLock<std::collections::HashMap<i64, String>>,
    key_file: PathBuf,
}

impl CryptoStore {
    pub fn new() -> Self {
        let key_file = PathBuf::from(
            std::env::var("FQ_KEY_FILE").unwrap_or_else(|_| "data/crypt_keys.json".into()),
        );
        Self {
            keys: RwLock::new(std::collections::HashMap::new()),
            key_file,
        }
    }

    pub fn load_key_from_disk(&self) -> Result<()> {
        // 1) crypt_keys.json: { "208700406": "9A1A..." }
        if self.key_file.exists() {
            let text = std::fs::read_to_string(&self.key_file)?;
            let map: std::collections::HashMap<String, String> = serde_json::from_str(&text)?;
            let mut guard = self.keys.write().unwrap();
            for (k, v) in map {
                if let Ok(ver) = k.parse::<i64>() {
                    guard.insert(ver, v.to_uppercase());
                }
            }
            tracing::info!("loaded {} keys from {}", guard.len(), self.key_file.display());
        }

        // 2) optional plaintext/index.json: { "key_version": ..., "aes_key": "..." }
        let idx = Path::new("data/chapters/plaintext/index.json");
        if idx.exists() {
            let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(idx)?)?;
            if let (Some(ver), Some(key)) = (
                v.get("key_version").and_then(|x| x.as_i64()),
                v.get("aes_key").and_then(|x| x.as_str()),
            ) {
                self.put_key(ver, key);
            }
        }

        // 3) env FQ_AES_KEY / FQ_KEY_VERSION
        if let (Ok(key), Ok(ver_s)) = (std::env::var("FQ_AES_KEY"), std::env::var("FQ_KEY_VERSION")) {
            if let Ok(ver) = ver_s.parse::<i64>() {
                self.put_key(ver, &key);
            }
        }

        Ok(())
    }

    pub fn put_key(&self, version: i64, hex: &str) {
        let hex = hex.trim().to_uppercase();
        if hex.len() == 32 {
            self.keys.write().unwrap().insert(version, hex);
            let _ = self.persist();
        }
    }

    pub fn get_key(&self, version: i64) -> Option<String> {
        self.keys.read().unwrap().get(&version).cloned()
    }

    pub fn any_key(&self) -> Option<(i64, String)> {
        self.keys.read().unwrap().iter().next().map(|(k, v)| (*k, v.clone()))
    }

    fn persist(&self) -> Result<()> {
        if let Some(parent) = self.key_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let map: std::collections::HashMap<String, String> = self
            .keys
            .read()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        std::fs::write(&self.key_file, serde_json::to_string_pretty(&map)?)?;
        Ok(())
    }

    /// registerkey 响应里的 key 字段 -> 真 AES key (32 hex)
    #[allow(dead_code)]
    pub fn decrypt_register_key_to_aes(&self, register_key_b64: &str) -> Result<String> {
        let full = decrypt_register_key(register_key_b64, REG_KEY_HEX)?;
        if full.len() < 32 {
            bail!("register key too short after decrypt");
        }
        Ok(full[..32].to_string())
    }

    pub fn decrypt_chapter_content(&self, content_b64: &str, key_version: i64) -> Result<String> {
        let key_hex = self
            .get_key(key_version)
            .or_else(|| self.any_key().map(|(_, k)| k))
            .ok_or_else(|| anyhow!("no aes key for version {key_version}"))?;
        decrypt_and_decompress(content_b64, &key_hex)
    }
}

fn hex_decode(hex: &str) -> Result<Vec<u8>> {
    let hex = hex.trim();
    if hex.len() % 2 != 0 {
        bail!("invalid hex length");
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).context("hex"))
        .collect()
}

#[allow(dead_code)]
fn decrypt_register_key(register_key_b64: &str, reg_key_hex: &str) -> Result<String> {
    let raw = B64.decode(register_key_b64.trim()).context("b64 register key")?;
    if raw.len() < 17 {
        bail!("register key ciphertext too short");
    }
    let (iv, ct) = raw.split_at(16);
    let key = hex_decode(reg_key_hex)?;
    let dec = Aes128CbcDec::new_from_slices(&key, iv)
        .map_err(|e| anyhow!("cipher init: {e}"))?
        .decrypt_padded_vec_mut::<Pkcs7>(ct)
        .map_err(|e| anyhow!("register decrypt pad: {e}"))?;
    Ok(dec.iter().map(|b| format!("{b:02X}")).collect())
}

/// content = base64(iv[16] || AES-CBC-PKCS7(gzip_html))
pub fn decrypt_and_decompress(content_b64: &str, key_hex: &str) -> Result<String> {
    let raw = B64.decode(content_b64.trim()).context("b64 content")?;
    if raw.len() < 17 {
        bail!("content too short");
    }
    let (iv, ct) = raw.split_at(16);
    let key = hex_decode(key_hex)?;
    if key.len() != 16 {
        bail!("aes key must be 16 bytes");
    }
    let pt = Aes128CbcDec::new_from_slices(&key, iv)
        .map_err(|e| anyhow!("cipher init: {e}"))?
        .decrypt_padded_vec_mut::<Pkcs7>(ct)
        .map_err(|e| anyhow!("content decrypt: {e}"))?;

    // gzip
    if pt.len() >= 2 && pt[0] == 0x1f && pt[1] == 0x8b {
        let mut d = GzDecoder::new(&pt[..]);
        let mut out = String::new();
        d.read_to_string(&mut out).context("gzip")?;
        return Ok(out);
    }
    // fallback raw utf8
    Ok(String::from_utf8_lossy(&pt).into_owned())
}

pub fn html_to_text(html: &str) -> String {
    let mut t = html.to_string();
    // titles
    let re_h1 = regex::Regex::new(r"(?is)<h1[^>]*>(.*?)</h1>").unwrap();
    t = re_h1
        .replace_all(&t, |caps: &regex::Captures| {
            let inner = strip_tags(&caps[1]);
            format!("{inner}\n\n")
        })
        .into_owned();
    let re_p = regex::Regex::new(r"(?i)</p\s*>").unwrap();
    t = re_p.replace_all(&t, "\n").into_owned();
    let re_br = regex::Regex::new(r"(?i)<br\s*/?>").unwrap();
    t = re_br.replace_all(&t, "\n").into_owned();
    t = strip_tags(&t);
    t = t
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&#10;", "\n");
    let re_sp = regex::Regex::new(r"\n{3,}").unwrap();
    re_sp.replace_all(t.trim(), "\n\n").into_owned()
}

fn strip_tags(s: &str) -> String {
    let re = regex::Regex::new(r"<[^>]+>").unwrap();
    re.replace_all(s, "").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decrypt_sample() {
        // uses live sample if present
        let p = Path::new("data/chapters/decoded_json/sample.json");
        if !p.exists() {
            return;
        }
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        let content = v["content"].as_str().unwrap();
        let html = decrypt_and_decompress(content, "9A1AF690605DDC2F556388A3A6B29744").unwrap();
        assert!(html.contains("第1章") || html.contains("暴雨") || html.contains("article"));
    }
}
