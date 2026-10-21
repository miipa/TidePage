use regex::Regex;
use serde::Serialize;

#[derive(Serialize)]
pub struct SuggestMeta {
    pub title: String,
    pub author: String,
}

fn clean_base(name: &str) -> String {
    let mut s = name.trim().to_string();
    s = s.rsplit_once('.').map(|(a, _)| a.to_string()).unwrap_or(s);
    let re_tag = Regex::new(r"^\[[^\]]{0,40}\]").unwrap();
    loop {
        let t = re_tag.replace(&s, "").trim_start().to_string();
        if t == s {
            break;
        }
        s = t;
    }
    s
}

pub fn from_filename(file_name: &str) -> SuggestMeta {
    let base = clean_base(file_name);

    let re_book_author = Regex::new(r"^《(.+?)》\s*(?:作者|著)?[:：\s]*\s*(.+?)$").unwrap();
    if let Some(c) = re_book_author.captures(&base) {
        let author = clean_author(c[2].trim());
        return SuggestMeta { title: c[1].trim().to_string(), author };
    }

    let re_paren = Regex::new(r"^(.+?)[（(]\s*(.+?)\s*[）)]\s*(?:\(?.*?\)?$)?").unwrap();
    if let Some(c) = re_paren.captures(&base) {
        let inner = c[2].trim();
        if inner.chars().count() <= 20 && looks_like_author(inner) {
            return SuggestMeta { title: c[1].trim().to_string(), author: clean_author(inner) };
        }
    }

    let re_author_sep = Regex::new(r"^(.+?)[\s\-—_]+(?:作者|著)\s*[:：]?\s*(.+?)$").unwrap();
    if let Some(c) = re_author_sep.captures(&base) {
        return SuggestMeta {
            title: c[1].trim().to_string(),
            author: clean_author(c[2].trim()),
        };
    }

    let re_zhi_tail = Regex::new(r"^(.+?)\s+([^\s]+?)\s*著\s*(\(?\d+(?:\.\d+)?\)?.*|V\d.*)?$").unwrap();
    if let Some(c) = re_zhi_tail.captures(&base) {
        let cand = c[2].trim();
        if cand.chars().count() <= 12 && cand != c[1].trim() {
            return SuggestMeta { title: c[1].trim().to_string(), author: cand.to_string() };
        }
    }

    SuggestMeta { title: base, author: String::new() }
}

fn looks_like_author(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() || t.chars().count() > 16 {
        return false;
    }
    if t.contains("完本") || t.contains("精校") || t.contains("校对") || t.contains("系列") || t.contains("全集") || t.contains("套装") {
        return false;
    }
    let has_digit = t.chars().any(|c| c.is_ascii_digit());
    let is_cjk = t.chars().all(|c| ('\u{4e00}'..='\u{9fff}').contains(&c) || c.is_alphabetic());
    is_cjk && !has_digit
}

fn clean_author(s: &str) -> String {
    let mut t = s.trim().to_string();
    for pat in ["作者:", "作者：", "著"] {
        t = t.replace(pat, "");
    }
    let re_paren = Regex::new(r"[（(][^）)]*[）)]").unwrap();
    t = re_paren.replace_all(&t, "").trim().to_string();
    t
}

pub fn from_text_start(text: &str) -> Option<String> {
    let head: String = text.chars().take(1200).collect();
    let re = Regex::new(r"(?m)(?:作者|著者|作 者)\s*[:：]\s*([^\s，,。;；:：\r\n]{1,20})").unwrap();
    if let Some(c) = re.captures(&head) {
        let a = c[1].trim().to_string();
        if !a.is_empty() && a != "暂无" {
            return Some(a);
        }
    }
    let re2 = Regex::new(r"(?m)^\s*(《.+?》)\s*[，,、]\s*([^\s，。]{1,16})\s*[著]").unwrap();
    if let Some(c) = re2.captures(&head) {
        return Some(c[2].trim().to_string());
    }
    None
}

pub fn suggest(path: &str, data_head: Option<&str>) -> SuggestMeta {
    let name = std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    // 建议链路改用新的文件名解析（覆盖《书名》/⊙/作者：等常见形态）；
    // from_filename 仍保留给 WebDAV 导入等其它链路，行为不变。
    let (title, author) = parse_meta_from_filename(&name);
    let mut m = SuggestMeta { title, author };
    if m.author.is_empty() {
        if let Some(head) = data_head {
            if let Some(a) = from_text_start(head) {
                m.author = a;
            }
        }
    }
    m
}


