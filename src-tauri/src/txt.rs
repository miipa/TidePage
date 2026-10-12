use crate::models::TocEntry;
use fancy_regex::Regex;

pub struct PlainDoc {
    pub text: String,
    pub ranges: Vec<(usize, usize)>,
    pub toc: Vec<TocEntry>,
}

/// 单章字符数上限：超过即沿段落（`\n`）边界细分为多个虚拟子章节。
/// 目的：封顶任何单章注入 DOM / CSS 多列同步布局的成本——这是打开大 TXT
/// （尤其英文/无「第X章」命名、整本进单章的书）首屏长时间无响应的根治点。
pub const MAX_CHAPTER_CHARS: usize = 50_000;

/// 把 `[start, end)`（字节区间，相对 `text`）按 `MAX_CHAPTER_CHARS` 沿 `\n`（段落）
/// 边界就近切成多个子 range。保证：
///  - 除「单行本身就超限」外，每段字符数 ≤ 上限；
///  - 切点落在 `\n` 之后（段落边界），绝不在 UTF-8 字符中间（兜底切大行时按 char 边界）；
///  - 子段首尾相接、完整覆盖 `[start, end)`，不重叠不丢字。
/// 不超限时返回单元素 `[(start, end)]`，调用方据长度决定是否加 `(k/N)` 后缀。
fn split_range_by_cap(text: &str, start: usize, end: usize) -> Vec<(usize, usize)> {
    if end <= start {
        return vec![(start, end)];
    }
    if text[start..end].chars().count() <= MAX_CHAPTER_CHARS {
        return vec![(start, end)];
    }
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut sub_start = start; // 当前子段起始（字节）
    let mut acc_chars = 0usize; // 当前子段已累计字符数
    let mut cur = start; // 扫描游标（字节）
    while cur < end {
        // 本行结束位置（含其后的 '\n'）：下一个 '\n' 之后，或区间末尾。
        let line_end = match text[cur..end].find('\n') {
            Some(i) => cur + i + 1,
            None => end,
        };
        let line_chars = text[cur..line_end].chars().count();
        // 情形 1：单行本身就超限 —— 先把已累计子段落段，再按字符边界把这行切成多块。
        if line_chars > MAX_CHAPTER_CHARS {
            if acc_chars > 0 {
                out.push((sub_start, cur));
                acc_chars = 0;
            }
            let mut p = cur;
            while p < line_end {
                let mut taken = 0usize;
                let mut q = line_end;
                for (i, ch) in text[p..line_end].char_indices() {
                    if taken >= MAX_CHAPTER_CHARS {
                        q = p + i;
                        break;
                    }
                    taken += 1;
                    // 走到行尾也要把 q 推进到实际末尾（覆盖最后一块）。
                    q = p + i + ch.len_utf8();
                }
                out.push((p, q));
                p = q;
            }
            sub_start = line_end;
            cur = line_end;
            continue;
        }
        // 情形 2：加上本行会超限且当前已有内容 —— 先落段，本行作为新子段起点。
        if acc_chars > 0 && acc_chars + line_chars > MAX_CHAPTER_CHARS {
            out.push((sub_start, cur));
            sub_start = cur;
            acc_chars = 0;
        }
        acc_chars += line_chars;
        cur = line_end;
    }
    if sub_start < end {
        out.push((sub_start, end));
    }
    if out.is_empty() {
        out.push((start, end));
    }
    out
}

/// 产出一个（可能被细分的）章节到 toc/ranges：范围字符数 ≤ 上限时原样产一条；
/// 超限时沿段落边界细分为多条，标题沿用原标题并加 `(k/N)` 序号后缀，chars 各自重算。
/// index 用 `toc.len()`，保证细分后 toc 与 ranges 仍等长、index 连续。
fn push_capped(
    text: &str,
    title: &str,
    s: usize,
    e: usize,
    toc: &mut Vec<TocEntry>,
    ranges: &mut Vec<(usize, usize)>,
) {
    let subs = split_range_by_cap(text, s, e);
    let n = subs.len();
    if n <= 1 {
        toc.push(TocEntry {
            title: title.to_string(),
            index: toc.len(),
            chars: text[s..e].chars().count() as i64,
        });
        ranges.push((s, e));
        return;
    }
    for (k, (ss, se)) in subs.into_iter().enumerate() {
        toc.push(TocEntry {
            title: format!("{} ({}/{})", title, k + 1, n),
            index: toc.len(),
            chars: text[ss..se].chars().count() as i64,
        });
        ranges.push((ss, se));
    }
}

