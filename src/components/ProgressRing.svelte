<script lang="ts">
  import { IconCheck } from "@tabler/icons-svelte";
  // 封面进度环（Task E）：内联 SVG 圆环 + 环心数字。
  // - full=true（手动已读）：画满环、环心显对勾，不显示真实 read_pct（避免「满环+30%」矛盾）。
  // - 其余：弧度按 pct，环心显 Math.round(pct)。
  // 配色走 CSS 变量：--accent 弧、--panel 底衬，半透明底 + backdrop-filter（参考 .fmt 角标，放左下角）。
  interface Props {
    pct: number;
    /** 手动已读：满环 + ✓，不显示真实进度。 */
    full?: boolean;
    size?: number;
  }
  let { pct, full = false, size = 30 }: Props = $props();

  const R = 15; // viewBox 36 内的半径
  const C = 2 * Math.PI * R;
  const clamped = $derived(full ? 100 : Math.max(0, Math.min(100, pct || 0)));
  const dash = $derived((clamped / 100) * C);
  // 「已读完」用对勾图标而非 ✓ 字形：字形在两套字体下粗细不一致，且会被读屏念出来。
  const label = $derived(full ? "" : String(Math.round(clamped)));
</script>

<span class="ring" class:full style="width:{size}px;height:{size}px;" aria-hidden="true">
  <svg viewBox="0 0 36 36" width={size} height={size}>
    <circle class="track" cx="18" cy="18" r={R} />
    <circle
      class="prog"
      cx="18"
      cy="18"
      r={R}
      stroke-dasharray="{dash} {C}"
      transform="rotate(-90 18 18)"
    />
  </svg>
  {#if full}
    <span class="num check"><IconCheck /></span>
  {:else}
    <span class="num">{label}</span>
  {/if}
</span>

<style>
  .ring {
    position: absolute;
    left: 6px;
    bottom: 6px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-pill);
    /* 实心半透明底衬（不用 backdrop-blur）：进度环随封面渲染在虚拟滚动网格里，
       模糊层会在滚动时对每个可见环持续触发 GPU 重绘、低端机掉帧。
       用较高不透明度的底达到同等「数字清晰不遮封面」效果。 */
    background: color-mix(in srgb, var(--panel) 88%, transparent);
    box-shadow: var(--shadow-sm), inset 0 1px 0 rgba(255, 255, 255, 0.14);
    pointer-events: none;
  }

  svg {
    display: block;
    transform: none;
  }

  .track {
    fill: none;
    stroke: color-mix(in srgb, var(--fg2) 30%, transparent);
    stroke-width: 3.5;
  }

  .prog {
    fill: none;
    stroke: var(--accent);
    stroke-width: 3.5;
    stroke-linecap: round;
    transition: stroke-dasharray var(--dur) var(--ease);
  }

  .ring.full .prog {
    /* 已读满环高亮 */
    stroke: var(--accent);
  }

  .num {
    position: absolute;
    font-size: var(--fs-2xs);
    line-height: 1;
    font-weight: 600;
    color: var(--accent);
    font-variant-numeric: tabular-nums;
  }

  /* 「已读完」的对勾：图标尺寸走 app.css 的全局 .ring .num.check 规则
     （scoped 样式选不中子组件内部的 <svg>） */
  .num.check {
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
</style>