// ===== 建议链路的文件名解析（独立于 from_filename：不参与入库/封面，只做展示建议）=====

use std::sync::OnceLock;

static RE_NOISE_BRACKET: OnceLock<Regex> = OnceLock::new();
static RE_NOISE_AT: OnceLock<Regex> = OnceLock::new();
static RE_NOISE_UNDERSCORE: OnceLock<Regex> = OnceLock::new();
static RE_NOISE_NUMERIC: OnceLock<Regex> = OnceLock::new();
static RE_AUTHOR_MARK: OnceLock<Regex> = OnceLock::new();
static RE_AUTHOR_PAREN: OnceLock<Regex> = OnceLock::new();
static RE_TAIL_GROUP: OnceLock<Regex> = OnceLock::new();
static RE_AUTHOR_SEP: OnceLock<Regex> = OnceLock::new();
static RE_NUMERIC_RANGE: OnceLock<Regex> = OnceLock::new();
static RE_TRAILING_RANGE: OnceLock<Regex> = OnceLock::new();
static RE_BOOK_TITLE: OnceLock<Regex> = OnceLock::new();
static RE_WRAPPED: OnceLock<Regex> = OnceLock::new();

fn re_noise_bracket() -> &'static Regex {
    RE_NOISE_BRACKET.get_or_init(|| Regex::new(r"^\[[^\]]{0,40}\]\s*").unwrap())
}
fn re_noise_at() -> &'static Regex {
    RE_NOISE_AT.get_or_init(|| Regex::new(r"^[^《【\[（(]{0,40}?@\s*").unwrap())
}
fn re_noise_underscore() -> &'static Regex {
    RE_NOISE_UNDERSCORE.get_or_init(|| Regex::new(r"^_[^_]{1,60}_+\s*").unwrap())
}
fn re_noise_numeric() -> &'static Regex {
    RE_NOISE_NUMERIC.get_or_init(|| Regex::new(r"^\d{1,4}[.、_\-\s]+").unwrap())
}
fn re_author_mark() -> &'static Regex {
    RE_AUTHOR_MARK.get_or_init(|| Regex::new(r"(?:作\s*者|作\s*家|著\s*者)\s*[：:]\s*(.+)").unwrap())
}
fn re_author_paren() -> &'static Regex {
    RE_AUTHOR_PAREN.get_or_init(|| Regex::new(r"[（(]([^（()）]{1,20}?)\s*著\s*[）)]").unwrap())
}
fn re_tail_group() -> &'static Regex {
    RE_TAIL_GROUP.get_or_init(|| {
        Regex::new(r"[（(【\[〔]([^（()）【】\[\]〔〕]{1,40})[）)】\]〕]\s*$").unwrap()
    })
}
fn re_author_sep() -> &'static Regex {
    RE_AUTHOR_SEP.get_or_init(|| Regex::new(r"^(.+)(?:\s+[-—–]\s+|\s*_\s*)(.+)$").unwrap())
}
fn re_numeric_range() -> &'static Regex {
    RE_NUMERIC_RANGE.get_or_init(|| Regex::new(r"\d+\s*[-~_至]\s*\d+").unwrap())
}
fn re_trailing_range() -> &'static Regex {
    RE_TRAILING_RANGE.get_or_init(|| Regex::new(r"\d+\s*[-~_]\s*\d+\s*$").unwrap())
}
fn re_book_title() -> &'static Regex {
    RE_BOOK_TITLE.get_or_init(|| Regex::new(r"《([^》]+)》").unwrap())
}
fn re_wrapped() -> &'static Regex {
    RE_WRAPPED.get_or_init(|| Regex::new(r"^(?:【(.+)】|\[(.+)\])$").unwrap())
}

