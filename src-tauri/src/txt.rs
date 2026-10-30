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

/// 大文件编码自动检测的采样上限（P7）：52MB GBK 整本跑三候选检测实测约 26.7s，
/// 只对头部 512KB 判定、整本仅解码一次后，冷开降到秒级；误判由整本替换符占比兜底。
pub const ENC_SAMPLE_BYTES: usize = 512 * 1024;
/// 采样判定后整本解码的 U+FFFD 占比上限：超过即视为采样误判，回退整本三候选判定。
const ENC_FULL_FFFD_LIMIT: f64 = 0.01;
/// 候选（非汉字密度）路径的最低采样信息量：采样里非 ASCII 字节占比达到它、却判不出中文时，
/// 说明采样看不出旧编码（例如头部以 ASCII 为主、正文才是 GBK/Big5）；候选多为单字节编码，
/// 解码不会产生 U+FFFD，整本 FFFD 兜底发现不了误判，必须直接回退整本判定。
const ENC_CANDIDATE_MIN_NON_ASCII: f64 = 0.02;
/// 「明显是中文书」的常用汉字密度阈值（整本/采样判定共用）。
const CJK_HIT: f64 = 0.15;
/// chardetng 候选的替换符占比可接受上限（非明显中文时的 U+FFFD 策略）。
const FFFD_OK: f64 = 0.001;

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
    decode_and_split_detect(path, pattern, forced).map(|(doc, _enc)| doc)
}

