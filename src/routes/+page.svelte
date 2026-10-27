<script lang="ts">
  // B7：首屏只静态加载 Library/WindowControls；Reader、全部弹窗与独立窗口按需 import。
  import Library from "../components/Library.svelte";
  import WindowControls from "../components/WindowControls.svelte";
  import { confirmAnswer, confirmBox, choiceBox, choiceAnswer, convertState, toasts, ui, copyText, dismissToast, config, THEMES, OPEN_BOOK_EVENT, requestOpenBook } from "../lib/stores.svelte";
  import { AI_OPEN_KEY, AI_WIN_H_KEY, AI_WIN_W_KEY } from "../lib/aiPanelKeys";
  import { t } from "../lib/i18n/index.svelte";
  import { onMount } from "svelte";
  import { IconCopy, IconX } from "@tabler/icons-svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api } from "../lib/api";
  import { outDissolve, outFade, outSlideR } from "../lib/motion";


  const params = typeof window !== "undefined" ? new URLSearchParams(window.location.search) : null;
  const standalone = !!params && params.get("ai") === "1";
  const panelKind = (params && params.get("panel")) || "";
  const standalonePanel = !!panelKind;
  // 独立阅读窗：有 book 参数且既非 AI 窗也非面板窗
  const standaloneBook = !!params && !!params.get("book") && !standalone && !standalonePanel;

  const aiRootStyle = $derived.by(() => {
    const t = THEMES[config.reader.theme];
    const bg = config.reader.bgColor || t.bg;
    const fg = config.reader.fgColor || t.fg;
    return `--rdr-bg:${bg};--rdr-fg:${fg};--panel:${bg};--fg:${fg};background:${bg};color:${fg}`;
  });

  // 封面扫描开关同步给后端：启动时一次 + 每次设置变更一次（后端默认 true，手动模式必须尽早告知）。
  // 独立窗口（AI / 面板 / 独立阅读）读同一份配置，值相同；多调一次无副作用。
  $effect(() => {
    void api.setCoverScan(config.library.coverScan !== "manual").catch(() => {});
  });

  // 自定义字体 @font-face：所有窗口（主窗口 / 独立阅读 / 独立 AI 窗口）统一注入，
  // 独立 AI 窗口不挂载 Reader，必须在这里也能加载用户安装的字体。
  // B1：启动打点 + 非首屏任务延后到空闲（字体预加载/更新检查不再与首屏抢主线程）。
  function whenIdle(fn: () => void, timeout: number): void {
    const ric = (globalThis as { requestIdleCallback?: (cb: () => void, o?: { timeout?: number }) => number })
      .requestIdleCallback;
    if (typeof ric === "function") ric(fn, { timeout });
    else setTimeout(fn, Math.min(timeout, 1200));
  }
  onMount(() => {
    void api.bootStage("page_mount");
  });

  // B7：首屏 ready 后空闲预取 Reader chunk（失败不影响首屏；真正打开时会再次 import）。
  $effect(() => {
    if (standalone || standalonePanel || standaloneBook) return;
    let cancelled = false;
    whenIdle(() => {
      if (!cancelled) void import("../components/Reader.svelte").catch(() => undefined);
    }, 6000);
    return () => { cancelled = true; };
  });

  // 启动后静默检查更新（失败不提示）：有新版本时由界面（设置入口/关于）显示红点。
  let updateCheckStarted = false;
  $effect(() => {
    if (updateCheckStarted || standalone || standalonePanel || standaloneBook) return;
    updateCheckStarted = true;
    let cancelled = false;
    whenIdle(() => {
      if (!cancelled) void import("../lib/update.svelte").then((m) => m.checkForUpdates()).catch(() => undefined);
    }, 4000);
    return () => {
      cancelled = true;
    };
  });

  $effect(() => {
    void config.reader.customFonts;
    // B7：字体注入不阻塞首屏（改字体时也会在下一空闲帧注入）。
    whenIdle(() => { void import("../lib/fontfaces").then((m) => m.syncCustomFontFaces()).catch(() => undefined); }, 1500);
  });
  // 启动即预加载设定字体，首次打开书籍时可直接以正确字体呈现（消除字体闪烁）
  //
  // fontSize 不作为依赖：preloadReaderFonts 每次调用都会对每个 family 发一次
  // document.fonts.load()，外加新建一个 3s setTimeout + Promise.race。字号滑块一拖
  // 就是几十个 input 事件，全都会触发一遍 —— 而字号决定的是「用多大字号渲染」，
  // 不是「需要哪些字体文件」，字体加载只认 family。所以这里只跟字体本身的三项。
  let fontPreloadTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    void config.reader.customFonts;
    void config.reader.fontFamily;
    void config.reader.latinFont;
    void config.reader.fontSize;
    // 换字体时值得立刻预加载，但仍合并同一帧内的连续改动，避免拖动字体选择器时逐项触发。
    // B1：非首屏任务——等空闲再预加载（timeout 兜底 1500ms），不与首屏行/列表抢主线程。
    if (fontPreloadTimer) clearTimeout(fontPreloadTimer);
    fontPreloadTimer = setTimeout(() => {
      fontPreloadTimer = undefined;
      whenIdle(() => {
        void api.bootStage("font_preload_start");
        void import("../lib/fontfaces").then((m) => m.preloadReaderFonts()).catch(() => undefined);
      }, 1500);
    }, 200);
    return () => {
      if (fontPreloadTimer) {
        clearTimeout(fontPreloadTimer);
        fontPreloadTimer = undefined;
      }
    };
  });

  $effect(() => {
    if (standalone) return;
    listen("ai-dock", () => {
      ui.aiOpen = true;
    });
  });

  // 窗口吸附体系：每个窗口启动一次（模块内按 label 自动区分基准窗/卫星窗），卸载时清理监听与组引用。
  // B7：吸附监听不参与首屏关键路径，延后到首个空闲帧启动（卫星窗拖动时仍生效）。
  let snapMod: typeof import("../lib/windowSnap") | null = null;
  $effect(() => {
    whenIdle(() => {
      void import("../lib/windowSnap").then((m) => {
        snapMod = m;
        void m.startSnap();
      }).catch(() => undefined);
    }, 1200);
    return () => {
      if (snapMod) void snapMod.stopSnap();
    };
  });

  // 吸附特效：卫星窗拖动进入吸附区显示预吸附虚线（tp-snap-preview），吸附命中闪一下发光（tp-snap-flash）。
  // 事件由 windowSnap.ts 在本窗 dispatch，故只在触发的那个卫星窗渲染。
  let snapPreview = $state(false);
  let snapFlash = $state(false);
  let snapFlashTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const onPreview = (e: Event) => { snapPreview = !!(e as CustomEvent<boolean>).detail; };
    const onFlash = () => {
      snapFlash = true;
      clearTimeout(snapFlashTimer);
      snapFlashTimer = setTimeout(() => { snapFlash = false; }, 1000);
    };
    window.addEventListener("tp-snap-preview", onPreview);
    window.addEventListener("tp-snap-flash", onFlash);
    return () => {
      clearTimeout(snapFlashTimer);
      window.removeEventListener("tp-snap-preview", onPreview);
      window.removeEventListener("tp-snap-flash", onFlash);
    };
  });

  // 全局封面刷新监听（C2：注册在 App 根组件，非书库页面内部；根组件常驻，卸载时解绑）。
  // 后端每本扫描完成发 cover-updated{ id }；连续事件在最后一次 300ms 后合并为一次刷新（防抖），
  // 递增 ui.coverTick，书库组件据此重拉当前窗口（按 cover_ver 令 per-book URL 失效）。
  $effect(() => {
    if (standalone) return;
    let tick: ReturnType<typeof setTimeout> | undefined;
    const un = listen<{ id: number }>("cover-updated", () => {
      clearTimeout(tick);
      tick = setTimeout(() => { ui.coverTick++; }, 300);
    });
    return () => {
      clearTimeout(tick);
      un.then((f) => f());
    };
  });

  // F6-H2：独立笔记窗请求主窗打开原书 —— 主窗切到阅读、显示并聚焦。
  $effect(() => {
    if (standalone || standalonePanel || standaloneBook) return;
    const un = listen<{ bookId: number }>(OPEN_BOOK_EVENT, (e) => {
      const id = e.payload?.bookId;
      if (!id) return;
      requestOpenBook(id);
      void (async () => {
        try {
          const { getCurrentWindow } = await import("@tauri-apps/api/window");
          const w = getCurrentWindow();
          await w.show();
          await w.setFocus();
        } catch { /* 聚焦失败不影响打开 */ }
      })();
    });
    return () => {
      un.then((f) => f());
    };
  });

  $effect(() => {
    if (standalone) return;
    // 捕获阶段处理 Esc：弹窗打开时阻止事件继续传播，
    // 否则同一次 Esc 会在关闭弹窗后又触发阅读界面的“返回/关闭”快捷键
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (!ui.settingsOpen && !ui.searchOpen && !ui.statsOpen && !ui.webdavOpen && !convertState.open && !confirmBox.active && !choiceBox.active) return;
      e.stopPropagation();
      if (ui.settingsOpen) ui.settingsOpen = false;
      if (ui.searchOpen) ui.searchOpen = false;
      if (ui.statsOpen) ui.statsOpen = false;
      if (ui.webdavOpen) ui.webdavOpen = false;
      if (convertState.open) { convertState.open = false; convertState.book = null; }
      if (confirmBox.active) confirmAnswer(false);
      if (choiceBox.active) choiceAnswer(null);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  });

  // 独立 AI 窗的尺寸与生命周期标记（tidepage.ai.winw/winh、tidepage.ai.open、ai-window-status）。
  // 守卫必须是 !standalone：只让 AI 窗自己记自己的尺寸，否则写进去的是主窗口的
  // innerWidth/innerHeight，openAiWindow() 会按「主窗口尺寸」创建 AI 窗。
  $effect(() => {
    if (!standalone) return;
    try {
      localStorage.setItem(AI_OPEN_KEY, "1");
    } catch { /* ignore */ }
    const un = listen<{ open: boolean }>("ai-window-status", () => {});
    const save = () => {
      try {
        localStorage.setItem(AI_WIN_W_KEY, String(window.innerWidth));
        localStorage.setItem(AI_WIN_H_KEY, String(window.innerHeight));
      } catch { /* ignore */ }
    };
    const onUnload = () => {
      save();
      try {
        localStorage.setItem(AI_OPEN_KEY, "0");
      } catch { /* ignore */ }
    };
    window.addEventListener("resize", save);
    window.addEventListener("beforeunload", onUnload);
    return () => {
      window.removeEventListener("resize", save);
      window.removeEventListener("beforeunload", onUnload);
      onUnload();
      un.then((f) => f());
    };
  });
