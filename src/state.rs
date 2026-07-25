use crate::crypto::CryptoStore;
use crate::fontmap::FontMapStore;
use crate::services::{ChapterService, WebClient};
use anyhow::Result;
use std::sync::Arc;

pub struct AppState {
    pub web: WebClient,
    pub chapter: ChapterService,
    pub crypto: Arc<CryptoStore>,
    pub fonts: Arc<FontMapStore>,
}

impl AppState {
    pub fn new() -> Result<Self> {
        let crypto = Arc::new(CryptoStore::new());
        let fonts = Arc::new(FontMapStore::new());
        let web = WebClient::with_fonts(fonts.clone())?;
        let chapter = ChapterService::new(web.clone(), crypto.clone())?;
        Ok(Self {
            web,
            chapter,
            crypto,
            fonts,
        })
    }
}
