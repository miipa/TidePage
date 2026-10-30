/**
 * 章内语义预处理：把各格式书籍不一致的 HTML 结构，归一到元素样式能稳定命中的标记上。
 *
 * 只加 class、不改文字与节点顺序，因此不会改变正文选区/划线/搜索使用的文本偏移。
 * 处理范围：
 * - 第一个标题（h1~h6）→ .tp-el-heading；其余标题 → .tp-el-subhead
 * - 没有标题标签时，依次尝试：与章节名一致的块、开头「第 N 章/节/回」块、标题类名块
 * - 标题之后的第一个可读正文段落 → .tp-el-lead；排除装饰线、广告符号与引用/图表/表格
 * - 清洗内联硬编码字体/颜色/白底/绝对字号，避免盖住阅读器主题
 * - 场景分割线 → .tp-el-hr；单图容器/图注 → .tp-el-figure / .tp-el-caption
 * - 脚注/尾注链接 → .tp-el-noteref，点击时由阅读器弹窗预览
 * - 标题内锚点（EPUB 目录回链）→ .tp-el-heading-link，阅读器按标题文字呈现且点击不跳转
 *
 * 为什么需要：TXT 通常是 .txt-body > h2 + p，EPUB 常见 .epub-doc.calibre > h1 + p，
 * 而元素样式原来只认 h2 和 #chapter-content > p:first-of-type，导致多数书的标题/首段特效落空。
 */
