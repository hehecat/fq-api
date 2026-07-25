use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchBook {
    pub book_id: String,
    pub book_name: String,
    pub author: String,
    #[serde(default)]
    pub abstract_text: String,
    #[serde(default)]
    pub thumb_url: String,
    #[serde(default)]
    pub word_count: Option<i64>,
    #[serde(default)]
    pub read_count: Option<i64>,
    #[serde(default)]
    pub creation_status: Option<i64>,
    #[serde(default)]
    pub first_chapter_id: Option<String>,
    #[serde(default)]
    pub last_chapter_id: Option<String>,
    #[serde(default)]
    pub last_chapter_title: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookDetail {
    pub book_id: String,
    pub book_name: String,
    pub author: String,
    #[serde(default)]
    pub author_id: Option<String>,
    #[serde(default)]
    pub abstract_text: String,
    #[serde(default)]
    pub thumb_url: String,
    #[serde(default)]
    pub word_number: Option<i64>,
    #[serde(default)]
    pub read_count: Option<i64>,
    #[serde(default)]
    pub creation_status: Option<i64>,
    #[serde(default)]
    pub chapter_total: Option<i64>,
    #[serde(default)]
    pub last_chapter_id: Option<String>,
    #[serde(default)]
    pub last_chapter_title: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub complete_category: Option<String>,
    #[serde(default)]
    pub genre: Option<String>,
    #[serde(default)]
    pub volume_name_list: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterItem {
    pub item_id: String,
    pub title: String,
    #[serde(default)]
    pub volume_name: Option<String>,
    #[serde(default)]
    pub need_pay: Option<i64>,
    #[serde(default)]
    pub order: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryData {
    pub book_id: String,
    pub all_item_ids: Vec<String>,
    pub chapters: Vec<ChapterItem>,
    #[serde(default)]
    pub volume_name_list: Vec<String>,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterData {
    pub book_id: String,
    pub book_name: String,
    pub item_id: String,
    pub title: String,
    pub content: String,
    pub content_html: Option<String>,
    #[serde(default)]
    pub next_item_id: Option<String>,
    #[serde(default)]
    pub pre_item_id: Option<String>,
    #[serde(default)]
    pub word_number: Option<i64>,
    pub source: String,
}