fn line_bounds(text: &str, pos: usize) -> (usize, usize) {
    let start = text[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let end = text[start..]
        .find('\n')
        .map(|i| start + i)
        .unwrap_or(text.len());
    (start, end)
}

pub fn split_plain(text: &str) -> PlainDoc {
    split_custom(text, crate::default_pattern()).unwrap_or_else(|_| PlainDoc {
        text: text.to_string(),
        ranges: vec![(0, text.len())],
        toc: vec![TocEntry {
            title: "全文".into(),
            index: 0,
            chars: text.chars().count() as i64,
        }],
    })
}

pub fn decode_and_split(
    path: &str,
    pattern: Option<&str>,
    forced: Option<&str>,
) -> Result<PlainDoc, String> {
    let data = std::fs::read(path).map_err(|e| format!("读取文件失败: {e}"))?;
    let (text, _enc) = decode_bytes(&data, forced);
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    match pattern {
        Some(p) if !p.trim().is_empty() => split_custom(&text, p),
        _ => Ok(split_plain(&text)),
    }
}

pub fn split_custom(text: &str, pattern: &str) -> Result<PlainDoc, String> {
    let full = if pattern.contains("(?m") {
        pattern.to_string()
    } else {
        format!("(?m){pattern}")
    };
    let re = Regex::new(&full).map_err(|e| format!("正则表达式无效: {e}"))?;
    let mut starts: Vec<(usize, String)> = Vec::new();
    for m in re.find_iter(text) {
        let m = m.map_err(|e| format!("正则执行出错: {e}"))?;
        let (s, _e) = line_bounds(text, m.start());
        if starts.last().map(|(p, _)| *p) == Some(s) {
            continue;
        }
        let title = text[s.._e].trim().to_string();
        if title.is_empty() {
            continue;
        }
        starts.push((s, title.chars().take(60).collect()));
    }

    let mut toc = Vec::new();
    let mut ranges = Vec::new();
    if starts.is_empty() {
        // 无任何章节匹配（英文书 / 非「第X章」命名）：不再退化为单一「全文」整章，
        // 而是按上限把全书切成「全文 (1/N)」…「全文 (N/N)」，封顶单章布局成本。
        push_capped(text, "全文", 0, text.len(), &mut toc, &mut ranges);
        return Ok(PlainDoc {
            text: text.to_string(),
            ranges,
            toc,
        });
    }
    if starts[0].0 > 0 {
        push_capped(text, "前言", 0, starts[0].0, &mut toc, &mut ranges);
    }
    for (i, (start, title)) in starts.iter().enumerate() {
        let end = if i + 1 < starts.len() {
            starts[i + 1].0
        } else {
            text.len()
        };
        // 单章超限（即便匹配到章名也可能极大）同样沿段落边界细分。
        push_capped(text, title, *start, end, &mut toc, &mut ranges);
    }
    Ok(PlainDoc {
        text: text.to_string(),
        ranges,
        toc,
    })
}

pub fn decode_bytes(data: &[u8], forced: Option<&str>) -> (String, &'static str) {
    // 手动兜底:用户在设置「正文字符编码」里显式指定(与 MOBI 共用 `mobi.encoding` 键)时,
    // 直接用对应 Decoder 解码整个字节流并返回,**跳过** BOM 嗅探 / UTF-8 / chardetng 自动链。
    // 有未映射字节也照常返回(不中断),保证手动兜底始终产出文本。参考 mobi::decode_html_bytes。
    if let Some(label) = forced {
        let label = label.trim();
        if !label.is_empty() && !label.eq_ignore_ascii_case("auto") {
            if let Some(enc) = encoding_rs::Encoding::for_label(label.as_bytes()) {
                let (cow, actual, _had_err) = enc.decode(data);
                return (cow.into_owned(), actual.name());
            }
        }
    }
    if data.len() >= 2 && data[0] == 0xFF && data[1] == 0xFE {
        let units: Vec<u16> = data[2..]
            .chunks(2)
            .filter(|c| c.len() == 2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return (String::from_utf16_lossy(&units), "utf-16le");
    }
    if data.len() >= 2 && data[0] == 0xFE && data[1] == 0xFF {
        let units: Vec<u16> = data[2..]
            .chunks(2)
            .filter(|c| c.len() == 2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return (String::from_utf16_lossy(&units), "utf-16be");
    }
    if data.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return (String::from_utf8_lossy(&data[3..]).into_owned(), "utf-8");
    }
    if let Ok(s) = std::str::from_utf8(data) {
        return (s.to_string(), "utf-8");
    }
    // ---- 非 UTF-8 自动链 ----
    // chardetng 对中文旧编码常误判:简体 GBK/GB18030 被猜成 Big5,而 Big5 能"零 U+FFFD"地
    // 把这些字节解成繁体错字 —— 替换符数量无法区分简/繁(两者互相都能零替换符解对方字节)。
    // 故改用「常用汉字密度」择优:
    //   · 解 chardetng 候选 A、GB18030(G)、Big5(B) 三者;
    //   · 若 G/B 的常用字命中率 max ≥ CJK_HIT(明显是中文书)→ 取命中率高者(并列取 GB18030,简体为主);
    //   · 否则(西文/日文/韩文等非中文)→ 退回 U+FFFD 策略:A 替换符低则用 A,否则取三者替换符最少者。
    let mut det = chardetng::EncodingDetector::new();
    det.feed(data, true);
    let enc = det.guess(None, true);
    let (s, _, _) = enc.decode(data);
    let (sg, _, _) = encoding_rs::GB18030.decode(data);
    let (sb, _, _) = encoding_rs::BIG5.decode(data);
    let g = cjk_common_score(&sg);
    let b = cjk_common_score(&sb);
    // 常用汉字密度阈值:达到即判定为中文书,进入简繁择优(西文/日文一般远低于此)。
    const CJK_HIT: f64 = 0.15;
    if g.max(b) >= CJK_HIT {
        if g >= b {
            return (sg.into_owned(), encoding_rs::GB18030.name());
        }
        return (sb.into_owned(), encoding_rs::BIG5.name());
    }
    // 非明显中文:沿用上一轮 U+FFFD 策略。
    const FFFD_OK: f64 = 0.001;
    if fffd_ratio(&s) <= FFFD_OK {
        return (s.into_owned(), enc.name());
    }
    pick_cjk_fallback(data, s, enc.name())
}

/// 常用汉字集合(同时含简体与繁体高频字),用于「常用汉字密度」打分以区分简/繁。
/// 覆盖高频区即可,不求大而全。
const CJK_COMMON: &str = "的一是不了在人有我他这个们中来上大为和国地到以说时要就出会可你对生能而子那得于着下自之年过发后作里用道行所然家种事成方多经去法学如都同现当没动面起看定天分还进好小部其些主样理心她本前开但因只从想实日者意无力它与长把机十民第公已工使情明性知全三又关点正业外将两高间由问很最重并物手应向头文体政美相见被利什二等产或新己制身果加西斯月话合回特代内信表化老给世位次度门任常先海通教儿原东声提立及比员解水名真论处走义各入口认条平系气题活尔更别打女变四神总何电数安少报才结反受目太量再感建务做接必场件计管期市直德资命山金指克许统区保至队形社便空决治展马科司五基眼书非则听白却界达光放强即像难且权思王象完设式色路记南品住告类求据程北边死张该交规万取拉格望觉术领共确传师观清今切院让识候带导争运笑飞风步改收根干造言联持组每济车亲极林服快办议往元英士证近失转夫令准布始怎呢存未远叫台单影具罗字爱击流备兵连调深商算质团集百需价花党华城石级整府离况请技际约这个们来为国时说实样东义点业关声处与长马话体产华经过还应万里车亲极将变总发头听觉观响丰欢這個們來為國時說實樣東義點業關聲處與長馬話體產華經過還應萬裡車親極將變總發頭聽覺觀響豐歡";

/// `s` 中属于常用汉字集合的字符占比(命中数 / 总字符数);空串记 0。
/// 常用字集合用 `OnceLock<HashSet<char>>` 构建一次复用。
fn cjk_common_score(s: &str) -> f64 {
    let total = s.chars().count();
    if total == 0 {
        return 0.0;
    }
    let set = cjk_common_set();
    let hit = s.chars().filter(|c| set.contains(c)).count();
    hit as f64 / total as f64
}

fn cjk_common_set() -> &'static std::collections::HashSet<char> {
    static SET: std::sync::OnceLock<std::collections::HashSet<char>> = std::sync::OnceLock::new();
    SET.get_or_init(|| CJK_COMMON.chars().collect())
}

/// U+FFFD(替换符)占比 = 替换符字符数 / 总字符数;空串记 0。
fn fffd_ratio(s: &str) -> f64 {
    let total = s.chars().count();
    if total == 0 {
        return 0.0;
    }
    s.chars().filter(|&c| c == '\u{FFFD}').count() as f64 / total as f64
}

/// 非中文输入在 chardetng 候选替换符偏高(疑似猜错)时的回退:
/// 用 `GB18030` 与 `BIG5` 各解一次,在 [候选 A, GB18030, Big5] 中选 U+FFFD 最少者;
/// 并列时优先级 GB18030 > Big5 > A。仅用于「非明显中文」分支(明显中文已走常用字密度择优)。
fn pick_cjk_fallback(
    data: &[u8],
    cand: std::borrow::Cow<'_, str>,
    cand_name: &'static str,
) -> (String, &'static str) {
    let ratio_a = fffd_ratio(&cand);
    let (sg, _, _) = encoding_rs::GB18030.decode(data);
    let (sb, _, _) = encoding_rs::BIG5.decode(data);
    let ratio_g = fffd_ratio(&sg);
    let ratio_b = fffd_ratio(&sb);
    // 初始取 GB18030(并列最高优先)。
    let mut best_ratio = ratio_g;
    let mut best: (String, &'static str) = (sg.into_owned(), encoding_rs::GB18030.name());
    if ratio_b < best_ratio {
        best_ratio = ratio_b;
        best = (sb.into_owned(), encoding_rs::BIG5.name());
    }
    if ratio_a < best_ratio {
        best = (cand.into_owned(), cand_name);
    }
    best
}

/// 渲染 TXT 章节（第 1 段若以“第X章”等标题开头，作为 h2 标题显示）
pub fn render_html_with_title(body: &str) -> String {
    let mut html = String::with_capacity(body.len() + 256);
    html.push_str("<div class=\"txt-body\">");
    let mut first = true;
    for para in body.split('\n') {
        let p = para.trim();
        if p.is_empty() {
            continue;
        }
        if first && is_heading(&p) {
            html.push_str("<h2>");
            escape_into(&mut html, p);
            html.push_str("</h2>");
            first = false;
            continue;
        }
        first = false;
        html.push_str("<p>");
        escape_into(&mut html, p);
        html.push_str("</p>");
    }
    html.push_str("</div>");
    html
}

fn is_heading(line: &str) -> bool {
    let t = line.trim().trim_end_matches(':').trim_end_matches('：');
    t.chars().count() <= 40
        && (t.starts_with("第")
            || (t.chars().all(|c| c.is_ascii_alphanumeric() || c.is_whitespace() || c == '.') && t.len() <= 24))
}

fn escape_into(out: &mut String, s: &str) {
    for ch in s.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            _ => out.push(ch),
        }
    }
}

pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一段 `lines` 行、每行约 `per` 个中文字 + 换行的文本（行间以 `\n` 分隔）。
    fn make_lines(lines: usize, per: usize) -> String {
        let line: String = "测".repeat(per);
        let mut s = String::new();
        for i in 0..lines {
            s.push_str(&line);
            if i + 1 < lines {
                s.push('\n');
            }
        }
        s
    }

    /// 校验：ranges 与 toc 等长、index 连续、首尾相接完整覆盖、各段 chars 与 range 一致。
    fn assert_doc_invariants(doc: &PlainDoc) {
        assert_eq!(doc.ranges.len(), doc.toc.len(), "ranges/toc 必须等长");
        assert!(!doc.ranges.is_empty(), "至少一章");
        let mut prev_end = 0usize;
        for (i, ((s, e), t)) in doc.ranges.iter().zip(&doc.toc).enumerate() {
            assert_eq!(t.index, i, "index 必须连续");
            assert_eq!(*s, prev_end, "子段必须首尾相接(无空洞/重叠)");
            assert!(*e >= *s);
            let real = doc.text[*s..*e].chars().count() as i64;
            assert_eq!(real, t.chars, "toc.chars 必须等于 range 实际字符数");
            prev_end = *e;
        }
        assert_eq!(prev_end, doc.text.len(), "最后一段必须覆盖到文本末尾");
    }

    #[test]
    fn no_match_full_text_is_chunked() {
        // 无「第X章」命名的大文本：不应退化为单一「全文」整章，而是分块。
        // 10000 行 × 10 字 ≈ 10 万字，远超 50_000 上限。
        let text = make_lines(10_000, 10);
        assert!(text.chars().count() > MAX_CHAPTER_CHARS);
        let doc = split_custom(&text, crate::default_pattern()).unwrap();
        assert!(doc.toc.len() >= 2, "全文应被切成多块, 实得 {}", doc.toc.len());
        for t in &doc.toc {
            assert!(t.title.starts_with("全文"), "子章标题应以『全文』开头: {}", t.title);
            assert!(
                t.chars as usize <= MAX_CHAPTER_CHARS,
                "每块字符数应 ≤ 上限, 实得 {}",
                t.chars
            );
        }
        assert!(doc.toc[0].title.contains("(1/"), "首块应带 (1/N) 序号");
        assert_doc_invariants(&doc);
    }

    #[test]
    fn oversized_chapter_is_subdivided() {
        // 匹配到「第一章」但单章极大：应沿段落边界细分为多个带 (k/N) 后缀的子章。
        let mut text = String::from("第一章 测试\n");
        text.push_str(&make_lines(10_000, 10));
        let doc = split_custom(&text, crate::default_pattern()).unwrap();
        assert!(doc.toc.len() >= 2, "超长单章应被细分, 实得 {}", doc.toc.len());
        for t in &doc.toc {
            assert!(t.title.starts_with("第一章"), "子章标题应沿用原章名: {}", t.title);
            assert!(
                t.chars as usize <= MAX_CHAPTER_CHARS,
                "每块字符数应 ≤ 上限, 实得 {}",
                t.chars
            );
        }
        assert_doc_invariants(&doc);
    }

    #[test]
    fn small_chapter_not_split() {
        // 小文本（无匹配）：仍是单章「全文」，不加序号后缀。
        let text = make_lines(10, 5);
        assert!(text.chars().count() <= MAX_CHAPTER_CHARS);
        let doc = split_custom(&text, crate::default_pattern()).unwrap();
        assert_eq!(doc.toc.len(), 1);
        assert_eq!(doc.toc[0].title, "全文");
        assert_doc_invariants(&doc);
    }

    #[test]
    fn decode_gb18030_without_bom() {
        // 复刻真实失败:简体 GB18030 正文,前面拼 8 个 0xA9 0xA5(上轮发现这会让 chardetng
        // 误判 Big5)。常用汉字密度择优应判为 GB18030、含简体正文、无乱码。
        let (body, _, _) =
            encoding_rs::GB18030.encode("第一章 测试内容:你好世界,潮汐页。这是一段简体中文正文,用于验证编码检测。");
        let mut data: Vec<u8> = Vec::new();
        for _ in 0..8 {
            data.push(0xA9);
            data.push(0xA5);
        }
        data.extend_from_slice(&body);
        let (s, enc) = decode_bytes(&data, None);
        assert_eq!(enc, encoding_rs::GB18030.name(), "应判为 GB18030, 实判: {enc}");
        assert!(s.contains("第一章"), "应解出『第一章』, 实得: {s}");
        assert!(s.contains("你好世界"), "应解出『你好世界』, 实得: {s}");
        assert!(cjk_common_score(&s) >= 0.15, "简体正文常用字密度应偏高");
    }

    #[test]
    fn decode_big5_not_misdetected() {
        // 繁体 Big5 不应被误伤为 GB18030:自动解码应判 Big5 且正确还原繁体正文。
        let (body, _, _) = encoding_rs::BIG5.encode("這裡是繁體中文測試內容,潮汐頁閱讀器。");
        let (s, enc) = decode_bytes(&body, None);
        assert_eq!(enc, encoding_rs::BIG5.name(), "应判为 Big5, 实判: {enc}");
        assert!(s.contains("繁體中文"), "应解出『繁體中文』, 实得: {s}");
        assert!(!s.contains('\u{FFFD}'), "不应含替换符, 实得: {s}");
    }

    #[test]
    fn forced_encoding_overrides_auto() {
        // 手动兜底:同一份 GBK 字节,Some("gbk") 按 GBK 强解正确;Some("big5") 按 Big5 强解
        // (得到不同的错字,但编码判定为 Big5);Some("auto")/None 走自动(判 GB18030)。
        let (body, _, _) = encoding_rs::GB18030.encode("第一章 你好世界");
        // 强制 gbk
        let (sg, eg) = decode_bytes(&body, Some("gbk"));
        assert_eq!(eg, encoding_rs::GBK.name());
        assert!(sg.contains("第一章") && sg.contains("你好世界"));
        // 强制 big5(按 Big5 解 GBK 字节 → 判定为 Big5)
        let (_sb, eb) = decode_bytes(&body, Some("big5"));
        assert_eq!(eb, encoding_rs::BIG5.name());
        // auto / None 走自动检测
        let (sa, ea) = decode_bytes(&body, Some("auto"));
        assert_eq!(ea, encoding_rs::GB18030.name());
        assert!(sa.contains("第一章"));
        let (sn, en) = decode_bytes(&body, None);
        assert_eq!(en, encoding_rs::GB18030.name());
        assert!(sn.contains("第一章"));
    }

    #[test]
    fn fallback_prefers_gb18030_over_garbage_candidate() {
        // 直接验证非中文分支的回退选择逻辑:当 chardetng 候选替换符偏高(此处用 UTF-8 lossy
        // 解 GB 字节模拟猜错 → 大量 U+FFFD)时,应在 A/GB18030/Big5 中选中 GB18030。
        let (body, _, _) = encoding_rs::GB18030.encode("第一章 你好世界,潮汐页。");
        let garbage = String::from_utf8_lossy(&body); // 含大量 U+FFFD 的错误候选
        assert!(fffd_ratio(&garbage) > 0.001, "构造的错误候选应有明显替换符");
        let (s, name) = pick_cjk_fallback(&body, garbage, "x-user-defined");
        assert!(!s.contains('\u{FFFD}'), "回退结果不应含替换符, 实得: {s}");
        assert!(s.contains("第一章") && s.contains("你好世界"), "应选中 GB18030 正确解码, 实得: {s}");
        assert_eq!(name, encoding_rs::GB18030.name());
    }

    #[test]
    fn huge_single_line_is_hard_split() {
        // 单行就超限(无换行可切):按字符边界兜底切,仍不越上限、不切坏 UTF-8。
        let text = "测".repeat(MAX_CHAPTER_CHARS * 2 + 123);
        let doc = split_custom(&text, crate::default_pattern()).unwrap();
        assert!(doc.toc.len() >= 2);
        for t in &doc.toc {
            assert!(t.chars as usize <= MAX_CHAPTER_CHARS);
        }
        assert_doc_invariants(&doc);
    }

    #[test]
    fn decode_ascii_is_utf8() {
        let (s, enc) = decode_bytes(b"Hello, plain ASCII text.\nLine two.", None);
        assert_eq!(enc, "utf-8");
        assert!(s.contains("Hello"));
        assert!(!s.contains('\u{FFFD}'));
    }

    #[test]
    fn decode_valid_utf8_stays_utf8() {
        // 合法 UTF-8(含中文)应命中严格 UTF-8 分支,不触发 GB18030/Big5 回退。
        let text = "第一章 中文内容:你好世界。";
        let (s, enc) = decode_bytes(text.as_bytes(), None);
        assert_eq!(enc, "utf-8");
        assert_eq!(s, text);
    }

    #[test]
    fn decode_utf8_bom() {
        let mut data = vec![0xEF, 0xBB, 0xBF];
        data.extend_from_slice("带 BOM 的中文".as_bytes());
        let (s, enc) = decode_bytes(&data, None);
        assert_eq!(enc, "utf-8");
        assert_eq!(s, "带 BOM 的中文");
    }

    #[test]
    fn decode_utf16_le_bom() {
        let text = "UTF-16 LE 中文";
        let mut data = vec![0xFF, 0xFE];
        for u in text.encode_utf16() {
            data.extend_from_slice(&u.to_le_bytes());
        }
        let (s, enc) = decode_bytes(&data, None);
        assert_eq!(enc, "utf-16le");
        assert_eq!(s, text);
    }

    #[test]
    fn decode_utf16_be_bom() {
        let text = "UTF-16 BE 中文";
        let mut data = vec![0xFE, 0xFF];
        for u in text.encode_utf16() {
            data.extend_from_slice(&u.to_be_bytes());
        }
        let (s, enc) = decode_bytes(&data, None);
        assert_eq!(enc, "utf-16be");
        assert_eq!(s, text);
    }
}
