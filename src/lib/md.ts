export function esc(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function inlineMd(s: string): string {
  return s
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/(^|[^*])\*([^*\n]+)\*/g, "$1<em>$2</em>")
    .replace(/\[([^\]]+)\]\((https?:[^)\s]+)\)/g, "$1");
}

/** 轻量 markdown → HTML（与 AI 面板一致的最小实现）。 */
export function mdToHtml(src: string): string {
  const lines = (src || "").replace(/\r\n/g, "\n").split("\n");
  const out: string[] = [];
  let para: string[] = [];
  let list: string[] | null = null;
  let code: string[] | null = null;
  const flushPara = () => {
    if (para.length) {
      out.push(`<p>${inlineMd(para.join("<br/>"))}</p>`);
      para = [];
    }
  };
  const flushList = () => {
    if (list) {
      out.push(`<ul>${list.map((li) => `<li>${inlineMd(li)}</li>`).join("")}</ul>`);
      list = null;
    }
  };
  for (const raw of lines) {
    const line = raw.trimEnd();
    if (line.trim().startsWith("```")) {
      if (code) {
        out.push(`<pre><code>${esc(code.join("\n"))}</code></pre>`);
        code = null;
      } else {
        flushPara();
        flushList();
        code = [];
      }
      continue;
    }
    if (code) {
      code.push(raw);
      continue;
    }
    const hm = line.match(/^(#{1,4})\s+(.*)/);
    if (hm) {
      flushPara();
      flushList();
      out.push(`<h${hm[1].length}>${inlineMd(esc(hm[2]))}</h${hm[1].length}>`);
      continue;
    }
    if (/^\s*([-*•])\s+/.test(line)) {
      flushPara();
      if (!list) list = [];
      list.push(esc(line.replace(/^\s*([-*•])\s+/, "")));
      continue;
    }
    const num = line.match(/^\s*(\d+[.、)])\s+(.*)/);
    if (num) {
      flushPara();
      flushList();
      out.push(`<p class="li">${esc(num[1] + " " + num[2])}</p>`);
      continue;
    }
    if (line.trim() === "") {
      flushPara();
      flushList();
      continue;
    }
    if (/^\s*>/.test(line)) {
      flushPara();
      flushList();
      out.push(`<blockquote>${inlineMd(esc(line.replace(/^\s*>\s?/, "")))}</blockquote>`);
      continue;
    }
    para.push(esc(line));
  }
  if (code) out.push(`<pre><code>${esc(code.join("\n"))}</code></pre>`);
  flushPara();
  flushList();
  return out.join("\n");
}