/// 解码并切分，同时返回**实际使用**的编码：load_txt 用它把自动判定写回 DB 缓存（P7）。
/// 原 decode_and_split 保留为只关心文本的包装（convert.rs / preview_split 不用改）。
pub fn decode_and_split_detect(
    path: &str,
    pattern: Option<&str>,
    forced: Option<&str>,
) -> Result<(PlainDoc, &'static str), String> {
    let data = std::fs::read(path).map_err(|e| format!("读取文件失败: {e}"))?;
    let (text, enc) = decode_bytes(&data, forced);
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let doc = match pattern {
        Some(p) if !p.trim().is_empty() => split_custom(&text, p)?,
        _ => split_plain(&text),
    };
    Ok((doc, enc))
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
        // 章节标记必须位于行首（允许行首空白，兼容缩进/居中的标题）。
        // 旧实现只要行内出现「第X篇/章」就把整行当章节标题：正文里的「仅仅是第一篇…」
        // 「到了第二篇…」会凭空多出目录项（真实样本：万世之名，第9章与第10章之间有 3 条假章节）。
        if !text[s..m.start()].trim().is_empty() {
            continue;
        }
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

/// P0-1：无 BOM UTF-16 的奇偶 NUL 启发式（纯函数，便于单测）。
/// 背景：无 BOM 的 UTF-16LE ASCII 文本字节里含大量 0x00，std::str::from_utf8 会成功，
/// 旧实现直接当 UTF-8 返回带 NUL 的乱码，永远走不到 UTF-16 解码。
/// 做法：只采样前 64KB；按字节位置分奇/偶两组统计 NUL：
/// - LE 文本：偶数字节是低位（ASCII 非 0），奇数字节是高位 0 → odd 侧 NUL 占比高、even 低；
/// - BE 文本相反。
/// 阈值取 0.3（中英混排时中文对不产生 NUL，ASCII 对约占一半，0.6 会漏判；随机二进制很难
/// 在单侧达到 0.3 且另一侧 <=0.05）。候选解码后再校验替换符/控制字符占比，双重防误判。
fn try_decode_utf16_without_bom(data: &[u8]) -> Option<(String, &'static str)> {
    const SAMPLE_LIMIT: usize = 64 * 1024;
    const MIN_PAIRS: usize = 8;
    const PAD_RATIO: f64 = 0.30;
    const OTHER_RATIO: f64 = 0.05;
    const MAX_FFFD_RATIO: f64 = 0.01;
    const MAX_CTRL_RATIO: f64 = 0.02;
    if data.len() < MIN_PAIRS * 2 || data.len() % 2 != 0 {
        return None;
    }
    let sample_len = data.len().min(SAMPLE_LIMIT) & !1;
    if sample_len < MIN_PAIRS * 2 {
        return None;
    }
    let pairs = sample_len / 2;
    let mut even_nul = 0usize;
    let mut odd_nul = 0usize;
    for c in data[..sample_len].chunks_exact(2) {
        if c[0] == 0 {
            even_nul += 1;
        }
        if c[1] == 0 {
            odd_nul += 1;
        }
    }
    let even_ratio = even_nul as f64 / pairs as f64;
    let odd_ratio = odd_nul as f64 / pairs as f64;
    let le_candidate = odd_ratio >= PAD_RATIO && even_ratio <= OTHER_RATIO;
    let be_candidate = even_ratio >= PAD_RATIO && odd_ratio <= OTHER_RATIO;
    // 两侧同时成立只可能是长度极短/极端构造：保守放弃，回落 UTF-8/自动链。
    if le_candidate == be_candidate {
        return None;
    }
    let (enc, label) = if le_candidate {
        (encoding_rs::UTF_16LE, "utf-16le")
    } else {
        (encoding_rs::UTF_16BE, "utf-16be")
    };
    let (text, _actual, _had_err) = enc.decode(data);
    let text = text.into_owned();
    if text.is_empty() {
        return None;
    }
    let total = text.chars().count().max(1) as f64;
    let bad = text.chars().filter(|&c| c == '\u{fffd}').count() as f64;
    let ctrl = text
        .chars()
        .filter(|&c| c.is_control() && c != '\n' && c != '\r' && c != '\t')
        .count() as f64;
    if bad / total > MAX_FFFD_RATIO || ctrl / total > MAX_CTRL_RATIO {
        return None;
    }
    Some((text, label))
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
    // P0-1：BOM 优先；无 BOM 时在 UTF-8 判定之前做保守的奇偶 NUL 启发式，
    // 否则 UTF-16LE ASCII 字节会被 from_utf8 判成合法 UTF-8（含 NUL 的乱码）。
    if let Some((text, label)) = try_decode_utf16_without_bom(data) {
        return (text, label);
    }
    if let Ok(s) = std::str::from_utf8(data) {
        return (s.to_string(), "utf-8");
    }
    // ---- 非 UTF-8 自动链 ----
    // P7：只对文件头部采样判定编码、整本仅解码一次；整本替换符占比超阈值再回退整本三候选判定。
    let (text, enc, _fell_back) = decode_auto_with_sample(data, ENC_SAMPLE_BYTES);
    (text, enc)
}

/// 自动检测（采样 + 整本兜底）：先只用头部采样选编码，再用它把整本解码一次。
/// 采样里没有非 ASCII 字节（无从判断旧编码）或整本替换符占比 > 1%（采样误判）时，
/// 回退到整本三候选判定，保证判定正确性不退化。
/// sample_len 只用于单测构造「采样与整本冲突」的输入；生产调用固定 ENC_SAMPLE_BYTES。
fn decode_auto_with_sample(data: &[u8], sample_len: usize) -> (String, &'static str, bool) {
    let sample = &data[..data.len().min(sample_len.max(1))];
    let (enc, from_cjk_script) = detect_by_sample(sample);
    // 候选路径的信息量守卫：采样里有可观的非 ASCII 字节（≥2%）却判不出中文 → 采样看不出旧编码
    // （例如超长 ASCII 头 + 正文才是 GBK/Big5）。候选多为单字节编码、解码不产生 U+FFFD，
    // 整本 FFFD 兜底发现不了这种误判，所以直接回退整本三候选判定。
    if !from_cjk_script && sample_non_ascii_ratio(sample) >= ENC_CANDIDATE_MIN_NON_ASCII {
        let (s, name) = detect_full(data);
        return (s, name, true);
    }
    let (text, actual, _had_err) = enc.decode(data);
    if sample_detection_reliable(sample, fffd_ratio(&text)) {
        return (text.into_owned(), actual.name(), false);
    }
    let (s, name) = detect_full(data);
    (s, name, true)
}

/// 采样判定是否可信：采样里必须出现非 ASCII 字节，且整本解码后的替换符占比不超过 1%。
fn sample_detection_reliable(sample: &[u8], full_fffd: f64) -> bool {
    sample.iter().any(|&b| b >= 0x80) && full_fffd <= ENC_FULL_FFFD_LIMIT
}

/// 采样里非 ASCII 字节的占比（非 ASCII 字节数 / 总字节数）；空样本记 0。
fn sample_non_ascii_ratio(sample: &[u8]) -> f64 {
    if sample.is_empty() {
        return 0.0;
    }
    let hi = sample.iter().filter(|&&b| b >= 0x80).count();
    hi as f64 / sample.len() as f64
}

/// 对一段字节按「常用汉字密度 + U+FFFD」三候选策略选出编码（与整本判定同一套打法）。
/// 返回 (编码对象, 是否来自汉字密度路径)：候选路径的判定来源对调用方有不同可信度要求。
fn detect_by_sample(sample: &[u8]) -> (&'static encoding_rs::Encoding, bool) {
    let mut det = chardetng::EncodingDetector::new();
    det.feed(sample, true);
    let enc = det.guess(None, true);
    let (s, _, _) = enc.decode(sample);
    let (sg, _, _) = encoding_rs::GB18030.decode(sample);
    let (sb, _, _) = encoding_rs::BIG5.decode(sample);
    let g = cjk_common_score(&sg);
    let b = cjk_common_score(&sb);
    if g.max(b) >= CJK_HIT {
        return (
            if g >= b {
                encoding_rs::GB18030
            } else {
                encoding_rs::BIG5
            },
            true,
        );
    }
    // 非明显中文:沿用上一轮 U+FFFD 策略（判定来源 = 候选路径）。
    if fffd_ratio(&s) <= FFFD_OK {
        return (enc, false);
    }
    let ratio_a = fffd_ratio(&s);
    let ratio_g = fffd_ratio(&sg);
    let ratio_b = fffd_ratio(&sb);
    let mut best_ratio = ratio_g;
    let mut best = encoding_rs::GB18030;
    if ratio_b < best_ratio {
        best_ratio = ratio_b;
        best = encoding_rs::BIG5;
    }
    if ratio_a < best_ratio {
        best = enc;
    }
    (best, false)
}

/// 整本三候选判定（原自动链的完整逻辑）：采样不可信时兜底用。
/// chardetng 对中文旧编码常误判:简体 GBK/GB18030 被猜成 Big5,而 Big5 能"零 U+FFFD"地
/// 把这些字节解成繁体错字 —— 替换符数量无法区分简/繁(两者互相都能零替换符解对方字节)。
/// 故改用「常用汉字密度」择优:
///   · 解 chardetng 候选 A、GB18030(G)、Big5(B) 三者;
///   · 若 G/B 的常用字命中率 max ≥ CJK_HIT(明显是中文书)→ 取命中率高者(并列取 GB18030,简体为主);
///   · 否则(西文/日文/韩文等非中文)→ 退回 U+FFFD 策略:A 替换符低则用 A,否则取三者替换符最少者。
fn detect_full(data: &[u8]) -> (String, &'static str) {
    let mut det = chardetng::EncodingDetector::new();
    det.feed(data, true);
    let enc = det.guess(None, true);
    let (s, _, _) = enc.decode(data);
    let (sg, _, _) = encoding_rs::GB18030.decode(data);
    let (sb, _, _) = encoding_rs::BIG5.decode(data);
    let g = cjk_common_score(&sg);
    let b = cjk_common_score(&sb);
    if g.max(b) >= CJK_HIT {
        if g >= b {
            return (sg.into_owned(), encoding_rs::GB18030.name());
        }
        return (sb.into_owned(), encoding_rs::BIG5.name());
    }
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
    fn mid_line_chapter_words_do_not_split() {
        // 真实样本：正文句子里出现「第一篇」「第二篇」时，旧规则会把整行当章节标题，
        // 在第 9 章与第 10 章之间凭空插入目录项。章节标记必须位于行首才能切章。
        let text = "第1章 开端\n　　正文第一段。\n　　仅仅是第一篇，太初秘法就需要打开肉身的密藏之门。\n　　到了第二篇，就是接引星辰力量入体。\n第2章 继续\n　　正文第二段。\n";
        let doc = split_custom(text, crate::default_pattern()).unwrap();
        let titles: Vec<&str> = doc.toc.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, vec!["第1章 开端", "第2章 继续"], "正文中的『第X篇』不应切章: {titles:?}");
        assert_doc_invariants(&doc);
    }

    #[test]
    fn indented_chapter_heading_is_accepted() {
        // 行首只有空白时仍算标题（部分 TXT 用空格/全角空格居中章节名）。
        let text = "　　第1章 缩进标题\n正文。\n　　第2章 另一个标题\n正文。\n";
        let doc = split_custom(text, crate::default_pattern()).unwrap();
        let titles: Vec<&str> = doc.toc.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, vec!["第1章 缩进标题", "第2章 另一个标题"], "缩进标题应保留: {titles:?}");
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

    // ---- P7 采样检测 ----

    #[test]
    fn sample_detection_picks_gb18030_and_big5() {
        let (gb, _, _) = encoding_rs::GB18030.encode("第一章 你好世界,潮汐页。这是一段简体中文正文。");
        let (enc_g, from_cjk_g) = detect_by_sample(&gb);
        assert_eq!(enc_g, encoding_rs::GB18030, "GBK 采样应判 GB18030");
        assert!(from_cjk_g, "中文采样应来自汉字密度路径");
        let (b5, _, _) = encoding_rs::BIG5.encode("這裡是繁體中文測試內容,潮汐頁閱讀器。");
        let (enc_b, from_cjk_b) = detect_by_sample(&b5);
        assert_eq!(enc_b, encoding_rs::BIG5, "Big5 采样应判 Big5");
        assert!(from_cjk_b, "中文采样应来自汉字密度路径");
    }

    #[test]
    fn sample_reliability_requires_non_ascii_and_low_fffd() {
        assert!(sample_detection_reliable(b"\xA4\xA4", 0.0), "含非 ASCII 且无替换符");
        assert!(!sample_detection_reliable(b"abcdef", 0.0), "全 ASCII 采样无从判断旧编码");
        assert!(!sample_detection_reliable(b"\xA4\xA4", 0.02), "整本替换符超 1% 视为误判");
        assert!(sample_detection_reliable(b"\xA4\xA4", 0.01), "恰好 1% 仍可信（<=）");
    }

    #[test]
    fn sample_misjudge_falls_back_to_full_detection() {
        // 采样区（前 200 字节）是干净的 GBK 正文 → 采样判 GB18030；后半段拼 0xFF 垃圾字节，
        // 整本按 GB18030 解出的替换符占比 > 1% → 必须回退整本三候选判定，且最终仍判 GB18030。
        let (body, _, _) = encoding_rs::GB18030
            .encode("第一章 你好世界,潮汐页。这是一段简体中文正文,用于验证编码检测。");
        let mut data: Vec<u8> = Vec::new();
        for _ in 0..10 {
            data.extend_from_slice(&body);
        }
        data.extend(std::iter::repeat(0xFFu8).take(60));
        let (text, name, fell_back) = decode_auto_with_sample(&data, 200);
        assert!(fell_back, "整本替换符超阈值应回退整本判定");
        assert_eq!(name, encoding_rs::GB18030.name(), "回退后仍应判 GB18030");
        assert!(text.contains("你好世界"), "回退后正文应正确, 实得: {text}");
    }

    #[test]
    fn sample_non_ascii_ratio_counts_high_bytes() {
        assert_eq!(sample_non_ascii_ratio(b""), 0.0, "空样本记 0");
        assert_eq!(sample_non_ascii_ratio(b"plain ascii"), 0.0, "全 ASCII 为 0");
        assert_eq!(
            sample_non_ascii_ratio(&[0x80, 0x81, b'A', b'B']),
            0.5,
            "一半高位字节应为 0.5"
        );
    }

    #[test]
    fn candidate_path_low_information_falls_back_to_full() {
        // 前 150 字节纯 ASCII + 随后 GBK 中文正文：采样（前 200 字节）只含少量中文，
        // 非 ASCII 占比 25%（≥2%），汉字密度达不到 0.15 → 走候选路；候选解码不产生 U+FFFD，
        // 必须由信息量守卫直接回退整本判定，最终正确判 GB18030。
        let (body, _, _) = encoding_rs::GB18030.encode("你好世界,潮汐页。这是一段简体中文正文。");
        let mut data: Vec<u8> = vec![b'A'; 150];
        for _ in 0..20 {
            data.extend_from_slice(&body);
        }
        let (text, name, fell_back) = decode_auto_with_sample(&data, 200);
        assert!(fell_back, "候选路信息量不足应回退整本判定");
        assert_eq!(name, encoding_rs::GB18030.name(), "回退后应判 GB18030");
        assert!(text.contains("你好世界"), "回退后正文应正确, 实得: {text}");
    }

    #[test]
    fn candidate_path_stays_trusted_for_near_ascii_sample() {
        // 采样几乎全是 ASCII（10/512 ≈ 1.95% < 2%）：新守卫不得触发，仍按候选路正常返回。
        let mut data: Vec<u8> = vec![0xE9; 10]; // Latin-1 的 é（放在采样区内）
        data.extend(std::iter::repeat(b'A').take(1000));
        let (_text, _name, fell_back) = decode_auto_with_sample(&data, 512);
        assert!(!fell_back, "近全 ASCII 采样不应因信息量守卫回退");
        // 纯 ASCII 输入：结果必须原样（任何单字节候选都能还原 ASCII），不因守卫产生乱码。
        let ascii = b"hello world, plain ascii only";
        let (text, _name, _fell_back) = decode_auto_with_sample(ascii, 512);
        assert_eq!(text.as_bytes(), ascii, "纯 ASCII 输入应原样返回");
    }

    #[test]
    fn sample_chain_keeps_utf8_and_forced_priority() {
        // UTF-8 快路径在采样链之前命中，编码名与文本都不变。
        let (s, enc) = decode_bytes("第一章 中文".as_bytes(), None);
        assert_eq!(enc, "utf-8");
        assert_eq!(s, "第一章 中文");
        // 强制编码优先于自动判定：同一份 GBK 字节被强制按 Big5 解，判定名必须是 Big5。
        let (gb, _, _) = encoding_rs::GB18030.encode("第一章 你好世界");
        let (_t, forced_enc) = decode_bytes(&gb, Some("big5"));
        assert_eq!(forced_enc, encoding_rs::BIG5.name(), "强制编码必须优先于自动链");
    }

    #[test]
    fn auto_detection_on_real_file_path_uses_sample() {
        // 端到端：GBK 字节（非 UTF-8）经 decode_bytes 应走采样链并判 GB18030。
        let (body, _, _) = encoding_rs::GB18030.encode("第一章 你好世界,潮汐页。");
        let (s, enc) = decode_bytes(&body, None);
        assert_eq!(enc, encoding_rs::GB18030.name());
        assert!(s.contains("你好世界"));
    }
    /// P0-1：无 BOM 的 UTF-16LE/BE（中英混排、含 ASCII 标点）应被启发式识别并正确解码。
    #[test]
    fn decode_utf16_no_bom_heuristic() {
        let text = "第一章 Hello, TidePage! 你好世界。";
        let mut le = Vec::new();
        for u in text.encode_utf16() {
            le.extend_from_slice(&u.to_le_bytes());
        }
        let (s, enc) = decode_bytes(&le, None);
        assert_eq!(enc, "utf-16le", "无 BOM LE 应识别");
        assert_eq!(s, text);

        let mut be = Vec::new();
        for u in text.encode_utf16() {
            be.extend_from_slice(&u.to_be_bytes());
        }
        let (s, enc) = decode_bytes(&be, None);
        assert_eq!(enc, "utf-16be", "无 BOM BE 应识别");
        assert_eq!(s, text);
    }

    /// P0-1：纯 ASCII UTF-8、GB18030 中文、少量 NUL 的二进制不得被误判为 UTF-16。
    #[test]
    fn decode_utf16_heuristic_does_not_misfire() {
        let (s, enc) = decode_bytes(b"plain ascii text, no BOM", None);
        assert_eq!(enc, "utf-8");
        assert_eq!(s, "plain ascii text, no BOM");

        let (gb, _, _) = encoding_rs::GB18030.encode("第一章 你好世界，潮汐页。");
        let (s, enc) = decode_bytes(&gb, None);
        assert_eq!(enc, encoding_rs::GB18030.name());
        assert!(s.contains("你好世界"), "{s}");

        // 奇偶两侧都只有零散 NUL（各约 0.1），不足以构成 UTF-16 的成对 padding。
        let mut bin = Vec::new();
        for i in 0..64u8 {
            bin.push(if i % 10 == 0 { 0 } else { i });
            bin.push(if i % 10 == 5 { 0 } else { i.wrapping_add(1) });
        }
        let (info, enc) = decode_bytes(&bin, None);
        assert_ne!(enc, "utf-16le");
        assert_ne!(enc, "utf-16be");
        assert!(!info.starts_with('\u{fffd}') || info.contains('\u{fffd}'));
    }

    /// P0-1：带 BOM 的 UTF-16 行为不变（BOM 优先于启发式）。
    #[test]
    fn decode_utf16_bom_takes_priority() {
        let text = "BOM 优先";
        let mut le = vec![0xFF, 0xFE];
        for u in text.encode_utf16() {
            le.extend_from_slice(&u.to_le_bytes());
        }
        let (s, enc) = decode_bytes(&le, None);
        assert_eq!((s.as_str(), enc), (text, "utf-16le"));
    }

}