/// 作者候选的名字判定：1..=20 字、无数字、无标点/空白、不含标签词、不以「第」开头。
fn name_like(s: &str) -> bool {
    const LABELS: &[&str] = &[
        "作者", "作家", "著者", "全本", "完结", "完", "未完", "校对", "精校", "未删", "加料",
        "番外", "修订", "全集", "连载", "更新", "卷", "章", "字", "万", "部", "求", "无广告",
        "排版", "重置", "完整版", "搜书吧", "sxsy", "soushu", "uaa",
    ];
    const PUNCT: &str = "，,、。;；!！?？：:（）()【】[]《》";
    let t = s.trim();
    let n = t.chars().count();
    if n == 0 || n > 20 {
        return false;
    }
    if t.starts_with('第') {
        return false;
    }
    if t.chars().any(|c| c.is_ascii_digit() || c.is_whitespace()) {
        return false;
    }
    if t.chars().any(|c| PUNCT.contains(c)) {
        return false;
    }
    let lower = t.to_lowercase();
    !LABELS.iter().any(|w| t.contains(w) || lower.contains(w))
}

/// 进度/标签段判定：以 至/到/第/卷/全/完/连载/更新/数字 开头，或含 全本/完结/章/卷/字/万。
fn progress_like(s: &str) -> bool {
    let t = s.trim();
    let Some(first) = t.chars().next() else { return false };
    if first.is_ascii_digit() {
        return true;
    }
    if ["至", "到", "第", "卷", "全", "完", "连载", "更新"]
        .iter()
        .any(|p| t.starts_with(p))
    {
        return true;
    }
    ["全本", "完结", "章", "卷", "字", "万"]
        .iter()
        .any(|w| t.contains(w))
}

/// 去扩展名：只在后缀像扩展名（≤5 个 ASCII 字母数字）时去掉，
/// 避免把 ROOM_NO.1301(1~11卷) 这类书名里的点当扩展名。
fn strip_known_ext(name: &str) -> String {
    if let Some((head, tail)) = name.rsplit_once('.') {
        let ok = !tail.is_empty()
            && tail.chars().count() <= 5
            && tail.chars().all(|c| c.is_ascii_alphanumeric());
        if ok {
            return head.to_string();
        }
    }
    name.to_string()
}

/// 噪声前缀清理：最多 3 轮；某条规则把剩余清空时跳过该条（最终空则退回原 stem）。
fn clean_noise(stem: &str) -> String {
    let mut cur = stem.trim().to_string();
    for _ in 0..3 {
        let before = cur.clone();
        for re in [
            re_noise_bracket(),
            re_noise_at(),
            re_noise_underscore(),
            re_noise_numeric(),
        ] {
            let next = re.replace(&cur, "").trim_start().to_string();
            if !next.is_empty() && next != cur {
                cur = next;
            }
        }
        if cur == before {
            break;
        }
    }
    if cur.trim().is_empty() {
        stem.trim().to_string()
    } else {
        cur
    }
}

/// 尾部括号组是否属于「标签」：空、纯数字、纯 ASCII 短串(≤8)、数字区间、含标签词/站点词。
fn tail_group_is_tag(inner: &str) -> bool {
    let s = inner.trim();
    if s.is_empty() {
        return true;
    }
    if s.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    if s.chars().count() <= 8 && s.chars().all(|c| c.is_ascii_alphanumeric()) {
        return true;
    }
    if re_numeric_range().is_match(s) {
        return true;
    }
    const LABELS: &[&str] = &[
        "作者", "作家", "著者", "全本", "完结", "完", "未完", "校对", "精校", "未删", "加料",
        "番外", "修订", "全集", "连载", "更新", "卷", "章", "字", "万", "部", "求", "无广告",
        "排版", "重置", "完整版", "搜书吧", "sxsy", "soushu", "uaa",
    ];
    let lower = s.to_lowercase();
    LABELS.iter().any(|w| s.contains(w) || lower.contains(w))
}

