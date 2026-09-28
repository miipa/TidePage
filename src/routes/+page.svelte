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
  import { confirmAnswer, confirmBox, choiceBox, choiceAnswer, convertState, toasts, ui, copyText, dismissToast, config, THEMES, openPanelWindow } from "../lib/stores.svelte";
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
            <button class={a.kind === "primary" ? "primary" : a.kind === "danger" ? "danger" : ""} onclick={() => choiceAnswer(a.id)}>{a.label}</button>
          {/each}
        </div>
      </div>
    </dialog>
  {/if}

  {#if ui.notesOpen}
    <!-- 笔记面板弹出入口：置于 NotesPanel 之外（不改其组件本体），成功后关闭内联笔记面板 -->
    <button
      class="notes-popout-btn"
      title="弹出为独立窗口"
      onclick={async () => { const ok = await openPanelWindow("notes", ui.currentBookId || 0, "笔记"); if (ok) ui.notesOpen = false; }}
    >⧉</button>
    <NotesPanel />
  {/if}

  {#if confirmBox.active}
    <div class="overlay" onpointerdown={(e) => { if (e.target === e.currentTarget) { confirmAnswer(false); } }}></div>
    <dialog open onclick={(e) => { if (e.target === e.currentTarget) { confirmAnswer(false); } }}>
      <div class="modal-body confirm-body">
        <p class="confirm-msg">{confirmBox.msg}</p>
        <div class="confirm-row">
          <button onclick={() => confirmAnswer(false)}>取消</button>
          <button class="primary" onclick={() => confirmAnswer(true)}>确定</button>
        </div>
      </div>
    </dialog>
  {/if}
{/if}

<!-- 吸附特效叠层（预吸附虚线 / 吸附命中发光），pointer-events:none 不挡交互 -->
{#if snapPreview || snapFlash}
  <div class="snap-fx" class:preview={snapPreview} class:flash={snapFlash}></div>
{/if}

<div class="fr-toast-wrap">
  {#each toasts.items as t (t.id)}
    <div class="toast-card" class:error={t.kind === "error"}>
      <div class="toast-msg">{t.msg}</div>
      <div class="toast-acts">
        <button class="icon" title="复制" onclick={() => copyText(t.msg)}>⧉</button>
        <button class="icon" title="关闭" onclick={() => dismissToast(t.id)}>×</button>
      </div>
    </div>
  {/each}
</div>

<style>
  .fr-toast-wrap {
    position: fixed;
    left: 16px;
    bottom: 16px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    z-index: 9999;
    max-width: min(560px, 80vw);
  }

  .toast-card {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    background: #26282cf2;
    color: #fff;
    border-radius: 12px;
    padding: 10px 10px 10px 14px;
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.22), 0 18px 44px rgba(0, 0, 0, 0.32), inset 0 1px 0 rgba(255, 255, 255, 0.08);
    animation: toast-in 0.34s cubic-bezier(0.32, 0.72, 0, 1) both;
    max-width: 100%;
    min-width: 0;
    box-sizing: border-box;
  }

  .toast-card.error {
    background: #8f3a34ee;
  }

  .toast-msg {
    flex: 1;
    min-width: 0;
    font-size: 12.5px;
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
    background: rgba(255, 255, 255, 0.12);
    border: none;
    color: #fff;
    padding: 2px 7px;
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
    z-index: 10001;
    pointer-events: none;
    border-radius: 6px;
  }
  .snap-fx.preview {
    outline: 2px dashed var(--accent, #4a6b8a);
    outline-offset: -6px;
    box-shadow: inset 0 0 0 6px color-mix(in srgb, var(--accent, #4a6b8a) 14%, transparent);
  }
  .snap-fx.flash {
    outline: 3px solid var(--accent, #4a6b8a);
    outline-offset: -3px;
    box-shadow: inset 0 0 24px color-mix(in srgb, var(--accent, #4a6b8a) 55%, transparent);
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
    z-index: 10000;
    width: 30px;
    height: 30px;
    border-radius: 8px;
    background: rgba(128, 128, 128, 0.18);
    border: none;
    cursor: pointer;
    font-size: 15px;
  }
  .notes-popout-btn:hover { background: rgba(128, 128, 128, 0.32); }

  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(20, 20, 24, 0.35);
    backdrop-filter: blur(3px) saturate(1.08);
    -webkit-backdrop-filter: blur(3px) saturate(1.08);
    z-index: 990;
    animation: overlay-in 0.24s cubic-bezier(0, 0, 0.2, 1) both;
  }

  @keyframes overlay-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .ai-window-root {
    height: 100%;
    background: var(--panel);
    color: var(--fg);
  }

  .wd-drawer {
    position: fixed;
    top: 0;
    right: 0;
    bottom: 0;
    width: min(520px, 92vw);
    z-index: 991;
    background: var(--panel);
    color: var(--fg);
    border-left: 1px solid var(--line);
    box-shadow: -10px 0 32px rgba(0, 0, 0, 0.22);
    animation: wd-in 0.36s cubic-bezier(0.32, 0.72, 0, 1) both;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  @keyframes wd-in {
    from { transform: translateX(24px); opacity: 0; }
    to { transform: translateX(0); opacity: 1; }
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
