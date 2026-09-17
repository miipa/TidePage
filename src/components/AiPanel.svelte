<script lang="ts">
  import { tick } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api } from "../lib/api";
  import {
    config,
    saveConfig,
    saveConfigNow,
    toast,
    ui,
    openAiWindow,
    saveAiMode,
    copyText,
    THEMES,
    AI_WIN_LABEL,
    stripNonTextHtml,
    type PromptItem,
  } from "../lib/stores.svelte";

  let { chapterTitle = "", chapterHtml = "", standalone = false }: {
    chapterTitle?: string;
    chapterHtml?: string;
    standalone?: boolean;
  } = $props();

  interface Msg {
    role: "user" | "assistant";
    content: string;
  }

  let msgs: Msg[] = $state([]);
  let input = $state("");
  let streaming = $state(false);
  let listEl: HTMLDivElement | undefined = $state();
  let autoFiredFor = "";
  let sessionSeq = 0;

  /* ---------- 阅读样式（翻页）相关 ---------- */
  let aiClip: HTMLDivElement | undefined = $state();
  let aiInner: HTMLDivElement | undefined = $state();
  let aiPage = $state(0);
  let aiPages = $state(1);
  let aiColW = 0;
  let aiStep = 0;
  let measureTimer: ReturnType<typeof setTimeout> | undefined;

  let profile = $derived(
    config.ai.profiles[Math.min(config.ai.active, config.ai.profiles.length - 1)] ?? config.ai.profiles[0]
  );

  let ctx = $state({ title: "", html: "" });

  /* ---------- 轻量 markdown → HTML ---------- */
  function esc(s: string): string {
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

  function mdToHtml(src: string): string {
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

  /* ---------- 章节上下文与请求 ---------- */
  function onChapterAuto(detail: { title: string; html: string }) {
    ctx = { title: detail.title, html: detail.html };
    tick().then(() => {
      listEl?.scrollTo({ top: listEl.scrollHeight });
      scheduleMeasure();
    });
    if (
      config.ai.enabled &&
      config.ai.autoOnChapter &&
      config.ai.prompts.length &&
      detail.title
    ) {
      const key = `auto:${detail.title}`;
      if (autoFiredFor !== key) {
        autoFiredFor = key;
        firePrompt(config.ai.prompts[0]);
      }
    }
  }

  $effect(() => {
    let done = false;
    const ctrl = listen<{ title: string; html: string }>("flowread-ai-chapter", (e) => {
      if (!done) onChapterAuto(e.payload);
    });
    // Same-window fallback event too (浏览器原生事件不跨窗口，但面板场景保证覆盖)
    const local = (e: Event) => {
      const ce = e as CustomEvent<{ title: string; html: string }>;
      if (!done) onChapterAuto(ce.detail);
    };
    window.addEventListener("flowread-ai-chapter", local);
    return () => {
      done = true;
      window.removeEventListener("flowread-ai-chapter", local);
      ctrl.then((fn) => fn());
    };
  });

  if (standalone) {
    try {
      const raw = localStorage.getItem("fr.ai.ctx");
      if (raw) ctx = JSON.parse(raw);
    } catch { /* ignore */ }
    window.addEventListener("storage", (e) => {
      if (e.key === "fr.ai.ctx" && e.newValue) {
        try {
          ctx = JSON.parse(e.newValue);
          tick().then(() => {
            listEl?.scrollTo({ top: listEl.scrollHeight });
            scheduleMeasure();
          });
        } catch { /* ignore */ }
      }
    });
  } else {
    $effect(() => {
      ctx = { title: chapterTitle, html: chapterHtml };
    });
  }

  function chapterText(html: string, limit = 9000): string {
    const div = document.createElement("div");
    div.innerHTML = stripNonTextHtml(html);
    const text = (div.textContent ?? "").replace(/\s+/g, " ").trim();
    return text.length > limit ? text.slice(0, limit) + "……(后文截断)" : text;
  }

  function push(role: Msg["role"], content: string): Msg {
    msgs.push({ role, content });
    return msgs[msgs.length - 1];
  }

  function errKindLabel(kind: string): string {
    switch (kind) {
      case "network": return "网络错误";
      case "timeout": return "连接超时";
      case "http_4xx": return "鉴权/请求错误(4xx)";
      case "http_5xx": return "服务端错误(5xx)";
      case "protocol": return "响应协议不匹配";
      case "model_id": return "模型ID非法";
      case "config": return "配置不完整";
      default: return "未知错误";
    }
  }

  // 后端 ai_chat 失败时 reject 结构化对象 {kind,message}；此处兼容字符串/Error 兜底分类。
  function normalizeAiError(e: unknown): { kind: string; message: string } {
    if (e && typeof e === "object") {
      const o = e as Record<string, unknown>;
      if ("message" in o || "kind" in o) {
        return {
          kind: typeof o.kind === "string" ? o.kind : "unknown",
          message: typeof o.message === "string" ? o.message : JSON.stringify(e),
        };
      }
    }
    const s = typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
    const kind = /超时|timeout/i.test(s)
      ? "timeout"
      : /网络|connect|dns|refused|unreachable/i.test(s)
        ? "network"
        : /HTTP 5\d\d/.test(s)
          ? "http_5xx"
          : /HTTP 4\d\d/.test(s)
            ? "http_4xx"
            : /协议不匹配/.test(s)
              ? "protocol"
              : /模型ID/.test(s)
                ? "model_id"
                : "unknown";
    return { kind, message: s };
  }

  async function run(system: string, userText: string) {
    if (streaming) {
      toast("AI 正在回复中…");
      return;
    }
    const profiles = config.ai.profiles;
    if (!profiles.length) {
      toast("请先在设置 → AI 配置端点", "error");
      return;
    }

    streaming = true;
    push("user", userText.length > 120 ? userText.slice(0, 120) + "…" : userText);
    const reply = push("assistant", "");
    scheduleMeasure();

    // 失败转移顺序：当前端点优先，其后按 profiles 列表顺序循环；最多尝试 profiles.length 次。
    const start = Math.min(Math.max(config.ai.active, 0), profiles.length - 1);
    const order: number[] = [];
    for (let k = 0; k < profiles.length; k++) order.push((start + k) % profiles.length);
    const maxTries = config.ai.failover ? order.length : 1;
    const failures: { name: string; kind: string; message: string }[] = [];

    for (let a = 0; a < maxTries; a++) {
      const idx = order[a];
      const p = profiles[idx];
      if (!p || !p.baseUrl || !p.model) {
        failures.push({ name: p?.name || `端点${idx + 1}`, kind: "config", message: "未填写地址或模型" });
        continue;
      }
      const label = p.name || `端点${idx + 1}`;
      if (a > 0) {
        const prev = failures[failures.length - 1];
        // 重试前清空上一个端点可能已流式写入的内容，避免拼接错乱
        reply.content = "";
        toast(`端点「${prev.name}」失败（${errKindLabel(prev.kind)}），已切换至「${label}」重试`);
      }
      const id = `s${++sessionSeq}-${Date.now()}`;
      let active = true;
      let un: (() => void) | null = null;
      const stop = listen<{ delta: string; done: boolean }>(`ai-token:${id}`, (e) => {
        if (!active) return;
        if (e.payload.delta) {
          reply.content += e.payload.delta;
          tick().then(() => {
            listEl?.scrollTo({ top: listEl.scrollHeight });
          });
        }
      });
      un = () => stop.then((f) => f());
      try {
        await api.aiChat({
          id,
          provider: p.provider,
          baseUrl: p.baseUrl,
          apiKey: p.apiKey,
          model: p.model,
          deployment: p.deployment,
          apiVersion: p.apiVersion,
          system,
          messages: [["user", userText]],
          proxyMode: p.proxyMode,
          proxyUrl: p.proxyUrl,
        });
        active = false;
        un?.();
        if (config.ai.active !== idx) {
          config.ai.active = idx;
          saveConfigNow();
        }
        streaming = false;
        scheduleMeasure();
        return;
      } catch (e) {
        active = false;
        un?.();
        const err = normalizeAiError(e);
        failures.push({ name: label, kind: err.kind, message: err.message });
        if (!config.ai.failover) break;
      }
    }

    reply.content = failures.length
      ? `[全部端点均失败]\n${failures
          .map((f) => `• 端点「${f.name}」：${errKindLabel(f.kind)}：${f.message}`)
          .join("\n")}`
      : "[错误] 没有可用端点";
    streaming = false;
    scheduleMeasure();
    toast("全部端点均失败，详见回复内容", "error");
  }

  function renderPrompt(p: PromptItem, limit = 9000): string {
    return p.text
      .split("{chapter}")
      .join(chapterText(ctx.html, limit))
      .split("{title}")
      .join(ctx.title);
  }

  function firePrompt(p: PromptItem) {
    if (!ctx.html) {
      toast("等待章节加载…");
      return;
    }
    run(config.ai.system, renderPrompt(p));
  }

  /* ---------- 翻页辅助数据 ---------- */
  let lastAi = -1;
  let followEnd = false; // 用户处于/选择跟随末页：新回复与流式增长时钉住最后一页；手动翻页即解除

  function qShort(m: Msg | undefined): string {
    if (!m) return "";
    return m.content.length > 26 ? m.content.slice(0, 26) + "…" : m.content;
  }

  function onGlobalKeys(e: KeyboardEvent) {
    const tgt = e.target as HTMLElement | null;
    if (tgt && (tgt.tagName === "INPUT" || tgt.tagName === "TEXTAREA" || tgt.isContentEditable)) return;
    if (ui.settingsOpen) return;
    const key = e.key.toLowerCase();
    for (const p of config.ai.prompts) {
      if (p.key && p.key.toLowerCase() === key) {
        e.preventDefault();
        firePrompt(p);
        return;
      }
    }
  }

  function send() {
    const q = input.trim();
    if (!q) return;
    input = "";
    const context = `当前正在阅读《${ctx.title}》，内容节选：\n${chapterText(ctx.html, 5000)}\n\n我的问题：${q}`;
    run(config.ai.system, context);
  }

  function popOut() {
    openAiWindow();
  }

  let panelWidth = $derived(config.reader.aiPanelWidth);

  function startResize(e: PointerEvent) {
    e.preventDefault();
    const startX = e.clientX;
    const startW = panelWidth;
    const move = (ev: PointerEvent) => {
      const w = Math.max(300, Math.min(760, startW + (startX - ev.clientX)));
      config.reader.aiPanelWidth = w;
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      saveConfig();
      scheduleMeasure();
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  /* ---------- 阅读样式：版式变量 ---------- */
  const readerStyle = $derived.by(() => {
    const t = THEMES[config.reader.theme];
    const r = config.reader;
    const s = config.ai.style;
    const latin = r.latinFont || ""; // 西文优先为全局偏好：抽屉/独立窗口/AI 自定义模式统一生效
    const fontOk = s.followReader || !!s.fontFamily;
    const fs = s.followReader ? r.fontSize : (s.fontSize || r.fontSize);
    const lh = s.followReader ? r.lineHeight : (s.lineHeight > 0 ? s.lineHeight : r.lineHeight);
    const ps = s.followReader ? r.paraSpacing : (s.paraSpacing > 0 ? s.paraSpacing : r.paraSpacing);
    return {
      font: fontOk
        ? (s.followReader ? r.fontFamily : s.fontFamily) || r.fontFamily
        : r.fontFamily,
      size: fs,
      lh,
      ps,
      mh: s.followReader ? r.marginH : (s.marginH > 0 ? s.marginH : r.marginH),
      mv: s.followReader ? r.marginV : (s.marginV > 0 ? s.marginV : r.marginV),
      bg: (!s.followReader && s.bgColor) || r.bgColor || t.bg,
      fg: (!s.followReader && s.fgColor) || r.fgColor || t.fg,
      latin,
      indent: s.followReader ? r.indent : (s.indent || r.indent),
      align: r.textAlign,
      extra: s.cssExtra || "",
    };
  });

  function fontStack(raw: string, latinFirst?: string): string {
    const norm = (list: string[]) => list
      .map((x) => x.trim().replace(/^["']|["']$/g, ""))
      .filter(Boolean)
      .map((x) => (/^[A-Za-z][A-Za-z0-9-]*$/.test(x) ? x : `"${x}"`))
      .join(", ");
    const base = norm((raw || "").split(","));
    const latin = norm([latinFirst || ""]);
    if (!latin) return base || '"Microsoft YaHei", "Segoe UI", sans-serif';
    return base ? `${latin}, ${base}` : latin;
  }

  const readerVars = $derived(
    `--ai-font:${fontStack(readerStyle.font, readerStyle.latin)};--ai-fs:${readerStyle.size}px;--ai-lh:${readerStyle.lh};` +
    `--ai-ps:${readerStyle.ps}em;--ai-bg:${readerStyle.bg};--ai-fg:${readerStyle.fg};` +
    `--ai-indent:${readerStyle.indent};--ai-align:${readerStyle.align}`
  );

  const readerCss = $derived(`
.ai-panel.reader .ai-mark, .ai-panel.reader .ai-mark :is(p,h1,h2,h3,h4,ul,ol,li,pre,code,blockquote) {
  color: var(--ai-fg) !important;
  font-family: var(--ai-font) !important;
  font-size: var(--ai-fs) !important;
  line-height: var(--ai-lh) !important;
  letter-spacing: 0.05em !important;
}
.ai-panel.reader .ai-mark { text-align: var(--ai-align, justify) !important; text-justify: inter-ideograph !important; font-synthesis: none !important; }
.ai-panel.reader .ai-mark p { margin: var(--ai-ps) 0 !important; text-indent: var(--ai-indent, 0em) !important; }
.ai-panel.reader .ai-mark p.li { text-indent: -1.4em !important; }
.ai-panel.reader .ai-q-title { font-family: var(--ai-font) !important; }
${readerStyle.extra}
`);

  let aiStyleTag: HTMLStyleElement | null = null;
  $effect(() => {
    if (isReader) {
      if (!aiStyleTag) {
        aiStyleTag = document.createElement("style");
        aiStyleTag.id = "flowread-ai-reader-css";
        document.head.appendChild(aiStyleTag);
      }
      aiStyleTag.textContent = readerCss;
    } else {
      document.getElementById("flowread-ai-reader-css")?.remove();
      aiStyleTag = null;
    }
  });

  const isReader = $derived(config.ai.display === "reader");

  const replyIdxs = $derived(msgs.map((m, i) => [m, i] as const).filter(([m]) => m.role === "assistant"));

  /* ---------- 阅读样式：整屏分栏翻页 ---------- */
  function measureAi() {
    if (!isReader || !aiClip || !aiInner) return;
    // 隐藏/叠层遮挡期间不测量：零宽瞬态会把 aiPages 误缩为 1，
    // 导致恢复布局后 wasAtEnd 误判、无辜把用户从中间页弹到末页
    if (!aiClip.offsetParent) return;
    const r = readerStyle;
    const T = aiClip.parentElement ? aiClip.parentElement.clientWidth : aiClip.clientWidth;
    const H = aiClip.clientHeight;
    if (T < 20 || H < 20) return;
    const mv = Math.max(8, Math.min(r.mv, Math.max(8, H / 2 - 8)));
    const gap = T > 440 ? 36 : 18;
    const maxCol = Math.max(240, Math.min(T, T - r.mh * 2));
    const side = Math.max(0, Math.round((T - maxCol) / 2));
    // 可视框只保留栏盒宽度：下一页的栏完全不可见（与主阅读器一致，杜绝“露半栏”）
    aiClip.style.marginLeft = `${side}px`;
    aiClip.style.marginRight = `${side}px`;
    const total = 1; // 单栏：AI 面板拉宽也只显示一栏，多出的空间给正文
    const colW = Math.floor((maxCol - (total - 1) * gap) / total);
    aiColW = total * colW + (total - 1) * gap;
    aiStep = maxCol + gap;
    const st = aiInner.style;
    st.top = `${mv}px`;
    st.bottom = `${mv}px`;
    st.width = `${maxCol}px`;
    st.columnWidth = `${colW}px`;
    st.columnGap = `${gap}px`;
    const cols = Math.max(1, Math.round((aiInner.scrollWidth + gap) / (colW + gap)));
    const newPages = Math.max(1, Math.ceil(cols / total));
    // 末页跟随只看 followEnd（新回复时置位、手动交互清零），
    // 不依赖"当前是否恰在末页"——布局瞬态（弹层遮挡/回流）不会误判跳页
    aiPages = newPages;
    aiPage = followEnd ? aiPages - 1 : Math.min(aiPage, aiPages - 1);
    const starts: number[] = [];
    for (const pg of aiInner.querySelectorAll<HTMLElement>(".ai-reply")) {
      const col = Math.round(pg.offsetLeft / (colW + gap));
      starts.push(Math.min(aiPages - 1, Math.max(0, Math.floor(col / total))));
    }
    pageStarts = starts;
    applyAiPage();
  }

  function applyAiPage() {
    if (!aiInner || !aiClip) return;
    aiInner.style.left = `${Math.round((aiClip.clientWidth - aiColW) / 2) - aiPage * aiStep}px`;
  }

  function scheduleMeasure(forceEnd = false) {
    if (!isReader) return;
    if (forceEnd) lastAi = -1;
    clearTimeout(measureTimer);
    measureTimer = setTimeout(() => {
      const cnt = isReader ? (aiInner?.querySelectorAll(".ai-reply").length ?? 0) : 0;
      measureAi();
      if (cnt !== lastAi) {
        lastAi = cnt;
        aiPage = aiPages - 1;
        applyAiPage();
      }
    }, 120);
  }

  function aiFlip(dir: number) {
    followEnd = false;
    const np = Math.max(0, Math.min(aiPage + dir, aiPages - 1));
    aiPage = np;
    applyAiPage();
  }

  function aiTurnBind(id: "next" | "prev"): boolean {
    const b = config.reader.bindings[id];
    return !!b && b.length > 0;
  }

  function onReaderKeys(e: KeyboardEvent) {
    if (!isReader) return;
    const tgt = e.target as HTMLElement | null;
    if (tgt && (tgt.tagName === "INPUT" || tgt.tagName === "TEXTAREA" || tgt.isContentEditable)) return;
    if (ui.settingsOpen) return;
    const k = e.key;
    const nextKeys = ["ArrowRight", "PageDown", "ArrowDown", ...(aiTurnBind("next") ? config.reader.bindings.next : [])];
    const prevKeys = ["ArrowLeft", "PageUp", "ArrowUp", ...(aiTurnBind("prev") ? config.reader.bindings.prev : [])];
    if (nextKeys.includes(k)) {
      e.preventDefault();
      e.stopPropagation();
      aiFlip(1);
    } else if (prevKeys.includes(k)) {
      e.preventDefault();
      e.stopPropagation();
      aiFlip(-1);
    }
  }

  $effect(() => {
    void isReader;
    if (!isReader) return;
    window.addEventListener("keydown", onReaderKeys, true);
    return () => window.removeEventListener("keydown", onReaderKeys, true);
  });

  function onListWheel(e: WheelEvent) {
    if (!isReader) return;
    if (aiPages <= 1) return;
    e.preventDefault();
    const d = e.deltaMode === 1 ? e.deltaY * 16 : e.deltaY;
    if (Math.abs(d) < 2) return;
    aiFlip(d > 0 ? 1 : -1);
  }

  let aiProgDrag = $state(false);
  let aiPct = $derived(aiPages > 1 ? Math.round((aiPage / (aiPages - 1)) * 100) : 100);

  // 每条回复的起始页与所属页范围（用于底部进度条分段 + 点击跳转）
  let pageStarts: number[] = $state([]);
  let aiReplyOfPage = $derived.by(() => {
    let cur = -1;
    for (let i = 0; i < pageStarts.length; i++) {
      if (pageStarts[i] <= aiPage) cur = i;
      else break;
    }
    return cur;
  });
  function segWeight(gi: number): number {
    if (!pageStarts.length) return 1;
    const start = pageStarts[gi] ?? 0;
    const end = gi + 1 < pageStarts.length ? pageStarts[gi + 1] : aiPages;
    return Math.max(1, end - start);
  }
  function segPageOf(gi: number): number {
    return pageStarts[gi] ?? 0;
  }

  function aiSeekFromEvent(e: PointerEvent): number {
    const rect = (e.currentTarget as HTMLDivElement).getBoundingClientRect();
    return Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
  }

  function aiSeekFrac(f: number) {
    followEnd = false;
    if (aiPages <= 1) { aiPage = 0; applyAiPage(); return; }
    aiPage = Math.max(0, Math.min(Math.round(f * (aiPages - 1)), aiPages - 1));
    applyAiPage();
  }

  function aiProgDown(e: PointerEvent) {
    aiProgDrag = true;
    try { (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId); } catch { /* ignore */ }
    aiSeekFrac(aiSeekFromEvent(e));
  }
  function aiProgMove(e: PointerEvent) {
    if (aiProgDrag) aiSeekFrac(aiSeekFromEvent(e));
  }
  function aiProgUp(e: PointerEvent) {
    if (!aiProgDrag) return;
    aiProgDrag = false;
    aiSeekFrac(aiSeekFromEvent(e));
  }

  let ro: ResizeObserver | null = null;
  $effect(() => {
    void isReader;
    void aiClip;
    void aiInner;
    void readerStyle;
    void panelWidth;
    if (!isReader || !aiClip) return;
    ro?.disconnect();
    ro = new ResizeObserver(() => scheduleMeasure());
    ro.observe(aiClip);
    if (aiInner) ro.observe(aiInner);
    // 仅重新测量（measureAi 内部按 followEnd 决定是否钉末页）；
    // 不能用 force，否则 readerStyle（含缩进）每次变化重跑本 effect 会把用户从中间页弹到末页
    const t = setTimeout(() => scheduleMeasure(), 300);
    return () => {
      clearTimeout(t);
      ro?.disconnect();
      ro = null;
    };
  });

  // 阅读样式下自动跳到最新一页（置位跟随；用户手动翻页时解除）
  $effect(() => {
    if (!isReader) return;
    void msgs.length;
    const last = msgs[msgs.length - 1];
    if (last && last.role === "assistant") {
      followEnd = true;
      queueMicrotask(() => scheduleMeasure(true));
    }
  });

  /* ---------- 独立窗口：跟随主窗口设置 ---------- */
  // 跨窗同步改由 stores.svelte.ts 里的全局 flowread-config 监听统一处理
  // （主/独立窗口双向、lastSavedJson 去重）；这里不再单独监听，避免旧配置覆盖刚切换的端点。
  // 首次进入时 load() 已从共享 localStorage 读取最新配置。

  /* ---------- 导出与清空 ---------- */
  function collectExport(): { title: string; html: string }[] {
    const items: { title: string; html: string }[] = [];
    for (let i = 0; i < msgs.length; i++) {
      const m = msgs[i];
      if (m.role !== "assistant" || !m.content.trim()) continue;
      let t = "AI 回复";
      if (i > 0 && msgs[i - 1].role === "user" && ctx.title.includes(" · ")) t += " · " + ctx.title;
      t += ` (${items.length + 1})`;
      items.push({ title: t, html: mdToHtml(m.content) });
    }
    return items;
  }

  async function exportKind(kind: "txt" | "epub") {
    if (streaming) {
      toast("AI 正在回复中，稍后再导出");
      return;
    }
    const items = collectExport();
    if (!items.length) {
      toast("窗口中还没有 AI 内容", "error");
      return;
    }
    const base = (ctx.title || "AI 笔记").replace(/[\\/:*?"<>|]/g, "_");
    const path = await api.pickSave("导出 AI 内容", kind, `${base}-AI.${kind}`);
    if (!path) return;
    try {
      await api.exportAi(path, kind, ctx.title || "AI 笔记", "FlowRead AI", items);
      toast(`已导出 ${items.length} 条 AI 内容：${path}`);
    } catch (e) {
      toast(`导出失败：${e}`, "error");
    }
  }

  function clearAll() {
    msgs.length = 0;
    aiPage = 0;
    lastAi = -1;
    scheduleMeasure(true);
    toast("已清空窗口中的 AI 内容");
  }

  function toggleDisplay() {
    config.ai.display = isReader ? "chat" : "reader";
    saveConfig();
    tick().then(() => scheduleMeasure(true));
  }
</script>

<svelte:window onkeydown={onGlobalKeys} />

{#if !standalone}
  <div class="ai-resize" onpointerdown={startResize}></div>
{/if}

<div class="ai-panel" class:standalone class:reader={isReader} style={(isReader ? readerVars + ";" : "") + (!standalone ? `width:${panelWidth}px;` : "")}>
  {#if isReader}
    <div class="ai-head">
      <span class="ai-title">{ctx.title || "AI 助手"}</span>
      <div class="ai-actions">
        <select class="ep-sel" title="切换端点" bind:value={config.ai.active} onchange={saveConfigNow}>
          {#each config.ai.profiles as p, i (p.id)}
            <option value={i}>{p.name || `端点${i + 1}`}</option>
          {/each}
        </select>
        <button class="mini" title={isReader ? "切回对话气泡样式（原始）" : "切换为阅读样式：像正文一样翻页、隐藏自己的提问"} onclick={toggleDisplay}>
          {isReader ? "对话样式" : "阅读样式"}
        </button>
        <button class="mini" title="导出 TXT" onclick={() => exportKind("txt")}>TXT</button>
        <button class="mini" title="导出 EPUB" onclick={() => exportKind("epub")}>ePub</button>
        <button class="mini" title="清空本窗口内容" onclick={clearAll}>清空</button>
        {#if standalone}
          <button class="mini" title="停靠回主窗口" onclick={async () => {
            saveAiMode("drawer");
            try {
              await import("@tauri-apps/api/event").then((m) => m.emit("ai-dock", {}));
            } catch { /* ignore */ }
            try {
              const m = await import("@tauri-apps/api/window");
              await m.getCurrentWindow().close();
            } catch {
              try { window.close(); } catch { /* ignore */ }
            }
          }}>⇲ 停靠</button>
        {:else}
          <button class="mini" title="弹出为独立窗口（记住大小）" onclick={popOut}>⧉</button>
          <button class="icon" onclick={() => (ui.aiOpen = false)}>×</button>
        {/if}
      </div>
    </div>
    <div class="prompt-bar">
      {#each config.ai.prompts as p (p.id)}
        <button class="prompt-chip" onclick={() => firePrompt(p)} disabled={streaming} title={p.text}>
          {p.name}<span class="pkey">{p.key}</span>
        </button>
      {/each}
      <button class="prompt-chip add" onclick={() => (ui.settingsOpen = true)} title="在设置→AI 中切换窗口样式/管理提示词">⚙</button>
      <div class="flex-fill"></div>
      <span class="pg-ind">{aiPages > 1 ? `${aiPage + 1}/${aiPages}` : ""}</span>
      {#if aiPages > 1}
        <button class="mini" onclick={() => aiFlip(-1)} title="上一页 (←)">‹</button>
        <button class="mini" onclick={() => aiFlip(1)} title="下一页 (→)">›</button>
      {/if}
    </div>
    <div class="ai-clip" bind:this={aiClip} role="document" onwheel={onListWheel}>
      <div class="ai-inner" bind:this={aiInner}>
        {#if !msgs.length}
          <div class="ai-page">
            <div class="ai-empty">
              {#if config.ai.enabled}
                端点：{profile?.name || "未配置"}｜正在阅读：{ctx.title || "—"}<br />
                点击上方提示词或输入问题，AI 回复将以阅读样式逐页呈现（←/→ 或滚轮翻页）。<br />
                窗口关闭后内容自动清空；可用上方 TXT/ePub 按钮导出。
              {:else}
                AI 未启用，请在设置中开启并配置端点。
              {/if}
            </div>
          </div>
        {/if}
        {#each replyIdxs as [m, gi] (gi)}
          <div class="ai-page ai-reply">
            {#if gi > 0 && msgs[gi - 1].role === "user"}
              <div class="ai-q-title">问：{qShort(msgs[gi - 1])}</div>
            {/if}
            <div class="ai-q-title">《{ctx.title || "AI 回复"}》</div>
            <div class="ai-mark" ondblclick={() => copyText(m.content)} title="双击复制回复">{@html m.content ? mdToHtml(m.content) : "<p>…</p>"}</div>
          </div>
        {/each}
      </div>
    </div>
    <div class="ai-footer">
      <span class="ai-pg-text">{aiPages > 1 ? `第 ${aiPage + 1}/${aiPages} 页` : "AI 阅读进度"}</span>
      <div
        class="ai-prog-track"
        class:dragging={aiProgDrag}
        role="slider"
        tabindex="-1"
        aria-label="翻页进度"
        aria-valuemin={1}
        aria-valuemax={aiPages}
        aria-valuenow={aiPage + 1}
        onpointerdown={aiProgDown}
        onpointermove={aiProgMove}
        onpointerup={aiProgUp}
        onpointercancel={aiProgUp}
      >
        <div class="ai-pg-segs">
          {#each replyIdxs as [m, gi] (gi)}
            <div
              class="ai-pg-seg"
              class:cur={aiReplyOfPage === gi}
              title={(gi > 0 && msgs[gi - 1]?.role === "user" ? qShort(msgs[gi - 1]) : "回复 " + (gi + 1))}
              style={`flex:${segWeight(gi)} 1 0`}
              onclick={(e) => { e.stopPropagation(); followEnd = false; aiPage = segPageOf(gi); applyAiPage(); }}
            ></div>
          {/each}
        </div>
        <div class="ai-prog-fill" style="width:{aiPages > 1 ? (aiPage / (aiPages - 1)) * 100 : 100}%"></div>
        <div class="ai-pg-dot" style="left:{aiPages > 1 ? (aiPage / (aiPages - 1)) * 100 : 100}%"></div>
      </div>
      <span class="ai-pg-text">{aiPct}%</span>
    </div>
    <div class="ai-input">
      <input
        placeholder="向 AI 提问（结合本章上下文），Enter 发送"
        bind:value={input}
        onkeydown={(e) => e.key === "Enter" && send()}
        disabled={streaming}
      />
      <button class="primary" onclick={send} disabled={streaming}>发送</button>
    </div>
  {:else}
    <div class="ai-head">
      <span class="ai-title">{ctx.title || "AI 助手"}</span>
      <div class="ai-actions">
        <select class="ep-sel" title="切换端点" bind:value={config.ai.active} onchange={saveConfigNow}>
          {#each config.ai.profiles as p, i (p.id)}
            <option value={i}>{p.name || `端点${i + 1}`}</option>
          {/each}
        </select>
        <button class="mini" title={isReader ? "切回对话气泡样式（原始）" : "切换为阅读样式：像正文一样翻页、隐藏自己的提问"} onclick={toggleDisplay}>
          {isReader ? "对话样式" : "阅读样式"}
        </button>
        <button class="mini" title="导出 TXT" onclick={() => exportKind("txt")}>TXT</button>
        <button class="mini" title="导出 EPUB" onclick={() => exportKind("epub")}>ePub</button>
        <button class="mini" title="清空本窗口内容" onclick={clearAll}>清空</button>
        {#if standalone}
          <button class="mini" title="停靠回主窗口" onclick={async () => {
            saveAiMode("drawer");
            try {
              await import("@tauri-apps/api/event").then((m) => m.emit("ai-dock", {}));
            } catch { /* ignore */ }
            try {
              const m = await import("@tauri-apps/api/window");
              await m.getCurrentWindow().close();
            } catch {
              try { window.close(); } catch { /* ignore */ }
            }
          }}>⇲ 停靠</button>
        {:else}
          <button class="mini" title="弹出为独立窗口（记住大小）" onclick={popOut}>⧉</button>
          <button class="icon" onclick={() => (ui.aiOpen = false)}>×</button>
        {/if}
      </div>
    </div>
    <div class="prompt-bar">
      {#each config.ai.prompts as p (p.id)}
        <button class="prompt-chip" onclick={() => firePrompt(p)} disabled={streaming} title={p.text}>
          {p.name}<span class="pkey">{p.key}</span>
        </button>
      {/each}
      <button class="prompt-chip add" onclick={() => (ui.settingsOpen = true)} title="在设置→AI 中切换窗口样式/管理提示词">⚙</button>
    </div>
    <div class="ai-list" bind:this={listEl}>
      {#each msgs as m, i (i)}
        <div class="ai-msg {m.role}" ondblclick={() => m.role === "assistant" && copyText(m.content)} title={m.role === "assistant" ? "双击复制回复" : ""}>
          <div class="ai-bubble">{m.content || (streaming && i === msgs.length - 1 ? "…" : "")}</div>
        </div>
      {:else}
        <div class="ai-empty">
          {#if config.ai.enabled}
            端点：{profile?.name || "未配置"}｜正在阅读：{ctx.title || "—"}<br />
            上方提示词可一键执行（默认 F1–F9 快捷键可自定义）；打开新章节自动执行第一个提示词；也可直接提问。
          {:else}
            AI 未启用，请在设置中开启并配置端点。
          {/if}
        </div>
      {/each}
    </div>
    <div class="ai-input">
      <input
        placeholder="向 AI 提问（结合本章上下文）…"
        bind:value={input}
        onkeydown={(e) => e.key === "Enter" && send()}
        disabled={streaming}
      />
      <button class="primary" onclick={send} disabled={streaming}>发送</button>
    </div>
  {/if}
</div>

<style>
  .ep-sel {
    max-width: 110px;
    font-size: 12px;
    padding: 3px 6px;
  }

  .prompt-bar {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
    padding: 8px 10px;
    border-bottom: 1px solid rgba(128, 128, 128, 0.13);
    align-items: center;
  }

  .flex-fill { flex: 1; }

  .pg-ind {
    font-size: 12px;
    opacity: 0.65;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  /* ---------- 阅读样式：底部进度条 ---------- */
  .ai-footer {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 5px 16px 8px;
    font-size: 12px;
    opacity: 0.9;
  }

  .ai-pg-text {
    white-space: nowrap;
    opacity: 0.75;
    font-variant-numeric: tabular-nums;
  }

  .ai-prog-track {
    flex: 1;
    height: 16px;
    position: relative;
    display: flex;
    align-items: center;
    cursor: pointer;
    touch-action: none;
  }

  .ai-prog-track::before {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    height: 4px;
    background: rgba(128, 128, 128, 0.15);
    border-radius: 2px;
    pointer-events: none;
  }

  .ai-pg-segs {
    position: absolute;
    left: 0;
    right: 0;
    height: 4px;
    display: flex;
    gap: 2px;
    pointer-events: none;
  }

  .ai-pg-seg {
    pointer-events: auto;
    background: rgba(128, 128, 128, 0.22);
    border-radius: 2px;
    height: 4px;
  }

  .ai-pg-seg.cur { background: rgba(128, 128, 128, 0.42); }
  .ai-pg-seg:hover { background: var(--accent, #4a6b8a); }

  .ai-prog-fill {
    position: relative;
    height: 4px;
    background: color-mix(in srgb, var(--accent, #4a6b8a) 55%, transparent);
    border-radius: 2px;
    pointer-events: none;
    z-index: 1;
    max-width: 100%;
    transition: width 0.18s ease;
  }

  .ai-pg-dot {
    position: absolute;
    top: 50%;
    transform: translate(-50%, -50%);
    width: 11px;
    height: 11px;
    border-radius: 50%;
    background: var(--accent, #4a6b8a);
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.3);
    opacity: 0;
    transition: opacity 0.15s;
    pointer-events: none;
    z-index: 2;
  }

  .ai-prog-track:hover .ai-pg-dot,
  .ai-prog-track.dragging .ai-pg-dot { opacity: 1; }
  .ai-prog-track.dragging { cursor: grab; }

  .prompt-chip {
    font-size: 12px;
    padding: 3px 9px;
    border-radius: 20px;
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }

  .prompt-chip.add {
    padding: 3px 8px;
  }

  .pkey {
    font-size: 10px;
    opacity: 0.55;
    font-family: Consolas, monospace;
  }

  .ai-resize {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: 6px;
    cursor: ew-resize;
    z-index: 46;
  }

  .ai-panel {
    position: absolute;
    right: 0;
    top: 0;
    bottom: 0;
    background: color-mix(in srgb, var(--rdr-bg) 92%, black 8%);
    border-left: 1px solid rgba(128, 128, 128, 0.2);
    z-index: 45;
    display: flex;
    flex-direction: column;
    box-shadow: -8px 0 24px rgba(0, 0, 0, 0.1);
    animation: slide-in 0.18s ease;
  }

  .ai-panel.standalone {
    position: fixed;
    width: 100%;
    height: 100%;
    border: none;
    box-shadow: none;
    animation: none;
  }

  .ai-panel.reader {
    background: var(--ai-bg, var(--rdr-bg));
    color: var(--ai-fg, var(--rdr-fg));
  }

  @keyframes slide-in {
    from { transform: translateX(28px); opacity: 0; }
    to { transform: translateX(0); opacity: 1; }
  }

  .ai-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 10px 14px;
    border-bottom: 1px solid rgba(128, 128, 128, 0.15);
    flex-wrap: wrap;
  }

  .ai-panel .ai-actions .mini {
    padding: 3px 7px;
    font-size: 11.5px;
  }

  .ai-panel.reader .ai-head {
    background: color-mix(in srgb, var(--ai-bg, var(--rdr-bg)) 92%, black 8%);
  }

  .ai-title {
    font-weight: 600;
    font-size: 13px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 1;
  }

  .ai-actions {
    display: flex;
    gap: 6px;
    align-items: center;
    flex-wrap: wrap;
    justify-content: flex-end;
  }

  .mini {
    font-size: 12px;
    padding: 3px 10px;
  }

  .ai-list {
    flex: 1;
    overflow-y: auto;
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .ai-msg {
    display: flex;
  }

  .ai-msg.user {
    justify-content: flex-end;
  }

  .ai-bubble {
    max-width: 88%;
    padding: 8px 12px;
    border-radius: 12px;
    font-size: 13px;
    line-height: 1.6;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .ai-msg.user .ai-bubble {
    background: var(--accent);
    color: #fff;
    border-bottom-right-radius: 4px;
  }

  .ai-msg.assistant .ai-bubble {
    background: rgba(128, 128, 128, 0.12);
    border-bottom-left-radius: 4px;
  }

  .ai-empty {
    text-align: center;
    opacity: 0.5;
    font-size: 13px;
    padding: 30px 20px;
    line-height: 1.7;
  }

  .ai-input {
    display: flex;
    gap: 8px;
    padding: 10px 12px;
    border-top: 1px solid rgba(128, 128, 128, 0.15);
  }

  .ai-input input {
    flex: 1;
  }

  /* ---------- 阅读样式：翻页正文 ---------- */
  .ai-clip {
    flex: 1;
    position: relative;
    overflow: hidden;
    min-height: 0;
  }

  .ai-inner {
    position: absolute;
    column-fill: auto;
  }

  .ai-page + .ai-page {
    break-before: column;
  }

  .ai-q-title {
    font-size: 0.78em;
    opacity: 0.5;
    margin: 0 0 0.7em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .ai-mark {
    text-align: justify;
    text-justify: inter-ideograph;
    font-synthesis: none;
    overflow-wrap: break-word;
    cursor: default;
    /* 与 Reader 的 --rdr-font（已含 latinFont 前置）同一机制；
       动态注入的 .ai-panel.reader .ai-mark !important 在其之后加载，稳定覆盖此基础值 */
    font-family: var(--rdr-font, "Microsoft YaHei", "Segoe UI", sans-serif);
  }

  .ai-mark :global(h1), .ai-mark :global(h2), .ai-mark :global(h3), .ai-mark :global(h4) {
    font-weight: 600;
    margin: 0.7em 0 0.4em;
  }

  .ai-mark :global(.li) {
    padding-left: 1.4em;
    text-indent: -1.4em;
  }

  .ai-mark :global(ul), .ai-mark :global(ol) {
    padding-left: 1.6em;
  }

  .ai-mark :global(li) { margin: 0.25em 0; }

  .ai-mark :global(pre) {
    background: rgba(128, 128, 128, 0.12);
    padding: 8px 10px;
    border-radius: 6px;
    overflow: hidden;
    white-space: pre-wrap;
    word-break: break-all;
    margin: 0.6em 0;
  }

  .ai-mark :global(code) {
    font-family: Consolas, "Courier New", monospace;
    background: rgba(128, 128, 128, 0.12);
    border-radius: 4px;
    padding: 0.05em 0.25em;
  }

  .ai-mark :global(blockquote) {
    margin: 0.6em 0 0.6em 1.2em;
    padding-left: 0.8em;
    border-left: 2px solid rgba(128, 128, 128, 0.35);
    opacity: 0.9;
  }

  .ai-page + .ai-page .ai-q-title:first-child { margin-top: 0.2em; }
</style>
