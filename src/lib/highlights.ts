import type { HighlightRow } from "./types";

/** 收集 root 下的全部文本节点（用于字符偏移 <-> DOM Range 互转）。 */
function textNodes(root: Node): Text[] {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const out: Text[] = [];
  let n = walker.nextNode();
  while (n) {
    out.push(n as Text);
    n = walker.nextNode();
  }
  return out;
}

/** 字符偏移区间 -> DOM Range；越界或空区间返回 null。 */
export function charRangeToRange(root: Node, start: number, end: number): Range | null {
  if (end <= start) return null;
  let acc = 0;
  let startNode: Text | null = null;
  let startOff = 0;
  let endNode: Text | null = null;
  let endOff = 0;
  for (const node of textNodes(root)) {
    const len = node.nodeValue?.length ?? 0;
    const next = acc + len;
    if (!startNode && start < next) {
      startNode = node;
      startOff = start - acc;
    }
    if (startNode && end <= next) {
      endNode = node;
      endOff = end - acc;
      break;
    }
    acc = next;
  }
  if (!startNode || !endNode) return null;
  const r = document.createRange();
  const clamp = (v: number, max: number) => Math.max(0, Math.min(v, max));
  try {
    r.setStart(startNode, clamp(startOff, startNode.nodeValue?.length ?? 0));
    r.setEnd(endNode, clamp(endOff, endNode.nodeValue?.length ?? 0));
  } catch {
    return null;
  }
  return r;
}

/** 当前选区相对 root 的字符偏移；选区不在 root 内或非文本节点时返回 null。 */
export function selectionOffsets(
  root: Node,
  range: Range,
): { start: number; end: number } | null {
  if (
    range.startContainer.nodeType !== Node.TEXT_NODE ||
    range.endContainer.nodeType !== Node.TEXT_NODE
  ) {
    return null;
  }
  if (!root.contains(range.startContainer) || !root.contains(range.endContainer)) return null;
  let acc = 0;
  let start = -1;
  let end = -1;
  for (const node of textNodes(root)) {
    const len = node.nodeValue?.length ?? 0;
    if (node === range.startContainer) start = acc + range.startOffset;
    if (node === range.endContainer) end = acc + range.endOffset;
    acc += len;
  }
  if (start < 0 || end < 0) return null;
  return { start: Math.min(start, end), end: Math.max(start, end) };
}

/** 在 root 内按纯文本查找 needle，返回覆盖首个命中位置的 Range。 */
export function findTextRange(root: Node, needle: string): Range | null {
  const text = textNodes(root).map((n) => n.nodeValue ?? "").join("");
  let q = needle.trim();
  if (!q) return null;
  let at = text.indexOf(q);
  if (at < 0 && q.length > 16) {
    q = q.slice(0, 16);
    at = text.indexOf(q);
  }
  if (at < 0) {
    // 放宽：折叠空白后查找，再映射回原始偏移
    let flat = "";
    const map: number[] = [];
    for (let i = 0; i < text.length; i++) {
      const ch = text[i];
      if (/\s/.test(ch)) {
        if (flat.length === 0 || flat[flat.length - 1] === " ") continue;
        flat += " ";
      } else {
        flat += ch;
      }
      map.push(i);
    }
    const fq = q.replace(/\s+/g, " ");
    const fi = flat.indexOf(fq);
    if (fi < 0) return null;
    const rawStart = map[fi] ?? 0;
    const rawEnd = map[Math.min(fi + fq.length - 1, map.length - 1)] ?? rawStart;
    return charRangeToRange(root, rawStart, rawEnd + 1);
  }
  return charRangeToRange(root, at, at + q.length);
}

/** range 覆盖到的每个文本节点，以及它在本节点内的相交区间（半开 [from,to)）。 */
function intersectingTexts(range: Range): { node: Text; from: number; to: number }[] {
  const container = range.commonAncestorContainer;
  const rootEl =
    container.nodeType === Node.ELEMENT_NODE ? (container as Element) : container.parentElement;
  if (!rootEl) return [];
  const out: { node: Text; from: number; to: number }[] = [];
  for (const node of textNodes(rootEl)) {
    if (!range.intersectsNode(node)) continue;
    const len = node.nodeValue?.length ?? 0;
    let from = node === range.startContainer ? range.startOffset : 0;
    let to = node === range.endContainer ? range.endOffset : len;
    from = Math.max(0, Math.min(from, len));
    to = Math.max(0, Math.min(to, len));
    if (to > from) out.push({ node, from, to });
  }
  return out;
}

/** **就地**把 range 包进一个或多个 <mark>，只切分文本节点、不搬动任何元素。
 *
 *  为什么不是 range.extractContents() + insertNode(mark)：PDF 文本层里每个 span 都靠自身的
 *  position:absolute + left/top 定位，跨 span 选区用 extractContents 会把整段 span 搬进 mark，
 *  它们随即退回普通流、堆到文本层原点 —— 划线跑到页面左上角就是这个原因。
 *  跨多个文本节点会生成多个 <mark>（共享同一个 data-hlid），每个都带齐样式类与 --hl-color。 */
function wrapRange(range: Range, makeMark: () => HTMLElement): void {
  for (const { node, from, to } of intersectingTexts(range)) {
    try {
      const target = from > 0 ? node.splitText(from) : node;
      const len = to - from;
      if ((target.nodeValue?.length ?? 0) > len) target.splitText(len);
      const mark = makeMark();
      target.parentNode?.insertBefore(mark, target);
      mark.appendChild(target);
    } catch {
      /* 单个文本节点切分失败：跳过它，不影响其余片段 */
    }
  }
}

function unwrapMarks(root: ParentNode): void {
  root.querySelectorAll("mark.tp-hl").forEach((m) => {
    const parent = m.parentNode;
    if (!parent) return;
    while (m.firstChild) parent.insertBefore(m.firstChild, m);
    parent.removeChild(m);
  });
}

/** 清除朗读高亮标记。 */
export function clearTtsMark(root: ParentNode): void {
  root.querySelectorAll("mark.tp-tts").forEach((m) => {
    const parent = m.parentNode;
    if (!parent) return;
    while (m.firstChild) parent.insertBefore(m.firstChild, m);
    parent.removeChild(m);
  });
}

/** 高亮当前朗读的文本区间（start<0 表示清除）。 */
export function paintTtsMark(root: HTMLElement, start: number, end: number): void {
  clearTtsMark(root);
  root.normalize();
  if (start < 0 || end <= start) return;
  const range = charRangeToRange(root, start, end);
  if (!range) return;
  wrapRange(range, () => {
    const mark = document.createElement("mark");
    mark.className = "tp-tts";
    return mark;
  });
}

/** 把当前章节的划线绘制到 root（先清除旧标记再重绘）。 */
export function paintHighlights(root: HTMLElement, highlights: HighlightRow[]): void {
  unwrapMarks(root);
  root.normalize();
  if (!highlights.length) return;
  const sorted = [...highlights]
    .filter((h) => h.pos_end > h.pos_start)
    .sort((a, b) => a.pos_start - b.pos_start);
  for (const h of sorted) {
    const range = charRangeToRange(root, h.pos_start, h.pos_end);
    if (!range) continue;
    wrapRange(range, () => {
      const mark = document.createElement("mark");
      mark.className = `tp-hl tp-hl-${h.style || "underline"}${h.note && h.note.trim() ? " tp-has-note" : ""}`;
      mark.dataset.hlid = String(h.id);
      if (h.color) mark.style.setProperty("--hl-color", h.color);
      return mark;
    });
  }
}
