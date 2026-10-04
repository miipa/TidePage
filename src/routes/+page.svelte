<script lang="ts">
  import Library from "../components/Library.svelte";
  import Reader from "../components/Reader.svelte";
  import SettingsModal from "../components/SettingsModal.svelte";
  import SearchModal from "../components/SearchModal.svelte";
  import StatsModal from "../components/StatsModal.svelte";
  import ConvertModal from "../components/ConvertModal.svelte";
  import AiPanel from "../components/AiPanel.svelte";
  import WebdavBrowser from "../components/WebdavBrowser.svelte";
  import StandaloneBook from "../components/StandaloneBook.svelte";
  import StandalonePanel from "../components/StandalonePanel.svelte";
  import NotesPanel from "../components/NotesPanel.svelte";
  import WindowControls from "../components/WindowControls.svelte";
  import { confirmAnswer, confirmBox, choiceBox, choiceAnswer, convertState, toasts, ui, copyText, dismissToast, config, THEMES, openPanelWindow } from "../lib/stores.svelte";
  import { t } from "../lib/i18n/index.svelte";
  import { IconArrowsSplit2, IconCopy, IconX } from "@tabler/icons-svelte";
  import { preloadReaderFonts, syncCustomFontFaces } from "../lib/fontfaces";
  import { startSnap, stopSnap } from "../lib/windowSnap";
  import { listen } from "@tauri-apps/api/event";

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

  // 自定义字体 @font-face：所有窗口（主窗口 / 独立阅读 / 独立 AI 窗口）统一注入，
  // 独立 AI 窗口不挂载 Reader，必须在这里也能加载用户安装的字体。
  $effect(() => {
    void config.reader.customFonts;
    syncCustomFontFaces();
  });

  // 启动即预加载设定字体，首次打开书籍时可直接以正确字体呈现（消除字体闪烁）
  $effect(() => {
    void config.reader.customFonts;
    void config.reader.fontFamily;
    void config.reader.latinFont;
    void config.reader.fontSize;
    preloadReaderFonts();
  });

  $effect(() => {
    if (standalone) return;
    listen("ai-dock", () => {
      ui.aiOpen = true;
    });
  });

  // 窗口吸附体系：每个窗口启动一次（模块内按 label 自动区分基准窗/卫星窗），卸载时清理监听与组引用。
  $effect(() => {
    void startSnap();
    return () => { void stopSnap(); };
  });

  // 吸附特效：卫星窗拖动进入吸附区显示预吸附虚线（fr-snap-preview），吸附命中闪一下发光（fr-snap-flash）。
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
    window.addEventListener("fr-snap-preview", onPreview);
    window.addEventListener("fr-snap-flash", onFlash);
    return () => {
      clearTimeout(snapFlashTimer);
      window.removeEventListener("fr-snap-preview", onPreview);
      window.removeEventListener("fr-snap-flash", onFlash);
    };
  });

  // 全局封面刷新监听（C2：注册在 App 根组件，非书库页面内部；根组件常驻，卸载时解绑）。
  // 后端每本扫描完成发 cover-updated{ id }；取 e.payload.id 记录最近更新的书，
  // 并节流递增 ui.coverTick，书库组件据此重拉当前窗口（按 cover_ver 令 per-book URL 失效）。
  $effect(() => {
    if (standalone) return;
    let tick: ReturnType<typeof setTimeout> | undefined;
    const un = listen<{ id: number }>("cover-updated", (e) => {
      const id = e.payload?.id;          // 后端 payload = { "id": id }
      if (typeof id === "number") ui.lastCoverId = id;
      clearTimeout(tick);
      tick = setTimeout(() => { ui.coverTick++; }, 300);
    });
    return () => {
      clearTimeout(tick);
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

  $effect(() => {
    if (standalone) return;
    try {
      localStorage.setItem("fr.ai.open", "1");
    } catch { /* ignore */ }
    const un = listen<{ open: boolean }>("ai-window-status", () => {});
    const save = () => {
      try {
        localStorage.setItem("fr.ai.winw", String(window.innerWidth));
        localStorage.setItem("fr.ai.winh", String(window.innerHeight));
      } catch { /* ignore */ }
    };
    const onUnload = () => {
      save();
      try {
        localStorage.setItem("fr.ai.open", "0");
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
    <AiPanel standalone />
  </div>
  {#if ui.settingsOpen}
    <div class="overlay" onpointerdown={(e) => { if (e.target === e.currentTarget) { ui.settingsOpen = false; } }}></div>
    <dialog open onclick={(e) => { if (e.target === e.currentTarget) { ui.settingsOpen = false; } }}>
      <SettingsModal initialTab="ai" />
    </dialog>
  {/if}
{/if}
{#if standalonePanel}
  <StandalonePanel />
{:else if standaloneBook}
  <StandaloneBook />
{:else if !standalone}
<div class="layout-root">
  {#if ui.view === "library"}
    <Library />
  {:else}
    <Reader />
  {/if}
</div>
{/if}

{#if !standalone}
  {#if ui.settingsOpen}
    <div class="overlay" onpointerdown={(e) => { if (e.target === e.currentTarget) { ui.settingsOpen = false; } }}></div>
    <dialog open onclick={(e) => { if (e.target === e.currentTarget) { ui.settingsOpen = false; } }}>
      <SettingsModal />
    </dialog>
  {/if}
  {#if ui.searchOpen}
    <div class="overlay" onpointerdown={(e) => { if (e.target === e.currentTarget) { ui.searchOpen = false; } }}></div>
    <dialog open onclick={(e) => { if (e.target === e.currentTarget) { ui.searchOpen = false; } }}>
      <SearchModal />
    </dialog>
  {/if}
  {#if ui.statsOpen}
    <div class="overlay" onclick={() => (ui.statsOpen = false)}></div>
    <dialog open onclick={(e) => { if (e.target === e.currentTarget) { ui.statsOpen = false; } }}>
      <StatsModal />
    </dialog>
  {/if}

  {#if ui.webdavOpen}
    <div class="overlay" onclick={() => (ui.webdavOpen = false)}></div>
    <div class="wd-drawer">
      <WebdavBrowser />
    </div>
  {/if}

  {#if convertState.open}
    <div class="overlay" onpointerdown={(e) => { if (e.target === e.currentTarget) { convertState.open = false; convertState.book = null; } }}></div>
    <dialog
      open
      onclick={(e) => { if (e.target === e.currentTarget) { convertState.open = false; convertState.book = null; } }}
    >
      {#if convertState.book}
        <ConvertModal />
      {/if}
    </dialog>
  {/if}

  {#if choiceBox.active}
    <div class="overlay" onpointerdown={(e) => { if (e.target === e.currentTarget) choiceAnswer(null); }}></div>
    <dialog open onclick={(e) => { if (e.target === e.currentTarget) choiceAnswer(null); }}>
      <div class="modal-body choice-body">
        <p class="confirm-msg">{choiceBox.msg}</p>
        <div class="choice-row">
          {#each choiceBox.actions as a (a.id)}
            <button data-testid={"choice-" + a.id} class={a.kind === "primary" ? "primary" : a.kind === "danger" ? "danger" : ""} onclick={() => choiceAnswer(a.id)}>{a.label}</button>
          {/each}
        </div>
      </div>
    </dialog>
  {/if}

  {#if ui.notesOpen}
    <!-- 笔记面板弹出入口：置于 NotesPanel 之外（不改其组件本体），成功后关闭内联笔记面板 -->
    <button
      class="notes-popout-btn"
      title={t("notes.popoutTip")}
      onclick={async () => { const ok = await openPanelWindow("notes", ui.currentBookId || 0, t("library.topbar.notes.label")); if (ok) ui.notesOpen = false; }}
    ><IconArrowsSplit2 /></button>
    <NotesPanel />
  {/if}

  {#if confirmBox.active}
    <div class="overlay" onpointerdown={(e) => { if (e.target === e.currentTarget) { confirmAnswer(false); } }}></div>
    <dialog open onclick={(e) => { if (e.target === e.currentTarget) { confirmAnswer(false); } }}>
      <div class="modal-body confirm-body">
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

<div class="fr-toast-wrap">
  <!-- 循环变量改名 ti：避免与 i18n 的 t() 同名遮蔽（t() 依赖 locale，必须在模板里调用） -->
  {#each toasts.items as ti (ti.id)}
    <div class="toast-card" class:error={ti.kind === "error"}>
      <div class="toast-msg">{ti.msg}</div>
      <div class="toast-acts">
        <button class="icon" data-testid="toast-copy" title={t("common.toast.copy")} onclick={() => copyText(ti.msg)}><IconCopy /></button>
        <button class="icon" data-testid="toast-close" title={t("common.toast.close")} onclick={() => dismissToast(ti.id)}><IconX /></button>
      </div>
    </div>
  {/each}
</div>

<style>
  .fr-toast-wrap {
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

  .notes-popout-btn {
    position: fixed;
    top: 10px;
    right: 56px;
    z-index: var(--z-window);
    width: 30px;
    height: 30px;
    border-radius: var(--radius-md);
    background: rgba(128, 128, 128, 0.18);
    border: none;
    cursor: pointer;
    font-size: 15px;
  }
  .notes-popout-btn:hover { background: rgba(128, 128, 128, 0.32); }

  .overlay {
    position: fixed;
    inset: 0;
    background: var(--scrim);
    backdrop-filter: blur(3px) saturate(1.08);
    -webkit-backdrop-filter: blur(3px) saturate(1.08);
    z-index: var(--z-drawer);
    animation: fr-fade-in var(--dur-scrim) var(--ease-in) both;
  }

 .ai-window-root {
    height: 100%;
    background: var(--panel);
    color: var(--fg);
  }

  /* WebDAV 抽屉：遮罩用 --z-drawer、抽屉本体用 --z-overlay —— 刻意压在 --z-modal 之下，
     这样即使用户先开抽屉再开设置/搜索弹窗，弹窗仍然盖住抽屉（与重构前的 991 < 1000 一致）。

     位置：从「贴右边、顶到底的整幅面板」改成「让开顶栏、四边留白的浮动面板」。
     ① top 让开顶栏：--fr-titlebar-h 由 Library/Reader 的 bind:clientHeight 实测写入 <html>，
        fallback 44px 覆盖首帧还没测到值的那一瞬（独立窗不挂顶栏时也吃这个兜底）。
        此前只有 WindowControls 消费过它做自身高度，浮层拿它做 top 偏移是第一次 ——
        少了这一步，抽屉从视口 y=0 起铺满，右侧那 520px 内的顶栏被完全盖住，
        视觉上就读成了「顶栏的延伸」而不是「浮在书库之上的一块面板」。
     ② 右/下留边（走 --sp-2，不写裸 px）+ --radius-xl：读作悬浮卡片；
        投影沿用 --shadow-drawer（这仍是 drawer 档，不换 --shadow-lg）。 */
  .wd-drawer {
    position: fixed;
    top: var(--fr-titlebar-h, 44px);
    right: var(--sp-2);
    bottom: var(--sp-2);
    width: min(520px, 92vw);
    z-index: var(--z-overlay);
    background: var(--panel);
    color: var(--fg);
    border: 1px solid var(--line);
    border-radius: var(--radius-xl);
    box-shadow: var(--shadow-drawer);
    animation: fr-drawer-in-right var(--dur-lift) var(--ease-premium) both;
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
</style>