const HEADING_SEL = "h1,h2,h3,h4,h5,h6";
const MARK_HEADING = "tp-el-heading";
const MARK_SUBHEAD = "tp-el-subhead";
const MARK_LEAD = "tp-el-lead";
const MARK_LEAD_QUOTED = "tp-el-lead-quoted";
const MARK_HR = "tp-el-hr";
const MARK_FIGURE = "tp-el-figure";
const MARK_CAPTION = "tp-el-caption";
const MARK_NOTEREF = "tp-el-noteref";
const MARK_HEADING_LINK = "tp-el-heading-link";
const BLOCK_DESC = "p,div,section,article,ul,ol,blockquote,table,figure,h1,h2,h3,h4,h5,h6";
/** 标题类元素：标签标题 + 预处理出的标题块。其内锚点按「标题文字」处理，不当行内链接。 */
const HEADING_HOST_SEL = HEADING_SEL + ",." + MARK_HEADING + ",." + MARK_SUBHEAD;
const TITLE_CLASS_RE = /(?:^|[-_ ])(?:title|heading|chapter[-_]?title|chaptertitle)(?:$|[-_ ])/i;
const CHAPTER_PATTERN = /^第\s*[0-9０-９一二三四五六七八九十百千万零〇两]+\s*[章节回卷篇]/u;
const LEADING_PUNCT = /^[\s"'“”‘’「」『』（）()【】\[\]…·—\-–_=*#~]+/u;

function clearMarks(root: ParentNode): void {
  const all = [MARK_HEADING, MARK_SUBHEAD, MARK_LEAD, MARK_LEAD_QUOTED, MARK_HR, MARK_FIGURE, MARK_CAPTION, MARK_NOTEREF, MARK_HEADING_LINK];
  for (const el of root.querySelectorAll(all.map((c) => "." + c).join(","))) {
    el.classList.remove(...all);
  }
}

function textOf(el: Element): string {
  return (el.textContent || "").replace(/\s+/g, " ").trim();
}

/** 至少含一个字母/数字/汉字级文字；只有分隔符、装饰线的块不能当标题或首段。 */
function hasReadable(text: string): boolean {
  return /[\p{L}\p{N}]/u.test(text);
}

/** 去掉开头引号/括号/装饰符后，第一个字符仍是文字；用来跳过「━━ 广告 ━━」这类段落。 */
function startsReadable(text: string): boolean {
  const head = text.replace(LEADING_PUNCT, "");
  return /^[\p{L}\p{N}]/u.test(head);
}

/** 块内是否还嵌套其它块；嵌套容器不能整个当标题/首段。 */
function hasBlockChild(el: Element): boolean {
  return !!el.querySelector(BLOCK_DESC);
}

function isUsableBlock(el: Element): boolean {
  return !hasBlockChild(el) && !el.closest("blockquote, figure, figcaption, table");
}

/** 拆分内联 style，保留括号/引号里的分号。 */
function splitDeclarations(style: string): string[] {
  const out: string[] = [];
  let depth = 0;
  let quote = "";
  let start = 0;
  for (let i = 0; i < style.length; i++) {
    const c = style[i];
    if (quote) {
      if (c === quote && style[i - 1] !== "\\") quote = "";
      continue;
    }
    if (c === "'" || c === "\"") { quote = c; continue; }
    if (c === "(") depth++;
    else if (c === ")") depth = Math.max(0, depth - 1);
    else if (c === ";" && depth === 0) {
      out.push(style.slice(start, i));
      start = i + 1;
    }
  }
  out.push(style.slice(start));
  return out;
}

const DROP_INLINE_PROPS = new Set(["font-family", "color", "background-color"]);
const ABS_FONT_SIZE = /^[+-]?(?:\d+\.?\d*|\.\d+)(?:px|pt|pc|cm|mm|in)(?:\s*!important)?$/i;
const HAS_URL = /url\(/i;

/** 清洗野生 EPUB 的内联字体/颜色/白底：这些硬编码会盖住阅读器字体、暗色主题和元素样式。 */
function sanitizeInlineStyles(root: ParentNode): void {
  for (const el of root.querySelectorAll<HTMLElement>("[style]")) {
    const raw = el.getAttribute("style");
    if (!raw) continue;
    const kept: string[] = [];
    for (const decl of splitDeclarations(raw)) {
      const idx = decl.indexOf(":");
      if (idx < 0) continue;
      const prop = decl.slice(0, idx).trim().toLowerCase();
      const value = decl.slice(idx + 1).trim();
      if (!prop || !value) continue;
      if (DROP_INLINE_PROPS.has(prop)) continue;
      if (prop === "background" && !HAS_URL.test(value)) continue;
      if (prop === "font-size" && ABS_FONT_SIZE.test(value)) continue;
      kept.push(decl.trim());
    }
    if (kept.length) el.setAttribute("style", kept.join("; "));
    else el.removeAttribute("style");
  }
  for (const el of root.querySelectorAll("[bgcolor]")) el.removeAttribute("bgcolor");
  for (const el of root.querySelectorAll("font")) {
    el.removeAttribute("color");
    el.removeAttribute("face");
    el.removeAttribute("size");
  }
}

const SCENE_BREAK_RE = /^[*＊#＃※✳✱✣✤✥✦✧❖◆◇○●◎□■△▲▽▼—–\-=＝~～·・]+$/u;

/** 场景分割线：仅含重复符号的独立段落，打上 hr 标记后由分隔线元素样式接管。 */
function markSceneBreaks(root: ParentNode): void {
  for (const el of root.querySelectorAll<HTMLElement>("p,div")) {
    if (!isUsableBlock(el)) continue;
    if (el.classList.contains(MARK_HEADING) || el.classList.contains(MARK_LEAD)) continue;
    const compact = textOf(el).replace(/\s+/g, "");
    if (compact.length < 3 || compact.length > 24) continue;
    if (SCENE_BREAK_RE.test(compact)) el.classList.add(MARK_HR);
  }
}

const CAPTION_TEXT_RE = /^(?:图|表|插图|图示|图注|Figure|Fig\.?|Photo)\s*[\d一二三四五六七八九十]/i;
const CAPTION_CLASS_RE = /(?:^|[-_ ])(caption|figcaption|image-caption|img-caption)(?:$|[-_ ])/i;

/** 图片/图注补齐：给单图容器与短图注打标记，分页模式下避免图与图注被拆开。 */
function markFiguresAndCaptions(root: ParentNode): void {
  for (const el of root.querySelectorAll<HTMLElement>("figure")) el.classList.add(MARK_FIGURE);
  for (const img of root.querySelectorAll<HTMLImageElement>("img")) {
    img.classList.add(MARK_FIGURE);
    const parent = img.parentElement;
    if (!parent || parent === root || parent.closest("figure,blockquote,table")) continue;
    if (parent.querySelectorAll("img").length !== 1 || textOf(parent).length > 8) continue;
    parent.classList.add(MARK_FIGURE);
  }
  for (const el of root.querySelectorAll<HTMLElement>("figcaption")) el.classList.add(MARK_CAPTION);
  for (const el of root.querySelectorAll<HTMLElement>("p,div")) {
    if (!isUsableBlock(el) || el.classList.contains(MARK_CAPTION)) continue;
    const text = textOf(el);
    if (!text || text.length > 80) continue;
    const classHit = CAPTION_CLASS_RE.test((el.className || "").toString());
    if (classHit || CAPTION_TEXT_RE.test(text)) el.classList.add(MARK_CAPTION);
  }
}

const NOTE_ID_RE = /(?:^|[-_#])(?:fn|ftn|note|footnote|endnote)(?:\d|[^a-z]|$)/i;
const NOTE_BACK_RE = /(?:fnref|ftnref|noteref|note-?ref|refback)/i;

/** 标记脚注/尾注链接：显式 noteref 属性、数字/符号标记，且目标 id 像注释。 */
function markNoteRefs(root: ParentNode): void {
  for (const a of root.querySelectorAll<HTMLAnchorElement>("a[href]")) {
    const href = a.getAttribute("href") || "";
    if (!href || /^(?:https?:|mailto:|javascript:)/i.test(href)) continue;
    const hashPos = href.indexOf("#");
    if (hashPos < 0) continue;
    const hash = href.slice(hashPos + 1);
    if (!hash || NOTE_BACK_RE.test(hash)) continue;
    const attrs = `${a.className || ""} ${a.getAttribute("epub:type") || ""} ${a.getAttribute("role") || ""}`;
    const explicit = /noteref|footnote|endnote|fnref|note-ref/i.test(attrs);
    const text = textOf(a);
    const numeric =
      /^[\[\(（【]?\s*(?:\d{1,3}|[①-⑳]|[ivxlcdm]{1,5}|[*†‡]+|注\s*\d+)\s*[\]\)）】]?$/i.test(text);
    if (!explicit && !numeric) continue;
    if (!explicit && !NOTE_ID_RE.test(hash)) continue;
    a.classList.add(MARK_NOTEREF);
  }
}

/** 标题内锚点：EPUB 常把标题文字整段包进 <a>（指向目录的回链），还有 <a id="page001"></a> 这类空锚点。
 *  原书作用域化 CSS 的类选择器比阅读器的 a 规则更具体，会把标题文字上成链接色、加下划线；
 *  点击也会触发跳转。这里只加 class（不改文字与节点顺序，正文文本偏移不变），
 *  由阅读器 CSS 还原标题观感、由 onBodyClick 忽略点击。 */
function markHeadingLinks(root: ParentNode): void {
  for (const a of root.querySelectorAll<HTMLAnchorElement>("a")) {
    if (a.classList.contains(MARK_NOTEREF)) continue;
    if (!a.closest(HEADING_HOST_SEL)) continue;
    a.classList.add(MARK_HEADING_LINK);
  }
}

/** 与章节名一致的块（不改文字，只加 class），解决 EPUB 用 p/div 当标题的情况。 */
function findTitleLikeBlock(body: HTMLElement, chapterTitle: string): HTMLElement | null {
  const title = chapterTitle.replace(/\s+/g, " ").trim();
  if (!title) return null;
  const blocks = [...body.querySelectorAll<HTMLElement>("p,div")].slice(0, 400);
  const byText = blocks.find((el) => {
    if (!isUsableBlock(el)) return false;
    const text = textOf(el);
    if (!hasReadable(text) || text.length > Math.max(140, title.length + 60)) return false;
    return text === title || text.startsWith(title) || title.startsWith(text);
  });
  if (byText) return byText;
  return blocks.find((el) => {
    if (!isUsableBlock(el)) return false;
    if (!TITLE_CLASS_RE.test((el.className || "").toString())) return false;
    const text = textOf(el);
    return hasReadable(text) && text.length <= 140;
  }) ?? null;
}

/** 没有标题标签时，正文里常见的「第一章 …」块。 */
function findChapterPatternBlock(body: HTMLElement): HTMLElement | null {
  const blocks = [...body.querySelectorAll<HTMLElement>("p,div")].slice(0, 400);
  return blocks.find((el) => {
    if (!isUsableBlock(el)) return false;
    const text = textOf(el).replace(LEADING_PUNCT, "");
    return text.length > 0 && text.length <= 80 && CHAPTER_PATTERN.test(text);
  }) ?? null;
}

function markHeading(el: HTMLElement | null): void {
  if (!el) return;
  el.classList.add(MARK_HEADING);
}

/** 预处理一章 HTML；浏览器环境外（如测试/SSR）原样返回。 */
export function normalizeChapterHtml(html: string, chapterTitle = ""): string {
  if (!html || !html.trim()) return html;
  if (typeof DOMParser === "undefined") return html;
  let doc: Document;
  try {
    doc = new DOMParser().parseFromString(html, "text/html");
  } catch {
    return html;
  }
  const body = doc.body;
  if (!body) return html;
  clearMarks(body);
  sanitizeInlineStyles(body);

  const headings = [...body.querySelectorAll<HTMLElement>(HEADING_SEL)].filter((el) => isUsableBlock(el));
  let headingEl: HTMLElement | null = headings[0] ?? null;
  if (headings.length) {
    headings.forEach((el, i) => {
      el.classList.add(i === 0 ? MARK_HEADING : MARK_SUBHEAD);
    });
  } else {
    headingEl = findTitleLikeBlock(body, chapterTitle) ?? findChapterPatternBlock(body);
    markHeading(headingEl);
  }

  markSceneBreaks(body);
  markFiguresAndCaptions(body);
  markNoteRefs(body);
  // 必须在标题标记与脚注标记之后：只认标记过的标题块，且不碰脚注上标链接。
  markHeadingLinks(body);

  const afterHeading = (el: Element): boolean => {
    if (!headingEl || el === headingEl) return !headingEl;
    return !!(headingEl.compareDocumentPosition(el) & Node.DOCUMENT_POSITION_FOLLOWING);
  };

  const pCandidates = [...body.querySelectorAll<HTMLParagraphElement>("p")].filter(
    (p) =>
      isUsableBlock(p) &&
      !p.closest("." + MARK_HEADING) &&
      !p.classList.contains(MARK_HEADING) &&
      !p.classList.contains(MARK_HR) &&
      !p.classList.contains(MARK_CAPTION) &&
      !p.closest("." + MARK_FIGURE) &&
      startsReadable(textOf(p)) &&
      textOf(p).length >= 5 &&
      afterHeading(p),
  );
  const leadEl: HTMLElement | null = pCandidates[0] ?? null;
  if (leadEl) {
    leadEl.classList.add(MARK_LEAD);
  } else {
    const divCandidate = [...body.querySelectorAll<HTMLElement>("div")].find(
      (div) =>
        !hasBlockChild(div) &&
        !div.closest("blockquote, figure, figcaption, table, ." + MARK_HEADING) &&
        !div.classList.contains(MARK_HEADING) &&
        !div.classList.contains(MARK_HR) &&
        !div.classList.contains(MARK_CAPTION) &&
        !div.closest("." + MARK_FIGURE) &&
        startsReadable(textOf(div)) &&
        textOf(div).length >= 5 &&
        afterHeading(div),
    );
    if (divCandidate) divCandidate.classList.add(MARK_LEAD);
  }

  const finalLead = body.querySelector<HTMLElement>("." + MARK_LEAD);
  if (finalLead) {
    const head = textOf(finalLead);
    if (/^[\s"'“”‘’「」『』（(【\[]/u.test(head)) finalLead.classList.add(MARK_LEAD_QUOTED);
  }

  return body.innerHTML;
}
