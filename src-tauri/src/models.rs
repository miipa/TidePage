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
    /// 该书自己的封面版本：每次提取/更换封面 +1，令 per-book 封面 URL 缓存失效。
    #[serde(default)]
    pub cover_ver: i64,
    /// 反规范化的阅读进度（0–100）：由 set_progress 写入路径维护（红线 5），供封面进度环渲染与状态过滤。
    #[serde(default)]
    pub read_pct: f64,
    /// 手动状态覆盖：0=自动推导 / 1=手动已读 / 2=手动未读。
    #[serde(default)]
    pub read_state: i64,
    /// 最近打开时间（打开动作写入）：状态推导的「打开」列（open_col），供前端 effectiveState 与 SQL 同源。
    #[serde(default)]
    pub last_opened: i64,
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

/// 全书图片查看器：一张图片的引用（按 spine/章节顺序）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookImage {
    /// 按需加载 URL（http://tidepage.localhost/...）
    pub url: String,
    /// 所属章节下标
    pub chapter_index: i64,
    /// 所属章节标题
    pub chapter_title: String,
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
    /// 全书图片列表（供图片查看器直接使用，避免打开后再发命令）。
    #[serde(default)]
    pub images: Vec<BookImage>,
    /// 是否检测为漫画书籍（图片资源远多于文本；用于漫画图片大小默认值与首次引导）。
    #[serde(default)]
    pub is_manga: bool,
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
    /// 划线颜色（CSS 颜色，空=跟随主题默认）
    pub color: String,
    pub note: String,
    pub created_at: i64,
}
