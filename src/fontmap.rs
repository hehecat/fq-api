//! 番茄 Web 搜索 `x-tt-zhal` 字体混淆还原。
//!
//! 响应头: `x-tt-zhal: k=<family>;f=<font_id>;d1=...;d2=...`
//! 混淆字落在 BMP PUA (U+E000–U+F8FF)，字形来自 awesome-font 子集。
//! 映射表由 tools/build_font_map.py 离线生成，落盘到 FQ_FONT_MAP_DIR。

use anyhow::{anyhow, bail, Context, Result};
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

static DEFAULT_MAP_DIR: Lazy<PathBuf> = Lazy::new(|| {
    std::env::var("FQ_FONT_MAP_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("font_maps"))
});

#[derive(Debug, Clone, Default)]
pub struct ZhalInfo {
    pub family: String,
    pub font_id: String,
    pub d1: String,
    pub d2: String,
}

impl ZhalInfo {
    pub fn parse(header: &str) -> Option<Self> {
        let mut family = String::new();
        let mut font_id = String::new();
        let mut d1 = String::new();
        let mut d2 = String::new();
        for part in header.split(';') {
            let part = part.trim();
            if let Some(v) = part.strip_prefix("k=") {
                family = v.to_string();
            } else if let Some(v) = part.strip_prefix("f=") {
                font_id = v.to_string();
            } else if let Some(v) = part.strip_prefix("d1=") {
                d1 = v.to_string();
            } else if let Some(v) = part.strip_prefix("d2=") {
                d2 = v.to_string();
            }
        }
        if font_id.is_empty() {
            return None;
        }
        Some(Self {
            family,
            font_id,
            d1,
            d2,
        })
    }

    pub fn font_urls(&self) -> Vec<String> {
        let mut urls = Vec::new();
        for d in [&self.d1, &self.d2] {
            if d.is_empty() {
                continue;
            }
            urls.push(format!(
                "https://{d}/obj/awesome-font/c/{}.woff",
                self.font_id
            ));
            urls.push(format!(
                "https://{d}/obj/awesome-font/c/{}.woff2",
                self.font_id
            ));
        }
        if urls.is_empty() {
            urls.push(format!(
                "https://lf6-awef.bytetos.com/obj/awesome-font/c/{}.woff",
                self.font_id
            ));
        }
        urls
    }
}

#[derive(Debug, Deserialize)]
struct MapFile {
    font_id: Option<String>,
    map: HashMap<String, String>,
}

#[derive(Debug, Default)]
pub struct FontMapStore {
    /// font_id -> (pua_codepoint -> char)
    cache: RwLock<HashMap<String, HashMap<u32, char>>>,
    map_dir: PathBuf,
}

impl FontMapStore {
    pub fn new() -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            map_dir: DEFAULT_MAP_DIR.clone(),
        }
    }

    pub fn with_dir(dir: impl Into<PathBuf>) -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            map_dir: dir.into(),
        }
    }

    pub fn map_dir(&self) -> &Path {
        &self.map_dir
    }

    pub fn load_font(&self, font_id: &str) -> Result<HashMap<u32, char>> {
        if let Some(m) = self.cache.read().unwrap().get(font_id).cloned() {
            return Ok(m);
        }
        let path = self.map_dir.join(format!("{font_id}.json"));
        let m = load_map_file(&path).with_context(|| format!("load font map {}", path.display()))?;
        self.cache
            .write()
            .unwrap()
            .insert(font_id.to_string(), m.clone());
        Ok(m)
    }

    pub fn try_load_font(&self, font_id: &str) -> Option<HashMap<u32, char>> {
        self.load_font(font_id).ok()
    }

    pub fn decode_with(&self, font_id: &str, text: &str) -> Result<String> {
        let m = self.load_font(font_id)?;
        Ok(decode_text(text, &m))
    }

    pub fn decode_optional(&self, zhal: Option<&ZhalInfo>, text: &str) -> (String, bool) {
        if !looks_obfuscated(text) {
            return (text.to_string(), false);
        }
        let Some(z) = zhal else {
            return (text.to_string(), false);
        };
        match self.load_font(&z.font_id) {
            Ok(m) => {
                let out = decode_text(text, &m);
                let used = out != text;
                (out, used)
            }
            Err(e) => {
                tracing::warn!("font map {} missing: {e:#}", z.font_id);
                (text.to_string(), false)
            }
        }
    }
}

fn load_map_file(path: &Path) -> Result<HashMap<u32, char>> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mf: MapFile = serde_json::from_str(&raw)?;
    let mut out = HashMap::with_capacity(mf.map.len());
    for (k, v) in mf.map {
        let cp: u32 = if let Some(hex) = k.strip_prefix("0x").or_else(|| k.strip_prefix("0X")) {
            u32::from_str_radix(hex, 16)?
        } else {
            k.parse()
                .map_err(|_| anyhow!("bad pua key in map: {k}"))?
        };
        let ch = v.chars().next().ok_or_else(|| anyhow!("empty char for {k}"))?;
        out.insert(cp, ch);
    }
    if out.is_empty() {
        bail!("empty font map {}", path.display());
    }
    Ok(out)
}

pub fn decode_text(text: &str, map: &HashMap<u32, char>) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let u = c as u32;
        if let Some(&rep) = map.get(&u) {
            out.push(fix_common_confusable(rep));
        } else if is_pua(u) {
            out.push(c);
        } else {
            out.push(fix_common_confusable(c));
        }
    }
    out
}

/// FreeType 偶发把常用字匹配到同形罕见字；表内/表外都兜一层。
fn fix_common_confusable(c: char) -> char {
    match c {
        '＿' | 'ￚ' | 'ー' | '─' | '㆒' | '㇐' => '一',
        '來' => '来',
        '冋' | '囙' => '回',
        '壐' => '重',
        '玍' => '主',
        '図' => '因',
        '圖' => '图',
        '叉' => '又',
        '夬' => '夫',
        '㆔' => '三',
        '𠂤' => '自',
        '都' => '都',
        '囯' | '㘡' => '国',
        _ => c,
    }
}

fn is_pua(u: u32) -> bool {
    (0xE000..=0xF8FF).contains(&u) || (0xF0000..=0xFFFFD).contains(&u)
}

pub fn looks_obfuscated(s: &str) -> bool {
    s.chars().any(|c| {
        let u = c as u32;
        (0xE000..=0xF8FF).contains(&u) || (0xF0000..=0xFFFFD).contains(&u)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_zhal() {
        let z = ZhalInfo::parse(
            "k=DNMrHsV173Pd4pgy;f=c207f68a84deae3;d1=lf6-awef.bytetos.com;d2=lf3-awef.bytetos.com",
        )
        .unwrap();
        assert_eq!(z.font_id, "c207f68a84deae3");
        assert_eq!(z.family, "DNMrHsV173Pd4pgy");
    }

    #[test]
    fn decode_sample() {
        let dir = PathBuf::from("font_maps");
        if !dir.join("c207f68a84deae3.json").exists() {
            return;
        }
        let store = FontMapStore::with_dir(dir);
        let s = store
            .decode_with("c207f68a84deae3", "\u{e4d5}\u{e520}\u{e474}戏\u{e460}")
            .unwrap();
        assert_eq!(s, "我不是戏神");
    }
}
