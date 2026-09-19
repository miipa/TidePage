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

/// 超大书库（16 万本）分页返回：total 为满足条件的总数（前端状态栏/虚拟滚动用），
/// rows 为当前 LIMIT/OFFSET 窗口，避免一次性经 IPC 回传全量。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookPage {
    pub total: i64,
    pub rows: Vec<BookRow>,
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
    #[serde(default)]
    pub encoding_warning: Option<String>,
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
pub struct BookSearchHit {
    pub chapter_index: i64,
    pub chapter_title: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HighlightRow {
    pub id: i64,
    pub book_id: i64,
    pub book_title: String,
    pub chapter_index: i64,
    pub chapter_title: String,
    pub pos_start: i64,
    pub pos_end: i64,
    pub text: String,
    pub style: String,
    pub note: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoonProgress {
    pub file: String,
    pub title: String,
    pub chapter: Option<String>,
    pub pct: Option<f64>,
}
