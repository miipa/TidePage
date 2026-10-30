// 脚注/尾注弹窗内容提取：只取目标节点，去掉回链、id、class、隐藏样式与内联样式，
// 避免携带原书脚注区的 display:none / 类名规则，导致弹窗内空白。
import { normalizeChapterHtml } from "./contentNormalize";

function findTarget(root: ParentNode, fragment: string): Element | null {
  const attr = fragment.replace(/["\\]/g, "\\$&");
  try {
    const byId = root.querySelector('[id="' + attr + '"]');
    if (byId) return byId;
    return root.querySelector('[name="' + attr + '"]');
  } catch {
    return null;
  }
}

/** 从已渲染的脚注节点复制可读内容；失败返回 null。 */
export function footnoteHtmlFromElement(el: Element): string | null {
  const clone = el.cloneNode(true) as HTMLElement;
  // 回链会重复跳到正文，弹窗里不显示。
  for (const a of [...clone.querySelectorAll("a")]) {
    const href = a.getAttribute("href") || "";
    const text = (a.textContent || "").trim();
    if (/(?:fnref|ftnref|noteref|note-?ref|refback)/i.test(href) || /^[↩←↑]|返回|back/i.test(text)) {
      a.remove();
    }
  }
  for (const node of [clone, ...clone.querySelectorAll<HTMLElement>("*")]) {
    node.removeAttribute("id");
    node.removeAttribute("class");
    node.removeAttribute("style");
    node.removeAttribute("hidden");
    node.removeAttribute("aria-hidden");
  }
  const text = (clone.textContent || "").replace(/\s+/g, " ").trim();
  if (!text) return null;
  return clone.innerHTML;
}

/** 从另一章的原始 HTML 中按 fragment 提取脚注内容。 */
export function extractFootnoteHtml(html: string, fragment: string): string | null {
  if (!html || !fragment || typeof DOMParser === "undefined") return null;
  try {
    const doc = new DOMParser().parseFromString(normalizeChapterHtml(html), "text/html");
    const target = findTarget(doc.body, fragment);
    return target ? footnoteHtmlFromElement(target) : null;
  } catch {
    return null;
  }
}
