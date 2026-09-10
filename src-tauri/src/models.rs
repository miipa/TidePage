use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookRow {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub author: String,
    pub format: String,
    pub size: i64,
    pub mtime: i64,
    pub added_at: i64,
    pub cover_path: Option<String>,
    pub meta_done: i64,
    pub last_read: i64,
    pub total_chars: i64,
    pub total_text_chars: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TocEntry {
    pub title: String,
    pub index: usize,
    pub chars: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterData {
    pub title: String,
    pub html: String,
    pub chars: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRow {
    pub day: String,
    pub started_at: i64,
    pub seconds: i64,
    pub chars: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Progress {
    pub chapter_index: i64,
    pub chapter_ratio: f64,
    pub pct: f64,
    pub updated_at: i64,
    pub device: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitRule {
    pub name: String,
    pub pattern: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenResult {
    pub book: BookRow,
    pub toc: Vec<TocEntry>,
    pub docs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub book_id: i64,
    pub book_title: String,
    pub chapter_index: i64,
    pub chapter_title: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoonProgress {
    pub file: String,
    pub title: String,
    pub chapter: Option<String>,
    pub pct: Option<f64>,
}