</script>

{#if standalone}
  <div class="ai-window-root" style={aiRootStyle}>
    {#await import("../components/AiPanel.svelte")}<div class="comp-loading">{t("common.loading")}</div>{:then { default: AiPanel }}
      <AiPanel standalone />
    {:catch}<div class="comp-loading">{t("boot.dynamicFailed")} <button onclick={() => location.reload()}>{t("boot.retry")}</button></div>{/await}
  </div>
  {#if ui.settingsOpen}
    <div class="overlay" out:outDissolve onpointerdown={(e) => { if (e.target === e.currentTarget) { ui.settingsOpen = false; } }}></div>
    <dialog open out:outFade={{ from: 0.97, up: false }} onclick={(e) => { if (e.target === e.currentTarget) { ui.settingsOpen = false; } }}>
      {#await import("../components/SettingsModal.svelte")}<div class="comp-loading">{t("common.loading")}</div>{:then { default: C }}
        <C initialTab="ai" />
      {:catch}<div class="comp-loading">{t("boot.dynamicFailed")} <button onclick={() => location.reload()}>{t("boot.retry")}</button></div>{/await}
    </dialog>
  {/if}
{/if}
{#if standalonePanel}
  {#await import("../components/StandalonePanel.svelte")}<div class="comp-loading">{t("common.loading")}</div>{:then { default: C }}
    <C />
  {:catch}<div class="comp-loading">{t("boot.dynamicFailed")}</div>{/await}
{:else if standaloneBook}
  {#await import("../components/StandaloneBook.svelte")}<div class="comp-loading">{t("common.loading")}</div>{:then { default: C }}
    <C />
  {:catch}<div class="comp-loading">{t("boot.dynamicFailed")}</div>{/await}
{:else if !standalone}
<div class="layout-root">
  {#if ui.view === "library"}
    <Library />
  {:else}
    {#await import("../components/Reader.svelte")}<div class="comp-loading">{t("common.loading")}</div>{:then { default: C }}
      <C />
    {:catch}<div class="comp-loading">{t("boot.dynamicFailed")} <button onclick={() => location.reload()}>{t("boot.retry")}</button></div>{/await}
  {/if}
</div>
{/if}

{#if !standalone}
  {#if ui.settingsOpen}
    <div class="overlay" out:outDissolve onpointerdown={(e) => { if (e.target === e.currentTarget) { ui.settingsOpen = false; } }}></div>
    <dialog open out:outFade={{ from: 0.97, up: false }} onclick={(e) => { if (e.target === e.currentTarget) { ui.settingsOpen = false; } }}>
      {#await import("../components/SettingsModal.svelte")}<div class="comp-loading">{t("common.loading")}</div>{:then { default: C }}
        <C />
      {:catch}<div class="comp-loading">{t("boot.dynamicFailed")} <button onclick={() => location.reload()}>{t("boot.retry")}</button></div>{/await}
    </dialog>
  {/if}
  {#if ui.searchOpen}
    <div class="overlay" out:outDissolve onpointerdown={(e) => { if (e.target === e.currentTarget) { ui.searchOpen = false; } }}></div>
    <dialog open out:outFade={{ from: 0.97, up: false }} onclick={(e) => { if (e.target === e.currentTarget) { ui.searchOpen = false; } }}>
      {#await import("../components/SearchModal.svelte")}<div class="comp-loading">{t("common.loading")}</div>{:then { default: C }}
        <C />
      {:catch}<div class="comp-loading">{t("boot.dynamicFailed")}</div>{/await}
    </dialog>
  {/if}
  {#if ui.statsOpen}
    <div class="overlay" out:outDissolve onclick={() => (ui.statsOpen = false)}></div>
    <dialog open out:outFade={{ from: 0.97, up: false }} onclick={(e) => { if (e.target === e.currentTarget) { ui.statsOpen = false; } }}>
      {#await import("../components/StatsModal.svelte")}<div class="comp-loading">{t("common.loading")}</div>{:then { default: C }}
        <C />
      {:catch}<div class="comp-loading">{t("boot.dynamicFailed")}</div>{/await}
    </dialog>
  {/if}

  {#if ui.webdavOpen}
    <div class="overlay" out:outDissolve onclick={() => (ui.webdavOpen = false)}></div>
    <div class="wd-drawer" out:outSlideR>
      {#await import("../components/WebdavBrowser.svelte")}<div class="comp-loading">{t("common.loading")}</div>{:then { default: C }}
        <C />
      {:catch}<div class="comp-loading">{t("boot.dynamicFailed")}</div>{/await}
    </div>
  {/if}

  {#if convertState.open}
    <div class="overlay" out:outDissolve onpointerdown={(e) => { if (e.target === e.currentTarget) { convertState.open = false; convertState.book = null; } }}></div>
    <dialog
      open
      out:outFade={{ from: 0.97, up: false }}
      onclick={(e) => { if (e.target === e.currentTarget) { convertState.open = false; convertState.book = null; } }}
    >
      {#if convertState.book}
        {#await import("../components/ConvertModal.svelte")}<div class="comp-loading">{t("common.loading")}</div>{:then { default: C }}
          <C />
        {:catch}<div class="comp-loading">{t("boot.dynamicFailed")}</div>{/await}
      {/if}
    </dialog>
  {/if}

  {#if choiceBox.active}
    <div class="overlay" out:outDissolve onpointerdown={(e) => { if (e.target === e.currentTarget) choiceAnswer(null); }}></div>
    <dialog open out:outFade={{ from: 0.97, up: false }} onclick={(e) => { if (e.target === e.currentTarget) choiceAnswer(null); }}>
      <div class="modal-body choice-body">
        <p class="confirm-msg">{choiceBox.msg}</p>
        {#if choiceBox.actions.some((a) => a.footer)}
          <div class="choice-row choice-stack">
            {#each choiceBox.actions.filter((a) => !a.footer) as a (a.id)}
              <button data-testid={"choice-" + a.id} class={a.kind === "primary" ? "primary" : a.kind === "danger" ? "danger" : ""} onclick={() => choiceAnswer(a.id)}>{a.label}</button>
            {/each}
          </div>
          <div class="confirm-row" style="margin-top:10px">
            {#each choiceBox.actions.filter((a) => a.footer) as a (a.id)}
              <button data-testid={"choice-" + a.id} class={a.kind === "primary" ? "primary" : a.kind === "danger" ? "danger" : ""} onclick={() => choiceAnswer(a.id)}>{a.label}</button>
            {/each}
          </div>
        {:else}
          <div class="choice-row">
            {#each choiceBox.actions as a (a.id)}
              <button data-testid={"choice-" + a.id} class={a.kind === "primary" ? "primary" : a.kind === "danger" ? "danger" : ""} onclick={() => choiceAnswer(a.id)}>{a.label}</button>
            {/each}
          </div>
        {/if}
      </div>
    </dialog>
  {/if}

  {#if ui.notesOpen}
    {#await import("../components/NotesPanel.svelte")}<div class="comp-loading">{t("common.loading")}</div>{:then { default: C }}
      <C />
    {:catch}<div class="comp-loading">{t("boot.dynamicFailed")}</div>{/await}
  {/if}

  {#if confirmBox.active}
    <div class="overlay" out:outDissolve onpointerdown={(e) => { if (e.target === e.currentTarget) { confirmAnswer(false); } }}></div>
    <dialog open out:outFade={{ from: 0.97, up: false }} onclick={(e) => { if (e.target === e.currentTarget) { confirmAnswer(false); } }}>
      <div class="modal-body confirm-body">
        {#if confirmBox.detail?.length}
          <div class="confirm-detail" style={"--cd-cols:" + (1 + Math.max(...confirmBox.detail.map((r) => r.cells.length)))}>
            {#each confirmBox.detail as row (row.label)}
              <span class="cd-label">{row.label}</span>
              {#each row.cells as cell, i (i)}
                <span class="cd-cell" class:end={cell.align === "end"}>{cell.text}</span>
              {/each}
            {/each}
          </div>
        {/if}
        <p class="confirm-msg">{confirmBox.msg}</p>
        <div class="confirm-row">
          <button data-testid="confirm-cancel" onclick={() => confirmAnswer(false)}>{t("common.confirm.cancel")}</button>
          <button class="primary" data-testid="confirm-ok" onclick={() => confirmAnswer(true)}>{t("common.confirm.ok")}</button>
        </div>
      </div>
    </dialog>
  {/if}
{/if}

<!-- 自绘窗口控制按钮：放在所有条件分支之外，保证每种窗口（书库/阅读/独立AI/独立书/面板）都渲染一次 -->
<WindowControls />

<!-- 吸附特效叠层（预吸附虚线 / 吸附命中发光），pointer-events:none 不挡交互 -->
{#if snapPreview || snapFlash}
  <div class="snap-fx" class:preview={snapPreview} class:flash={snapFlash}></div>
{/if}

<div class="tp-toast-wrap">
  <!-- 循环变量改名 ti：避免与 i18n 的 t() 同名遮蔽（t() 依赖 locale，必须在模板里调用） -->
  {#each toasts.items as ti (ti.id)}
    <div class="toast-card" class:error={ti.kind === "error"} out:outFade={{ from: 0.96 }}>
      <div class="toast-msg">{ti.msg}</div>
      <div class="toast-acts">
        <button class="icon" data-testid="toast-copy" title={t("common.toast.copy")} onclick={() => copyText(ti.msg)}><IconCopy /></button>
        <button class="icon" data-testid="toast-close" title={t("common.toast.close")} onclick={() => dismissToast(ti.id)}><IconX /></button>
      </div>
    </div>
  {/each}
</div>

<style>
  .comp-loading { position: fixed; inset: 0; display: grid; place-items: center; gap: 8px; color: var(--fg2); background: var(--bg); z-index: var(--z-modal); font-size: var(--fs-sm); }
  .comp-loading button { padding: 4px 12px; }
  .tp-toast-wrap {
    position: fixed;
    left: 16px;
    bottom: 52px; /* 底栏高度约 30–34px + 间距：通知浮在书库状态栏 / 阅读页底栏之上，不遮挡 */
    display: flex;
    flex-direction: column;
    gap: 8px;
    z-index: var(--z-toast);
    max-width: min(560px, 80vw);
  }

  .toast-card {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    background: var(--panel);
    color: var(--fg);
    border: 1px solid var(--line);
    border-radius: var(--radius-lg);
    padding: 10px 10px 10px 14px;
    box-shadow: var(--shadow-lg), var(--edge-hi);
    animation: toast-in var(--dur-lift) var(--ease-premium) both;
    max-width: 100%;
    min-width: 0;
    box-sizing: border-box;
  }

  .toast-card.error {
    background: var(--danger);
    color: var(--accent-fg);
    border-color: transparent;
  }

  .toast-msg {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-xs);
    line-height: 1.55;
    white-space: pre-wrap;
    word-break: break-all;
    max-height: 38vh;
    overflow-y: auto;
  }

  .toast-acts {
    display: flex;
    gap: 4px;
    flex-shrink: 0;
  }

  .toast-acts button {
    background: var(--hover-bg);
    border: none;
    color: var(--fg);
    padding: 2px 7px;
  }

  .toast-card.error .toast-acts button {
    color: var(--accent-fg);
    background: color-mix(in srgb, var(--accent-fg) 16%, transparent);
  }

  @keyframes toast-in {
    from { opacity: 0; transform: translateY(12px) scale(0.98); }
    to { opacity: 1; transform: none; }
  }
  .layout-root {
    height: 100%;
  }

  /* 吸附特效叠层 */
  .snap-fx {
    position: fixed;
    inset: 0;
    z-index: var(--z-toast);
    pointer-events: none;
    border-radius: var(--radius-sm);
    /* 预览态只改 box-shadow（虚线描边用 box-shadow 画，outline 不可过渡）*/
    transition: box-shadow var(--dur-fast) var(--ease);
  }
  .snap-fx.preview {
    /* 预览态：虚线 outline 不可过渡，改用可插值的实心 inset 描边 */
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  .snap-fx.flash {
    box-shadow: inset 0 0 0 3px var(--accent), inset 0 0 24px color-mix(in srgb, var(--accent) 55%, transparent);
    animation: snap-flash-out 1s ease forwards;
  }
  @keyframes snap-flash-out {
    from { opacity: 1; }
    to { opacity: 0; }
  }

  .overlay {
    position: fixed;
    inset: 0;
    background: var(--scrim);
    backdrop-filter: blur(3px) saturate(1.08);
    -webkit-backdrop-filter: blur(3px) saturate(1.08);
    z-index: var(--z-drawer);
    animation: tp-fade-in var(--dur-scrim) var(--ease-in) both;
  }

 .ai-window-root {
    height: 100%;
    background: var(--panel);
    color: var(--fg);
  }

  /* WebDAV 抽屉：遮罩用 --z-drawer、抽屉本体用 --z-overlay —— 刻意压在 --z-modal 之下，
     这样即使用户先开抽屉再开设置/搜索弹窗，弹窗仍然盖住抽屉（与重构前的 991 < 1000 一致）。
     形态是「顶栏之下的悬浮卡片」（top 让出自绘顶栏，四边留 --sp-2、四角 --radius-xl），
     不再 top:0 贴边满高——那会与右上角自绘窗口按钮同层相撞。 */
  .wd-drawer {
    position: fixed;
    top: var(--tp-titlebar-h, 44px);
    right: var(--sp-2);
    bottom: var(--sp-2);
    width: min(520px, 92vw);
    z-index: var(--z-overlay);
    background: var(--panel);
    color: var(--fg);
    border: 1px solid var(--line);
    border-radius: var(--radius-xl);
    box-shadow: var(--shadow-drawer);
    animation: tp-drawer-in-right var(--dur-lift) var(--ease-premium) both;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

 .ai-window-root :global(.ai-panel) {
    color: var(--fg);
  }

  .confirm-body {
    min-width: 380px;
    max-width: 480px;
  }

  .confirm-msg {
    white-space: pre-line;
    line-height: 1.7;
    margin: 0 0 16px;
    word-break: break-all;
  }

  .confirm-detail {
    display: grid;
    grid-template-columns: repeat(var(--cd-cols, 2), max-content);
    justify-content: start;
    column-gap: 14px;
    row-gap: 4px;
    margin: 0 0 14px;
    line-height: 1.6;
    font-variant-numeric: tabular-nums;
  }

  .confirm-detail .cd-label { color: var(--fg2); }
  .confirm-detail .cd-cell { white-space: nowrap; }
  .confirm-detail .cd-cell.end { text-align: end; }

  .confirm-row {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .choice-body { min-width: 420px; max-width: 560px; }

  .choice-row {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 6px;
  }
  /* 含底部操作的选项框：选项纵向铺满，操作按钮单独一行。 */
  .choice-row.choice-stack {
    flex-direction: column;
    align-items: stretch;
  }
  .choice-row.choice-stack button { text-align: left; }
</style>