/// 书名清理：循环剥标签括号 / 残留作者标记 / 数字区间 / ⊙段 / 整体包裹，最后 trim 边界符号。
fn clean_title(mut t: String, base: &str) -> String {
    for _ in 0..8 {
        let before = t.clone();
        // 1) 尾部括号组（标签/数字区间/纯数字/短 ASCII/站点词）
        if let Some(c) = re_tail_group().captures(&t) {
            if let Some(m) = c.get(0) {
                if m.start() > 0 && tail_group_is_tag(c[1].trim()) {
                    t = t[..m.start()].trim_end().to_string();
                }
            }
        }
        // 2) 尾部残留的「作者」「著」
        let trimmed = t.trim_end().to_string();
        if trimmed.ends_with("作者") {
            t = trimmed[..trimmed.len() - "作者".len()].to_string();
        } else if trimmed.ends_with('著') {
            t = trimmed[..trimmed.len() - "著".len()].to_string();
        }
        // 3) 尾部数字区间与 ⊙… 段
        if let Some(m) = re_trailing_range().find(&t) {
            if m.end() == t.trim_end().len() {
                t = t[..m.start()].to_string();
            }
        }
        if let Some(pos) = t.rfind('⊙') {
            t = t[..pos].to_string();
        }
        // 4) 整体被【…】/[…] 包裹 → 取内部
        if let Some(c) = re_wrapped().captures(&t) {
            let inner = c
                .get(1)
                .or_else(|| c.get(2))
                .map(|m| m.as_str().trim())
                .unwrap_or("");
            if !inner.is_empty() {
                t = inner.to_string();
            }
        }
        // 5) trim 边界空白与连接符
        t = t
            .trim_matches(|c: char| c.is_whitespace() || "_-—·.、,，:：".contains(c))
            .to_string();
        if t == before {
            break;
        }
    }
    if t.is_empty() {
        let b = base.trim();
        if b.is_empty() {
            "未命名".to_string()
        } else {
            b.to_string()
        }
    } else {
        t
    }
}

/// 从文件名解析 (书名, 作者)，仅用于「建议」展示（suggest_meta），不参与入库/封面。
/// 不改任何文件、不写库；失败回退原文，不 panic。
pub fn parse_meta_from_filename(file_name: &str) -> (String, String) {
    let stem = strip_known_ext(file_name.trim());
    if stem.trim().is_empty() {
        return ("未命名".to_string(), String::new());
    }
    let base = clean_noise(&stem);

    let mut author = String::new();
    let mut title_left: Option<String> = None;

    // A. 作者：/作家：/著者：
    if let Some(c) = re_author_mark().captures(&base) {
        if let Some(m) = c.get(0) {
            let raw = c[1].trim();
            let cut = raw
                .find(|ch: char| ch.is_whitespace() || "（(【[〔_".contains(ch))
                .unwrap_or(raw.len());
            let mut a = raw[..cut].trim().to_string();
            loop {
                let mut changed = false;
                for w in ["搜书吧", "sxsy.org", "sxsy", "soushu", "uaa", "@sosdbot"] {
                    if a.ends_with(w) {
                        a.truncate(a.len() - w.len());
                        changed = true;
                    }
                }
                while a.ends_with('_') {
                    a.pop();
                    changed = true;
                }
                let trimmed = a.trim().to_string();
                if trimmed != a {
                    a = trimmed;
                    changed = true;
                }
                if !changed {
                    break;
                }
            }
            let n = a.chars().count();
            if (1..=20).contains(&n) {
                author = a;
                title_left = Some(base[..m.start()].to_string());
            }
        }
    }

    // B. ⊙ 分段
    if author.is_empty() {
        let segs: Vec<&str> = base
            .split('⊙')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if segs.len() >= 3 && progress_like(segs[2]) && name_like(segs[1]) {
            author = segs[1].to_string();
            title_left = Some(segs[0].to_string());
        } else if segs.len() == 2 && !progress_like(segs[1]) && name_like(segs[1]) {
            author = segs[1].to_string();
            title_left = Some(segs[0].to_string());
        }
    }

    // C. （XXX 著）
    if author.is_empty() {
        if let Some(c) = re_author_paren().captures(&base) {
            if let Some(m) = c.get(0) {
                let a = c[1].trim();
                if name_like(a) {
                    author = a.to_string();
                    title_left = Some(base[..m.start()].to_string());
                }
            }
        }
    }

    // D. 行尾括号内是严格名字
    if author.is_empty() {
        if let Some(c) = re_tail_group().captures(&base) {
            if let Some(m) = c.get(0) {
                let a = c[1].trim();
                if name_like(a) {
                    author = a.to_string();
                    title_left = Some(base[..m.start()].to_string());
                }
            }
        }
    }

    // E. 「 - 」或「_」分隔
    if author.is_empty() {
        if let Some(c) = re_author_sep().captures(&base) {
            let left = c[1].trim();
            let mut right = c[2].trim().to_string();
            // 先剥尾部括号组
            loop {
                let Some(cg) = re_tail_group().captures(&right) else { break };
                let Some(m) = cg.get(0) else { break };
                if m.start() == 0 {
                    break;
                }
                right = right[..m.start()].trim_end().to_string();
            }
            if !left.is_empty() && name_like(&right) {
                author = right;
                title_left = Some(left.to_string());
            }
        }
    }

    // 书名：优先书名号内容；否则作者左侧；否则 base。
    let title = if let Some(c) = re_book_title().captures(&base) {
        c[1].trim().to_string()
    } else if let Some(l) = title_left {
        l
    } else {
        base.clone()
    };
    (clean_title(title, &base), author)
}

