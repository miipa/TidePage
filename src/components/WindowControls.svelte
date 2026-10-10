<!-- 自绘窗口控制按钮：最小化 / 最大化·还原 / 关闭。
     所有窗口类型通用（主窗口 / 独立 AI 窗 / 独立阅读窗 / 面板窗），
     由 +page.svelte 在所有条件分支之外挂载一次，故无需按窗口类型分支。
     无边框窗口拖拽由顶栏的 data-tauri-drag-region 处理，本组件按钮不参与拖拽。 -->
<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { IconMinus, IconSquare, IconSquares, IconX } from "@tabler/icons-svelte";
  import { t } from "../lib/i18n/index.svelte";

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
  <button class="wc-btn" data-testid="wc-min" aria-label={t("common.window.minimize")} title={t("common.window.minimize")} onclick={onMinimize}>
    <IconMinus size={20} stroke={1.5} />
  </button>
  <button
    class="wc-btn"
    data-testid="wc-max"
    aria-label={maximized ? t("common.window.restore") : t("common.window.maximize")}
    title={maximized ? t("common.window.restore") : t("common.window.maximize")}
    onclick={onToggleMaximize}
  >
    {#if maximized}
      <IconSquares size={20} stroke={1.5} />
    {:else}
      <IconSquare size={20} stroke={1.5} />
    {/if}
  </button>
  <button class="wc-btn wc-close" data-testid="wc-close" aria-label={t("common.action.close")} title={t("common.action.close")} onclick={onClose}>
    <IconX size={20} stroke={1.5} />
  </button>
</div>

<style>
  /* 固定右上角覆盖层：容器高度 = 当前窗口顶栏实际高度（--fr-titlebar-h，由承载顶栏的
     组件测高写入），按钮在容器内垂直居中，使按钮中心线与顶栏图标中心线重合、自动对齐。
     z-index 取层级表最高的 --z-window，保证任何时候（连设置弹窗打开时）都能操作窗口。 */
  .win-ctl {
    position: fixed;
    top: 0;
    right: 0;
    height: var(--fr-titlebar-h, 44px);
    z-index: var(--z-window);
    display: flex;
    align-items: center;
    gap: 2px;
    padding-right: 6px;
    user-select: none;
    -webkit-user-select: none;
    /* 容器本身不拦截指针：右侧预留区（如独立 AI 窗标题行的 112px）要能让顶栏的
       drag-region 接住拖拽，否则「可拖面只剩一条缝」。按钮自身恢复 auto，点击/hover 不变。 */
    pointer-events: none;
  }

  /* 与顶栏图标按钮（button.icon / .tb-btn / .lib-tb-btn）看齐：30×30、圆角、图标 20px。
     颜色/静止透明度优先跟阅读主题（Reader 在阅读期间发布 --rdr-wc-fg / --rdr-wc-opacity），
     其余窗口回落外壳 --fg / 0.82；hover 1、底取当前前景色 8%。
     ⚠️ 用专属的 --rdr-wc-fg 而**不是** --rdr-fg：后者被 applyReaderSurfaceVars() 常驻写在
        <html> 上（见 Reader.svelte 的说明），外壳界面里也有值，做不了「是否在阅读」的信号。
        用它当首选色会让暗色外壳下的三个按钮落到近黑纸色上（实测 1.27:1，等于隐形）。 */
  .wc-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--ctl-h);
    height: var(--ctl-h);
    padding: 0;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    /* 阅读时 Reader 会把 --rdr-wc-fg / --rdr-wc-opacity 发布到 :root（见其 $effect），
       这里优先跟阅读主题；书库等其它窗口没有这两个变量，回落外壳 --fg / 0.82。 */
    color: var(--rdr-wc-fg, var(--fg));
    opacity: var(--rdr-wc-opacity, 0.82);
    cursor: pointer;
    pointer-events: auto;
    transition: background var(--dur-fast) var(--ease), color var(--dur-fast) var(--ease), opacity var(--dur-fast) var(--ease), transform var(--dur-fast) var(--ease);
  }

  .wc-btn:hover {
    opacity: 1;
    /* 阅读主题下取 --rdr-wc-fg 8%（与阅读区 hover 同一配方）；外壳下该变量缺失，
       回落成 --fg 8%，即等价于原来的 var(--hover-bg)。 */
    background: color-mix(in srgb, var(--rdr-wc-fg, var(--fg)) 8%, transparent);
  }

  .wc-btn:active {
    transform: scale(0.9);
  }

  .wc-close:hover {
    opacity: 1;
    background: var(--danger);
    color: var(--accent-fg);
  }
</style>
