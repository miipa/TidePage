<script lang="ts">
  // 封面进度环（Task E）：内联 SVG 圆环 + 环心数字。
  // - full=true（手动已读）：画满环、环心显 ✓，不显示真实 read_pct（避免「满环+30%」矛盾）。
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
  const label = $derived(full ? "✓" : String(Math.round(clamped)));
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
  <span class="num" class:check={full}>{label}</span>
</span>

<style>
  .ring {
    position: absolute;
    left: 6px;
    bottom: 6px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-pill, 999px);
    /* 半透明底衬 + 模糊，保证深浅色主题下环心数字都清晰、不遮封面主体 */
    background: color-mix(in srgb, var(--panel) 62%, transparent);
    backdrop-filter: blur(4px);
    -webkit-backdrop-filter: blur(4px);
    box-shadow: var(--shadow-sm);
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
    transition: stroke-dasharray var(--dur, 0.2s) var(--ease);
  }

  .ring.full .prog {
    /* 已读满环高亮 */
    stroke: var(--accent);
  }

  .num {
    position: absolute;
    font-size: var(--fs-2xs, 10px);
    line-height: 1;
    font-weight: 600;
    color: var(--accent);
    font-variant-numeric: tabular-nums;
  }

  .num.check {
    font-size: 13px;
    font-weight: 700;
  }
</style>
