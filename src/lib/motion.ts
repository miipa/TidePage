/**
 * 退场动效的统一来源。
 *
 * 为什么需要它：此前全应用只有「入场」（各组件用 CSS @keyframes 写在自己的
 * <style> 里），没有退场 —— 抽屉 340ms 滑入、0ms 消失，菜单、弹窗、面板一律如此。
 * 入场有重量、退场是硬切，读起来像元素被拔掉而不是被送走，开关的因果关系不闭合。
 *
 * 为什么放在 lib 而不是各组件自写：
 *   1) 时长/曲线必须与 app.css 的 --dur-exit / --ease-exit 同源，否则某处手滑
 *      写回 340ms 就又出现「进得慢、出得更快」的失配（这正是原状）；
 *   2) reduced-motion 的判定在这里只做一次，全应用语义一致；
 *   3) 只动 transform / opacity，不触发布局 —— 与 app.css 的性能纪律同源。
 *
 * 用法：<div class="overlay" out:outDissolve>；抽屉类挂在包裹元素上
 * （Reader.svelte 的 .panel-slot 承载层就是为此存在）。
 * **不能挂在组件上** —— out: 只接受元素，挂在 <Panel /> 上会报
 * component_invalid_directive。面板组件的根元素在它自己内部，父级拿不到。
 * **只声明 out:，绝不用 transition:** —— transition: 是双向的，Svelte 会用同一个
 * 配置再跑一遍 intro，而入场早已由各组件自己的 CSS @keyframes 承担。两套机制叠在一起
 * 时 WAAPI 会压过 CSS animation（实测），表现为「元素先淡到看不见、再在 intro 结束那一刻
 * 弹回原样」。入场交给 CSS，退场交给这里，两边各管一段。
 *
 * ── Svelte transition 的 t 约定（本文件唯一的方向依据，改公式前必读）──
 *   css(t) 里的 t：**1 = 元素原貌（可见、在位），0 = 消失**。
 *   outro 期间 t 从 1 走到 0，intro 期间从 0 走到 1。官方 fade 就是
 *   `opacity: ${t}`，fly 是 `(1-t)*y`（outro 从 0 位移走向 y）。
 *   本文件历史版本把 opacity 写成 `1-t`（当成「进度 0→1」用），方向整个反了：
 *   退场第一帧就归零、末帧回到 1，实测采样 opacity 0.000 → 0.926 后节点被卸载，
 *   读起来是「瞬间消失 + 160ms 的幽灵淡入再被拔掉」；入场方向同样反。
 *   位移项 `dx * t` 因为在 t=1（outro 起点）取到满位移，方向本来是对的 —— 这也正是
 *   上一轮探针只看到「有插值」就放行的原因。**别再只验证「有动画」，要验证方向。**
 */

import { cubicOut, cubicIn } from "svelte/easing";
import { prefersReducedMotion } from "./reducedMotion";

/** 与 app.css 的 --dur-exit 同值（0.16s）。 */
const EXIT_MS = 160;

/** 加速离开：起步慢、加速走（与入场的 --ease-premium 互为镜像）。 */
function easeOutAccel(t: number): number {
  return cubicIn(t);
}

/** 减速收尾：给「本来就在原地」的浮层用（遮罩、就地展开的提示）。 */
function easeOut(t: number): number {
  return cubicOut(t);
}

/** reduced-motion 下时长归零：退场立即完成，等价于原来的硬切，但不写两套模板。 */
function dur(ms: number): number {
  return prefersReducedMotion() ? 0 : ms;
}

export interface MotionOpts {
  /** 水平位移量（px），退场方向为正 = 向右。正数用于右侧抽屉。 */
  dx?: number;
  /** 起始缩放，退场时向该值收拢。1 表示不缩放。 */
  from?: number;
  /** 位移方向：true = 向上收（菜单/浮层的来路是按钮），false = 向下落。 */
  up?: boolean;
}

/* ------------------------------------------------------------------ */
/* 通用退场                                                            */
/* ------------------------------------------------------------------ */

/** 通用浮层退场：反向位移 + 收拢 + 淡出。用于菜单、下拉、轻量浮层。
 *  形参走 Svelte 的 (node, params) 约定，故既能 out:outFade 直接引用，
 *  也能 out:outFade={{ from: 0.96 }} 传参覆盖。 */
export function outFade(node: Element, o: MotionOpts = {}) {
  const dx = o.dx ?? 0;
  const from = o.from ?? 0.97;
  const dy = o.up === false ? 4 : -4;
  return {
    duration: dur(EXIT_MS),
    easing: easeOutAccel,
    // p = 退场进度（0=原貌，1=离开画面）；t 是 Svelte 的反向约定（1=原貌），故 p=1-t。
    // 用 p 表达后：起点 = 元素当前的静置态（不位移、满不透明），终点 = dx/dy/from，
    // 既方向正确，也消除了「关闭瞬间先跳到偏移位再滑回」的一帧跳变。
    css: (t: number) => {
      const p = 1 - t;
      return `opacity: ${1 - p}; transform: translate(${dx * p}px, ${dy * p}px) scale(${1 - (1 - from) * p});`;
    },
  };
}

/** 就地淡出：只降不透明度，不位移。用于遮罩、就地出现的提示条。
 *  ⚠️ 退场里绝不要给「靠 transform 居中」的元素（tts-bar / tp-note-tip）用带 transform 的退场，
 *     只能用本函数 —— 否则居中位移会被退场的 transform 覆盖而跳走。 */
export function outDissolve(node: Element) {
  return {
    duration: dur(EXIT_MS),
    easing: easeOut,
    css: (t: number) => `opacity: ${t};`,
  };
}

/* ------------------------------------------------------------------ */
/* 抽屉 / 侧面板                                                       */
/* ------------------------------------------------------------------ */

/** 右侧抽屉退场。与 app.css 的 tp-drawer-in-right 严格对位。
 *  入场 24px、退场 20px，略短一点：出画时用户已经开始期待下一个动作，
 *  不需要跑完全程（这是 --dur-exit 取入场一半的同一个理由）。 */
export function outSlideR(node: Element) {
  return {
    duration: dur(EXIT_MS),
    easing: easeOutAccel,
    css: (t: number) => {
      const p = 1 - t;
      return `opacity: ${1 - p}; transform: translateX(${20 * p}px);`;
    },
  };
}

/** 左侧抽屉退场（目录）。与 tp-drawer-in-left 对位。 */
export function outSlideL(node: Element) {
  return {
    duration: dur(EXIT_MS),
    easing: easeOutAccel,
    css: (t: number) => {
      const p = 1 - t;
      return `opacity: ${1 - p}; transform: translateX(${-20 * p}px);`;
    },
  };
}