#[cfg(test)]
mod suggest_parse_tests {
    use super::parse_meta_from_filename;

    /// 24 条真实文件名样例：逐字断言（书名, 作者）。
    #[test]
    fn parses_real_world_file_names() {
        let cases: &[(&str, &str, &str)] = &[
            ("《绝世兵王》（校对全本）作者：五十二策", "绝世兵王", "五十二策"),
            ("《全能师尊》（校对版全本）作者：不喝茶的芋头", "全能师尊", "不喝茶的芋头"),
            ("《夜行者：平妖二十年》（校对版全本）作者：南无袈裟理科佛", "夜行者：平妖二十年", "南无袈裟理科佛"),
            ("《琴瑟和鸣》1_18章全_作者：Ya【伪娘乱伦雌堕】", "琴瑟和鸣", "Ya"),
            ("《我一魅魔，你让我参加恋综？》作者：一天两顿真君（1-870）", "我一魅魔，你让我参加恋综？", "一天两顿真君"),
            ("《顾倾城灵异侦探事件簿》（校对版1-7卷全本）作者：顾倾城", "顾倾城灵异侦探事件簿", "顾倾城"),
            ("通天竹修 作者：多脑鱼", "通天竹修", "多脑鱼"),
            ("双穿：我春野樱！打穿海贼！ 作者：风评优良的小违", "双穿：我春野樱！打穿海贼！", "风评优良的小违"),
            ("[sxsy.org]我的综漫水晶宫 作者：星雨七(1)", "我的综漫水晶宫", "星雨七"),
            ("[更116]淫宫美人录（np）作家：棠梨煎雪", "淫宫美人录", "棠梨煎雪"),
            ("uaa_《超级房东》作者：俗人（1-999）", "超级房东", "俗人"),
            ("soushu2025_com@希腊带恶人_作者：紫夜弦歌搜书吧", "希腊带恶人", "紫夜弦歌"),
            ("当修士真是开心呐！⊙再来一瓶老干妈⊙至第737章", "当修士真是开心呐！", "再来一瓶老干妈"),
            ("卧底卧成魔尊夫人⊙Aijerr", "卧底卧成魔尊夫人", "Aijerr"),
            ("【体罚进行时】（完） 作者：努力完结鸭", "体罚进行时", "努力完结鸭"),
            ("喂！婚萝莉是犯法的！就算萝莉是指挥官也不行！（三间萝 著）(3)", "喂！婚萝莉是犯法的！就算萝莉是指挥官也不行！", "三间萝"),
            ("37[四月天VIP完结]《皇子我不爱》", "皇子我不爱", ""),
            ("《贪吃小色女》", "贪吃小色女", ""),
            ("绝境黑夜", "绝境黑夜", ""),
            ("我不是渣女⊙至第347章", "我不是渣女", ""),
            ("进化论 （1-565）", "进化论", ""),
            ("soushu2023.com@调教从嫂子开始[搜书吧]", "调教从嫂子开始", ""),
            ("新建 文本文档", "新建 文本文档", ""),
            ("ROOM_NO.1301(1~11卷)", "ROOM_NO.1301", ""),
        ];
        for (input, title, author) in cases {
            assert_eq!(
                parse_meta_from_filename(input),
                (title.to_string(), author.to_string()),
                "input={input:?}"
            );
        }
    }

    /// 负例：标签括号不能被当成作者。
    #[test]
    fn never_treats_tags_as_author() {
        for bad in ["（1-324）", "（校对版全本）", "（全本）", "（完）"] {
            assert!(!super::name_like(bad), "{bad:?} 不应被当作作者名");
        }
    }
}
