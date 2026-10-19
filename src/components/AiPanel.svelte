<script lang="ts">
  import { aiErrDetail, t } from "../lib/i18n/index.svelte";
  import { seedEndpointLabel, seedPromptLabel } from "../lib/seedNames.svelte";
  import { onDestroy, tick } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api } from "../lib/api";
  import { esc, mdToHtml } from "../lib/md";
  import { dragSort } from "../lib/dragSort";
import { reorderByDrop } from "../lib/reorder";
  import { dragOutHandle } from "../lib/windowSnap";
  import { AI_CTX_KEY } from "../lib/aiPanelKeys";
  import {
    IconArrowsSplit2,
    IconChevronLeft,
    IconChevronRight,
    IconSettings,
    IconX,
  } from "@tabler/icons-svelte";
  import {
    config,
    saveConfig,
    saveConfigNow,
    toast,
    ui,
    openAiWindow,
    saveAiMode,
    THEMES,
    AI_WIN_LABEL,
    stripNonTextHtml,
    resolveEndpointProxy,
    marginScopeOf,
    type PromptItem,
  } from "../lib/stores.svelte";
  import {
    formControlOf,
    pushVerdict,
    reduceWheelTurn,
    WHEEL_TURN_IDLE,
    type WheelTurnState,
  } from "../lib/readerInput";

  let { chapterTitle = "", chapterHtml = "", standalone = false, onclose }: {
    chapterTitle?: string;
    chapterHtml?: string;
    standalone?: boolean;
    /** 抽屉模式下由 Reader 传入：× 关闭与「弹出为独立窗成功」时调用，统一把单一状态源 rightPanel 置 null。 */
    onclose?: () => void;
  } = $props();

  interface Msg {
    role: "user" | "assistant";
    content: string;
  }

  let msgs: Msg[] = $state([]);
  let input = $state("");
  let streaming = $state(false);
  let listEl: HTMLDivElement | undefined = $state();

  /* ---------- 流式增量合并（原理见 pushDelta 的注释） ---------- */
  const STREAM_MS = 90;
  let streamBuf = "";
  let streamLast = 0;
  let streamTimer: ReturnType<typeof setTimeout> | undefined;
  onDestroy(() => dropStream());
  let autoFiredFor = "";
  let sessionSeq = 0;

  /* ---------- 阅读样式（翻页）相关 ---------- */
  let aiClip: HTMLDivElement | undefined = $state();
  let aiInner: HTMLDivElement | undefined = $state();
  /** 面板根元素：判断「焦点是否在面板内」（按键接管条件之一）用。 */
  let panelEl: HTMLDivElement | undefined = $state();
  let aiPage = $state(0);
  let aiPages = $state(1);
  /** 面板内滚轮翻页的累积状态（判定在 lib/readerInput.reduceWheelTurn）。 */
  let aiWheelState: WheelTurnState = WHEEL_TURN_IDLE;
  let aiColW = 0;
  let aiStep = 0;
  let measureTimer: ReturnType<typeof setTimeout> | undefined;

  let profile = $derived(
    config.ai.profiles[Math.min(config.ai.active, config.ai.profiles.length - 1)] ?? config.ai.profiles[0]
  );

  let ctx = $state({ title: "", html: "" });

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

  // 独立 AI 窗：测标题行高度写入全局变量让右上角 WindowControls 垂直对齐。
  // ai-head 有 10px 上下内边距，标题行 ai-head-top 从 y=10 起，故容器高度取 saHeadH + 20，
  // 使控件中心线与标题行中心线重合。仅 standalone 时写入，避免抽屉模式污染主窗顶栏变量。
  let saHeadH = $state(0);
  $effect(() => {
    if (standalone && saHeadH > 0) {
      document.documentElement.style.setProperty("--tp-titlebar-h", saHeadH + 20 + "px");
    }
  });

  $effect(() => {
    let done = false;
    const ctrl = listen<{ title: string; html: string }>("tidepage-ai-chapter", (e) => {
      if (!done) onChapterAuto(e.payload);
    });
    // Same-window fallback event too (浏览器原生事件不跨窗口，但面板场景保证覆盖)
    const local = (e: Event) => {
      const ce = e as CustomEvent<{ title: string; html: string }>;
      if (!done) onChapterAuto(ce.detail);
    };
    window.addEventListener("tidepage-ai-chapter", local);
    return () => {
      done = true;
      window.removeEventListener("tidepage-ai-chapter", local);
      ctrl.then((fn) => fn());
    };
  });

  if (standalone) {
    try {
      const raw = localStorage.getItem(AI_CTX_KEY);
      if (raw) ctx = JSON.parse(raw);
    } catch { /* ignore */ }
    window.addEventListener("storage", (e) => {
      if (e.key === AI_CTX_KEY && e.newValue) {
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

  const AI_CTX_CHARS = 9000;
  function chapterText(html: string, limit = AI_CTX_CHARS): string {
    const div = document.createElement("div");
    div.innerHTML = stripNonTextHtml(html);
    const text = (div.textContent ?? "").replace(/\s+/g, " ").trim();
    return text.length > limit ? text.slice(0, limit) + t("ai.truncated") : text;
  }

  function push(role: Msg["role"], content: string): Msg {
    msgs.push({ role, content });
    return msgs[msgs.length - 1];
  }

  function errKindLabel(kind: string): string {
    switch (kind) {
      case "network": return t("ai.errKind.network");
      case "timeout": return t("ai.errKind.timeout");
      case "http_4xx": return t("ai.errKind.http4xx");
      case "http_5xx": return t("ai.errKind.http5xx");
      case "protocol": return t("ai.errKind.protocol");
      case "model_id": return t("ai.errKind.modelId");
      case "config": return t("ai.errKind.config");
      default: return t("ai.errKind.unknown");
    }
  }

  // 后端 ai_chat 失败时 reject 结构化对象 {kind,code,message}；此处兼容字符串/Error 兜底分类。
  function normalizeAiError(e: unknown): { kind: string; code: string; message: string } {
    if (e && typeof e === "object") {
      const o = e as Record<string, unknown>;
      if ("message" in o || "kind" in o || "code" in o) {
        return {
          kind: typeof o.kind === "string" ? o.kind : "unknown",
          code: typeof o.code === "string" ? o.code : "",
          message: typeof o.message === "string" ? o.message : JSON.stringify(e),
        };
      }
    }
    // 非结构化错误（字符串 / Error / 其它对象）：类别无从得知，一律 unknown。
    // 早先这里按「超时/网络/协议不匹配/模型ID」等中文词猜类别，但后端静态中文已随
    // AiError.code 搬进文案表，这些词只会出现在上游原文里，按它猜反而会猜错。
    const s = typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
    return { kind: "unknown", code: "", message: s };
  }

  /* ---------- 流式增量合并 ----------
   * LLM 每秒吐 20–80 个 delta。逐 delta 直接写 reply.content 的代价是三重的：
   *   1) 写一次深 $state 代理 → 模板里的 mdToHtml 把**整条**消息重解析一遍，
   *      而消息在变长，故整次回答的累计解析量是 O(内容长度²)；
   *   2) {@html} 整体替换该条消息的 DOM 子树（每 delta 一次全量重建）；
   *   3) 紧随其后的 scrollHeight 读触发一次强制布局。
   * 改为 delta 先入普通字符串缓冲、至多每 STREAM_MS 合并写入一次，每秒渲染次数从
   * ~50 降到 ~11。90ms 与 scheduleMeasure 的 120ms 同量级；流式动画本来就是按
   * 「字在长」读的，11Hz 与 50Hz 人眼分辨不出。
   * 首帧是 leading edge（streamLast 初值 0 → 等待为 0），故「单个 delta 一次到齐」
   * 的短回复仍然立刻上屏，不吃这 90ms。 */
  function flushStream(target: Msg) {
    if (streamTimer !== undefined) {
      clearTimeout(streamTimer);
      streamTimer = undefined;
    }
    streamLast = Date.now();
    if (!streamBuf) return;
    const add = streamBuf;
    streamBuf = "";
    target.content += add;
    tick().then(() => {
      listEl?.scrollTo({ top: listEl.scrollHeight });
    });
  }

  /** 收到一个流式 delta：入缓冲，并保证至多每 STREAM_MS 落一次 content。 */
  function pushDelta(target: Msg, delta: string) {
    streamBuf += delta;
    if (streamTimer !== undefined) return;
    const wait = Math.max(0, STREAM_MS - (Date.now() - streamLast));
    streamTimer = setTimeout(() => {
      streamTimer = undefined;
      flushStream(target);
    }, wait);
  }

  /** 丢弃缓冲（切换端点重试、或整体失败要改写 content 时用）。 */
  function dropStream() {
    if (streamTimer !== undefined) {
      clearTimeout(streamTimer);
      streamTimer = undefined;
    }
    streamBuf = "";
  }

  async function run(system: string, userText: string) {
    if (streaming) {
      toast(t("ai.busy"));
      return;
    }
    const profiles = config.ai.profiles;
    if (!profiles.length) {
      toast(t("ai.noEndpoint"), "error");
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
    const failures: { name: string; kind: string; code: string; message: string }[] = [];

    for (let a = 0; a < maxTries; a++) {
      const idx = order[a];
      const p = profiles[idx];
      if (!p || !p.baseUrl || !p.model) {
        failures.push({ name: seedEndpointLabel(p?.name ?? "", t("ai.endpointDefault", { n: idx + 1 })), kind: "config", code: "", message: t("ai.noAddrOrModel") });
        continue;
      }
      const label = seedEndpointLabel(p.name, t("ai.endpointDefault", { n: idx + 1 }));
      if (a > 0) {
        const prev = failures[failures.length - 1];
        // 重试前清空上一个端点可能已流式写入的内容，避免拼接错乱
        // （缓冲里还没落盘的那段也要丢，否则稍后 flush 会把它接到新端点的内容前面）
        dropStream();
        reply.content = "";
        toast(t("ai.failoverToast", { prev: prev.name, kind: errKindLabel(prev.kind), next: label }));
      }
      const id = `s${++sessionSeq}-${Date.now()}`;
      let active = true;
      let un: (() => void) | null = null;
      const stop = listen<{ delta: string; done: boolean }>(`ai-token:${id}`, (e) => {
        if (!active) return;
        if (e.payload.delta) {
          pushDelta(reply, e.payload.delta);
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
          ...resolveEndpointProxy(p),
        });
        active = false;
        un?.();
        flushStream(reply); // 收尾：把缓冲里最后一段立即落盘，不等下一个节拍
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
        failures.push({ name: label, kind: err.kind, code: err.code, message: err.message });
        if (!config.ai.failover) break;
      }
    }

    // 逐条失败行：code 命中 → 用译文整句顶替（译文本身已含类别描述与动态尾巴，
    // 再叠一层 errKind 会变成「网络错误：网络失败：…」这种重复）；未命中则沿用旧格式。
    // 先丢弃缓冲：下面整体改写 content，若缓冲里还留着上一段的残片，稍后 flush 会把它接在后面。
    dropStream();
    reply.content = failures.length
      ? `${t("ai.allFailedTitle")}\n${failures
          .map((f) => {
            const coded = aiErrDetail(f);
            return coded !== null
              ? t("ai.endpointFailedLineCoded", { name: f.name, err: coded })
              : t("ai.endpointFailedLine", { name: f.name, kind: errKindLabel(f.kind), msg: f.message });
          })
          .join("\n")}`
      : t("ai.noEndpointAvailable");
    streaming = false;
    scheduleMeasure();
    toast(t("ai.allFailedToast"), "error");
  }

  function renderPrompt(p: PromptItem, limit = AI_CTX_CHARS): string {
    return p.text
      .split("{chapter}")
      .join(chapterText(ctx.html, limit))
      .split("{title}")
      .join(ctx.title);
  }

  function firePrompt(p: PromptItem) {
    if (!ctx.html) {
      toast(t("ai.waitChapter"));
      return;
    }
    run(config.ai.system, renderPrompt(p));
  }

  /** 提示词按钮真实拖拽排序（特效同主程序顶栏，走 dragSort + .tp-drag-* 样式）。
   *  直接重排 config.ai.prompts 数组并持久化：这样「自动执行第一个提示词」(prompts[0])
   *  在排序变动后天然指向新的首位，无需另存顺序表，避免出现执行到旧首位的漂移。 */
  function promptDrop(from: string, target: string, place: "before" | "after") {
    const next = reorderByDrop(config.ai.prompts, from, target, place, (p) => String(p.id));
    if (!next) return;
    config.ai.prompts = next;
    saveConfig();
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
    // i18n-allow: 用户内容：AI 提示词正文是种子数据（用户可编辑），规则要求永不翻译。
    const context = `当前正在阅读《${ctx.title}》，内容节选：\n${chapterText(ctx.html, 5000)}\n\n我的问题：${q}`;
    run(config.ai.system, context);
  }

  async function popOut() {
    // 弹出为独立窗口成功后关闭抽屉（onclose→rightPanel=null）；失败则保持抽屉打开
    const ok = await openAiWindow();
    if (ok) onclose?.();
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
    const rm = r.margins[marginScopeOf(r)];
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
      mh: s.followReader ? rm.h : (s.marginH > 0 ? s.marginH : rm.h),
      mv: s.followReader ? rm.v : (s.marginV > 0 ? s.marginV : rm.v),
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
        aiStyleTag.id = "tidepage-ai-reader-css";
        document.head.appendChild(aiStyleTag);
      }
      aiStyleTag.textContent = readerCss;
    } else {
      document.getElementById("tidepage-ai-reader-css")?.remove();
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
    // 只用 transform 表达页偏移（合成层），避免改 left 触发布局。
    const x = Math.round((aiClip.clientWidth - aiColW) / 2) - aiPage * aiStep;
    aiInner.style.left = "0px";
    aiInner.style.transform = `translate3d(${x}px,0,0)`;
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

  /* ---------- 阅读样式：内容区四角点击翻页热区 ----------
   * 左上/左下 → 上一页；右上/右下 → 下一页；中段不响应。复用 aiFlip。
   * pointerup 命中四角即时翻页（跟手、无任何 setTimeout/判定窗/延时）。
   * 选词优先（不依赖延时，三条判定）：
   *   ① pointerdown→pointerup 位移超阈值（划词/拖拽）→ 不翻页；
   *   ② 抬起时存在非折叠选区 → 不翻页；
   *   ③ 命中可交互元素 closest("a,button,input,textarea,select") → 不翻页。 */
  const CLIP_MOVE_THRESHOLD = 6; // px，指针按下→抬起位移超此值视为划词/拖拽
  let clipDown: { x: number; y: number } | null = null;

  function onClipPointerDown(e: PointerEvent) {
    if (!isReader) return;
    if (e.button !== 0) { clipDown = null; return; } // 仅主键
    clipDown = { x: e.clientX, y: e.clientY };
  }

  function onClipPointerUp(e: PointerEvent) {
    if (!isReader || !aiClip) return;
    const down = clipDown;
    clipDown = null;
    if (!down || e.button !== 0) return;
    // ① 位移超阈值 → 划词/拖拽，不翻页（取样时机=按下与抬起之间）
    if (Math.hypot(e.clientX - down.x, e.clientY - down.y) > CLIP_MOVE_THRESHOLD) return;
    // ② 抬起时刻存在非折叠选区（划词结果）→ 选词优先
    const sel = window.getSelection();
    if (sel && !sel.isCollapsed && sel.toString().trim()) return;
    // ③ 命中可交互元素优先
    const el = e.target as HTMLElement | null;
    if (el && el.closest("a,button,input,textarea,select")) return;
    // 命中四角？上下各 35%、左右各 30%
    const rect = aiClip.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    const zoneW = rect.width * 0.3;
    const zoneH = rect.height * 0.35;
    const inCorner = y <= zoneH || y >= rect.height - zoneH;
    if (!inCorner) return; // 垂直中段不响应
    let dir = 0;
    if (x <= zoneW) dir = -1;
    else if (x >= rect.width - zoneW) dir = 1;
    if (!dir) return; // 水平中间不响应
    aiFlip(dir); // 命中即时翻页，无延时
  }

  function aiTurnBind(id: "next" | "prev"): boolean {
    const b = config.reader.bindings[id];
    return !!b && b.length > 0;
  }

  /** 面板是否「持有输入」：只看焦点是否在面板子树内。
   *  不再看「指针停在面板上」——鼠标恰好停在抽屉上时，→ 会被面板整段吃掉，
   *  正文一动不动，用户完全看不出是悬停导致的。滚轮归属（onListWheel/isWheelLocal）不受影响。
   *  独立窗里没有正文可让，默认持有（保持既有行为）。 */
  function aiHoldsInput(): boolean {
    if (standalone) return true;
    const a = document.activeElement;
    return !!a && !!panelEl && panelEl.contains(a);
  }

  /**
   * 阅读抽屉的按键接管（window 捕获阶段，早于 Reader.onKeyDown 的冒泡阶段）。
   *
   * 旧实现只要 isReader 就对方向键/PageUp/PageDown 无条件 preventDefault + stopPropagation，
   * 结果 AI 只有 1 页时 aiFlip 被 clamp 成空操作，而正文那边又被 stopPropagation 挡死 —— 按键
   * 完全没反应。现在加三重条件，任一不满足就放行给正文：
   *   ① 抽屉确实展开：本组件只在 Reader 的 aiDrawerOpen 为真（或作为独立窗）时挂载，挂载即成立；
   *   ② aiPages > 1：AI 确有多页可翻，否则接管只是白吃一次按键；
   *   ③ 面板持有输入（焦点确实在面板子树内；指针悬停不算——悬停抢键会让正文「按不动」）。
   * ↑/↓ 不写死进键表：正文侧有 scrollDown/scrollUp 绑定，只有面板确实持有输入时才归 AI 消费。
   */
  function onReaderKeys(e: KeyboardEvent) {
    if (!isReader) return;
    // 与 Reader.onKeyDown 共用同一判定（INPUT/TEXTAREA/SELECT/contenteditable，且必须可见）。
    const formDev = formControlOf(e.target);
    if (formDev === "visible") {
      pushVerdict("swallowed:formControl(visible)", e);
      return;
    }
    if (formDev === "hidden") pushVerdict("swallowed:formControl(hidden)", e);
    if (ui.settingsOpen) {
      pushVerdict("swallowed:modal(settingsOpen)", e);
      return;
    }
    const held = aiHoldsInput();
    if (!standalone) {
      if (aiPages <= 1) return; // ② 单页无页可翻 → 放行
      if (!held) return;        // ③ 焦点不在面板内 → 放行（不再看指针悬停）
    }
    const k = e.key;
    const nextKeys = ["ArrowRight", "PageDown", ...(held ? ["ArrowDown"] : []), ...(aiTurnBind("next") ? config.reader.bindings.next : [])];
    const prevKeys = ["ArrowLeft", "PageUp", ...(held ? ["ArrowUp"] : []), ...(aiTurnBind("prev") ? config.reader.bindings.prev : [])];
    if (nextKeys.includes(k)) {
      e.preventDefault();
      e.stopPropagation();
      pushVerdict("swallowed:aiPanel(focus)", e);
      aiFlip(1);
    } else if (prevKeys.includes(k)) {
      e.preventDefault();
      e.stopPropagation();
      pushVerdict("swallowed:aiPanel(focus)", e);
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
    // 单页时不拦截：让滚轮继续冒泡回落到正文（否则停在面板上滚轮两边都没反应）。
    if (aiPages <= 1) return;
    e.preventDefault();
    // 已消费这次滚轮，必须断掉冒泡：否则正文 onWheel 会再翻一页，AI 与正文同时动。
    e.stopPropagation();
    // 与正文共用同一套归一化 + 阈值判定：deltaMode=2 的设备 deltaY=1 也能翻，
    // 小步进先累积再翻，而不是像旧实现那样 |d|<2 直接吞掉。
    const r = reduceWheelTurn(aiWheelState, e.deltaMode, e.deltaY, Date.now());
    aiWheelState = r.state;
    if (r.turn !== 0) aiFlip(r.turn);
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
  // 跨窗同步改由 stores.svelte.ts 里的全局 tidepage-config 监听统一处理
  // （主/独立窗口双向、lastSavedJson 去重）；这里不再单独监听，避免旧配置覆盖刚切换的端点。
  // 首次进入时 load() 已从共享 localStorage 读取最新配置。

  /* ---------- 导出与清空 ---------- */
  function collectExport(): { title: string; html: string }[] {
    const items: { title: string; html: string }[] = [];
    for (let i = 0; i < msgs.length; i++) {
      const m = msgs[i];
      if (m.role !== "assistant" || !m.content.trim()) continue;
      // 局部变量改名 itemTitle：避免与 i18n 的 t() 同名遮蔽
      let itemTitle = t("ai.replyTitle");
      if (i > 0 && msgs[i - 1].role === "user" && ctx.title.includes(" · ")) itemTitle += " · " + ctx.title;
      itemTitle += ` (${items.length + 1})`;
      items.push({ title: itemTitle, html: mdToHtml(m.content) });
    }
    return items;
  }

  async function exportKind(kind: "txt" | "epub") {
    if (streaming) {
      toast(t("ai.busyExport"));
      return;
    }
    const items = collectExport();
    if (!items.length) {
      toast(t("ai.emptyWindow"), "error");
      return;
    }
    const base = (ctx.title || t("ai.notesTitle")).replace(/[\\/:*?"<>|]/g, "_");
    const path = await api.pickSave(t("ai.exportDialogTitle"), kind, `${base}-AI.${kind}`);
    if (!path) return;
    try {
      await api.exportAi(path, kind, ctx.title || t("ai.notesTitle"), "TidePage AI", items);
      toast(t("ai.exported", { n: items.length, path }));
    } catch (e) {
      toast(t("ai.exportFailed", { err: String(e) }), "error");
    }
  }

  function clearAll() {
    msgs.length = 0;
    aiPage = 0;
    lastAi = -1;
    scheduleMeasure(true);
    toast(t("ai.cleared"));
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

<!-- 共享块：顶部 header（两分支共用，无参数）。两行布局：
     第一行 = 标题 +（standalone: ⇲停靠 / 抽屉: ×）；第二行 = 动作按钮单独成行、左对齐、换行规律。 -->
<!-- 标题行内容：standalone 与抽屉两分支共用（仅右侧按钮不同）。 -->
{#snippet aiHeadTopInner()}
  <span class="ai-title">{ctx.title || t("ai.assistantLabel")}</span>
  {#if standalone}
    <button class="mini" data-testid="ai-dock" title={t("ai.dockTip")} onclick={async () => {
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
    }}>{t("ai.dock")}</button>
  {:else}
    <button class="icon" title={t("ai.closePanel")} aria-label={t("ai.closePanel")} onclick={() => onclose?.()}><IconX /></button>
  {/if}
{/snippet}

{#snippet aiHead()}
  <div class="ai-head">
    {#if standalone}
      <!-- 独立窗：标题行整行可拖（"deep" 允许点在子树元素上也算拖拽），双击最大化/还原。 -->
      <div class="ai-head-top sa-head" data-tauri-drag-region="deep" bind:clientHeight={saHeadH}>
        {@render aiHeadTopInner()}
      </div>
    {:else}
      <!-- 阅读抽屉：标题行兼作「拖出面板 → 弹为独立窗」手柄；action 自身会忽略 button/input/select。 -->
      <div class="ai-head-top" use:dragOutHandle={popOut} bind:clientHeight={saHeadH}>
        {@render aiHeadTopInner()}
      </div>
    {/if}
    <div class="ai-actions">
      <select class="ep-sel" data-testid="ai-endpoint" title={t("ai.switchEndpoint")} bind:value={config.ai.active} onchange={saveConfigNow}>
        {#each config.ai.profiles as p, i (p.id)}
          <option value={i}>{seedEndpointLabel(p.name, t("ai.endpointDefault", { n: i + 1 }))}</option>
        {/each}
      </select>
      <button class="mini" data-testid="ai-display" title={isReader ? t("ai.displayBubbleTip") : t("ai.displayReaderTip")} onclick={toggleDisplay}>
        {isReader ? t("ai.displayBubble") : t("ai.displayReader")}
      </button>
      <button class="mini" data-testid="ai-export-txt" title={t("ai.exportTxt")} onclick={() => exportKind("txt")}>TXT</button>
      <button class="mini" data-testid="ai-export-epub" title={t("ai.exportEpub")} onclick={() => exportKind("epub")}>EPUB</button>
      <button class="mini" data-testid="ai-clear" title={t("ai.clearTip")} onclick={clearAll}>{t("ai.clear")}</button>
      {#if !standalone}
        <button class="mini" data-testid="ai-popout" title={t("common.panel.popout")} onclick={popOut}><IconArrowsSplit2 /></button>
      {/if}
    </div>
  </div>
{/snippet}

<!-- 共享块：提示词栏。showPager=true 时（阅读样式）在末尾追加翻页指示与 ‹ › 按钮 -->
{#snippet promptBar(showPager: boolean)}
  <div class="prompt-bar" use:dragSort={{ onDrop: promptDrop }}>
    {#each config.ai.prompts as p (p.id)}
      <button class="prompt-chip" data-drag-id={p.id} onclick={() => firePrompt(p)} disabled={streaming} title={p.text}>
        {seedPromptLabel(p.name)}<span class="pkey">{p.key}</span>
      </button>
    {/each}
    <button class="prompt-chip add" onclick={() => (ui.settingsOpen = true)} title={t("ai.settingsTip")}><IconSettings /></button>
    {#if showPager}
      <div class="flex-fill"></div>
      <span class="pg-ind">{aiPages > 1 ? `${aiPage + 1}/${aiPages}` : ""}</span>
      {#if aiPages > 1}
        <button class="mini" data-testid="ai-prev-page" onclick={() => aiFlip(-1)} title={t("ai.prevPage")}><IconChevronLeft /></button>
        <button class="mini" data-testid="ai-next-page" onclick={() => aiFlip(1)} title={t("ai.nextPage")}><IconChevronRight /></button>
      {/if}
    {/if}
  </div>
{/snippet}

<div
  class="ai-panel"
  class:standalone
  class:reader={isReader}
  role="group"
  bind:this={panelEl}
  style={(isReader ? readerVars + ";" : "") + (!standalone ? `width:${panelWidth}px;` : "")}
>
  {#if isReader}
    {@render aiHead()}
    {@render promptBar(true)}
    <div class="ai-clip" bind:this={aiClip} role="document" onwheel={onListWheel} onpointerdown={onClipPointerDown} onpointerup={onClipPointerUp}>
      <div class="ai-inner" bind:this={aiInner}>
        {#if !msgs.length}
          <div class="ai-page">
            <div class="ai-empty">
              {#if config.ai.enabled}
                {t("ai.metaLine", { endpoint: profile?.name || t("ai.notConfigured"), title: ctx.title || "—" })}<br />
                {t("ai.readerStyleHint")}<br />
                {t("ai.readerExportHint")}
              {:else}
                {t("ai.readerDisabled")}
              {/if}
            </div>
          </div>
        {/if}
        {#each replyIdxs as [m, gi] (gi)}
          <div class="ai-page ai-reply">
            {#if gi > 0 && msgs[gi - 1].role === "user"}
              <div class="ai-q-title">{t("ai.askLabel")} {qShort(msgs[gi - 1])}</div>
            {/if}
            <div class="ai-q-title">{t("ai.bookTitleLine", { title: ctx.title || t("ai.replyTitle") })}</div>
            <div class="ai-mark">{@html m.content ? mdToHtml(m.content) : "<p>…</p>"}</div>
          </div>
        {/each}
      </div>
    </div>
    <div class="ai-footer">
      <span class="ai-pg-text" data-testid="ai-page">{aiPages > 1 ? t("ai.pageLabel", { cur: aiPage + 1, total: aiPages }) : t("ai.progressLabel")}</span>
      <div
        class="ai-prog-track"
        class:dragging={aiProgDrag}
        role="slider"
        tabindex="-1"
        aria-label={t("ai.pageProgressAria")}
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
              title={(gi > 0 && msgs[gi - 1]?.role === "user" ? qShort(msgs[gi - 1]) : t("ai.replyNo", { n: gi + 1 }))}
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
        data-testid="ai-input"
        placeholder={t("ai.inputPlaceholderFull")}
        bind:value={input}
        onkeydown={(e) => e.key === "Enter" && send()}
        disabled={streaming}
        title={t("ai.askWithContextTip")}
      />
      <button class="primary" data-testid="ai-send" onclick={send} disabled={streaming}>{t("ai.send")}</button>
    </div>
  {:else}
    {@render aiHead()}
    {@render promptBar(false)}
    <div class="ai-list" bind:this={listEl}>
      {#each msgs as m, i (i)}
        <div class="ai-msg {m.role}">
          <div class="ai-bubble">{m.content || (streaming && i === msgs.length - 1 ? "…" : "")}</div>
        </div>
      {:else}
        <div class="ai-empty">
          {#if config.ai.enabled}
            {t("ai.metaLine", { endpoint: profile?.name || t("ai.notConfigured"), title: ctx.title || "—" })}<br />
            {t("ai.promptHint")}
          {:else}
            {t("ai.readerDisabled")}
          {/if}
        </div>
      {/each}
    </div>
    <div class="ai-input">
      <input
        data-testid="ai-input"
        placeholder={t("ai.inputPlaceholder")}
        bind:value={input}
        onkeydown={(e) => e.key === "Enter" && send()}
        disabled={streaming}
        title={t("ai.askWithContextTip")}
      />
      <button class="primary" data-testid="ai-send" onclick={send} disabled={streaming}>{t("ai.send")}</button>
    </div>
  {/if}
</div>

<style>
  .ep-sel {
    max-width: 110px;
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    padding: 3px 6px;
  }

  .prompt-bar {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    row-gap: 6px;
    padding: 8px 10px;
    border-bottom: 1px solid rgba(128, 128, 128, 0.13);
    align-items: center;
    align-content: flex-start;
  }

  .flex-fill { flex: 1; }

  .pg-ind {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
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
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
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
    border-radius: var(--radius-pill);
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
    border-radius: var(--radius-pill);
    height: 4px;
  }

  .ai-pg-seg.cur { background: rgba(128, 128, 128, 0.42); }
  .ai-pg-seg {
    transition: background var(--dur-fast) var(--ease);
  }
  .ai-pg-seg:hover { background: var(--accent); }

  .ai-prog-fill {
    position: relative;
    height: 4px;
    background: color-mix(in srgb, var(--accent) 55%, transparent);
    border-radius: var(--radius-pill);
    pointer-events: none;
    z-index: var(--z-base);
    max-width: 100%;
    transition: width var(--dur) var(--ease);
  }

  .ai-pg-dot {
    position: absolute;
    top: 50%;
    transform: translate(-50%, -50%);
    width: 11px;
    height: 11px;
    border-radius: 50%;
    background: var(--accent);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.32), 0 1px 1px rgba(0, 0, 0, 0.18), inset 0 1px 0 rgba(255, 255, 255, 0.35);
    opacity: 0;
    transition: opacity var(--dur-fast) var(--ease);
    pointer-events: none;
    z-index: var(--z-raised);
  }

  .ai-prog-track:hover .ai-pg-dot,
  .ai-prog-track.dragging .ai-pg-dot { opacity: 1; }
  .ai-prog-track.dragging { cursor: grab; }

  .prompt-chip {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    padding: 3px 9px;
    border-radius: var(--radius-pill);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    min-height: 26px;
    box-sizing: border-box;
    white-space: nowrap;
    /* 拖拽排序：禁用触屏滚动手势，指针拖动才可靠（与主程序顶栏一致）。 */
    touch-action: none;
  }

  .prompt-chip.add {
    padding: 3px 8px;
  }

  .pkey {
    font-size: var(--fs-2xs);
    letter-spacing: var(--ls-2xs);
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
    z-index: var(--z-raised);
  }

  /* AI 面板本体：与其它侧面板同一套表面规格（底色/描边/投影走 --rdr-*，
     宽度由状态注入 --sidepanel-w，方向为右侧故不加 .sidepanel--left）。
     阅读区内打开时额外叠一层 --ai-* 变量（面板自己的配色方案）。 */
  .ai-panel {
    position: absolute;
    right: 0;
    top: 0;
    bottom: 0;
    width: var(--sidepanel-w);
    background: var(--rdr-surface);
    border-left: 1px solid var(--rdr-line);
    z-index: var(--z-drawer);
    display: flex;
    flex-direction: column;
    box-shadow: var(--rdr-shadow-panel);
    animation: tp-drawer-in-right var(--dur-lift) var(--ease-premium) both;
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

 .ai-head {
    display: flex;
    flex-direction: column;   /* 两行：标题行 + 动作行 */
    align-items: stretch;
    gap: var(--sp-1);
    padding: 10px 14px;
    border-bottom: 1px solid var(--rdr-line);
  }

  .ai-head-top {
    display: flex;
    align-items: center;
    justify-content: space-between; /* 标题左、关闭/停靠右 */
    gap: var(--sp-2);
  }

  /* 独立 AI 窗：顶部第一行既是拖拽区，也要给右上角自绘窗口控制按钮留出空间，
     避免「⇲ 停靠」按钮被覆盖；空白处可拖动窗口、双击最大化/还原。 */
  .ai-head-top.sa-head {
    padding-right: 112px;
  }

  .ai-panel .ai-actions .mini {
    padding: 3px 7px;
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
  }

  .ai-panel.reader .ai-head {
    background: color-mix(in srgb, var(--ai-bg, var(--rdr-bg)) 92%, black 8%);
  }

  .ai-title {
    font-weight: 600;
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 1 1 auto;
    min-width: 0;
  }

  .ai-actions {
    display: flex;
    gap: 6px;
    align-items: center;
    /* 单行显示：不再换行到第二排；极窄时整行可横向滑动（隐藏滚动条）作为安全阀。 */
    flex-wrap: nowrap;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
    -ms-overflow-style: none;
  }

  .ai-actions::-webkit-scrollbar { width: 0; height: 0; }

  /* 统一动作区各控件高度，消除锯齿；按钮不收缩，端点下拉可收缩以让整行尽量单行放下 */
  .ai-actions > * {
    height: 26px;
    box-sizing: border-box;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: 0 0 auto;
  }

  .ai-actions .ep-sel {
    flex: 0 1 auto;
    min-width: 62px;
  }

  .mini {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
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
    /* 新消息入场：从下方轻微上浮落位。msgs 以 index 为 key，追加新消息才挂载该元素，
       故动画只在「新气泡出现」时跑一次；流式更新只改最后一条文本、元素不重挂，不重播。
       keyframes 定义在 app.css（Svelte 会重命名组件内 @keyframes，reduce-motion 覆盖引用不到）。 */
    animation: tp-bubble-in var(--dur-page) var(--ease-premium) both;
  }

  .ai-msg.user {
    justify-content: flex-end;
  }

  .ai-bubble {
    max-width: 88%;
    padding: 8px 12px;
    border-radius: var(--radius-lg);
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
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
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
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
    left: 0;
    column-fill: auto;
    will-change: transform;
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
    border-radius: var(--radius-sm);
    overflow: hidden;
    white-space: pre-wrap;
    word-break: break-all;
    margin: 0.6em 0;
  }

  .ai-mark :global(code) {
    font-family: Consolas, "Courier New", monospace;
    background: rgba(128, 128, 128, 0.12);
    border-radius: var(--radius-xs);
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
