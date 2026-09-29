<!-- 自绘窗口控制按钮：最小化 / 最大化·还原 / 关闭。
     所有窗口类型通用（主窗口 / 独立 AI 窗 / 独立阅读窗 / 面板窗），
     由 +page.svelte 在所有条件分支之外挂载一次，故无需按窗口类型分支。
     无边框窗口拖拽由顶栏的 data-tauri-drag-region 处理，本组件按钮不参与拖拽。 -->
<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { IconMinus, IconSquare, IconSquares, IconX } from "@tabler/icons-svelte";

  const win = getCurrentWindow();
  let maximized = $state(false);

  // 初始态 + 监听窗口尺寸变化（最大化/还原会触发 resize），据此切换最大化/还原图标。
  $effect(() => {
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        maximized = await win.isMaximized();
      } catch { /* ignore */ }
      try {
        unlisten = await win.onResized(async () => {
          try {
            maximized = await win.isMaximized();
          } catch { /* ignore */ }
        });
      } catch { /* ignore */ }
    })();
    return () => unlisten?.();
  });

  async function onMinimize() {
    try {
      await win.minimize();
    } catch { /* ignore */ }
  }

  async function onToggleMaximize() {
    try {
      await win.toggleMaximize();
      maximized = await win.isMaximized();
    } catch { /* ignore */ }
  }

  async function onClose() {
    try {
      await win.close();
    } catch {
      try { window.close(); } catch { /* ignore */ }
    }
  }
</script>

<div class="win-ctl">
  <button class="wc-btn" aria-label="最小化" title="最小化" onclick={onMinimize}>
    <IconMinus size={20} stroke={1.75} />
  </button>
  <button
    class="wc-btn"
    aria-label={maximized ? "还原" : "最大化"}
    title={maximized ? "还原" : "最大化"}
    onclick={onToggleMaximize}
  >
    {#if maximized}
      <IconSquares size={20} stroke={1.75} />
    {:else}
      <IconSquare size={20} stroke={1.75} />
    {/if}
  </button>
  <button class="wc-btn wc-close" aria-label="关闭" title="关闭" onclick={onClose}>
    <IconX size={20} stroke={1.75} />
  </button>
</div>

<style>
  /* 固定右上角覆盖层：容器高度 = 当前窗口顶栏实际高度（--fr-titlebar-h，由承载顶栏的
     组件测高写入），按钮在容器内垂直居中，使按钮中心线与顶栏图标中心线重合、自动对齐。
     z-index 高于一切顶栏与弹层（含 notes 面板 1090、设置弹窗 1100），保证任何时候都能操作窗口。 */
  .win-ctl {
    position: fixed;
    top: 0;
    right: 0;
    height: var(--fr-titlebar-h, 44px);
    z-index: 100000;
    display: flex;
    align-items: center;
    gap: 2px;
    padding-right: 6px;
    user-select: none;
    -webkit-user-select: none;
  }

  /* 与顶栏图标按钮（button.icon / .tb-btn / .lib-tb-btn）看齐：30×30、圆角、图标 20px、
     默认 opacity 0.82，hover 1 + var(--hover-bg)；颜色走主题变量，深浅色都正常。 */
  .wc-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 30px;
    padding: 0;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--fg);
    opacity: 0.82;
    cursor: pointer;
    transition: background 0.12s ease, color 0.12s ease, opacity 0.12s ease;
  }

  .wc-btn:hover {
    opacity: 1;
    background: var(--hover-bg);
  }

  .wc-btn:active {
    transform: scale(0.9);
  }

  .wc-close:hover {
    opacity: 1;
    background: var(--danger);
    color: #fff;
  }
</style>
