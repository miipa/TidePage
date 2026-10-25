/**
 * WebDAV 断点同步的「前端落位」决策（纯函数）。
 *
 * 为什么单独成模块：后端 webdav_sync_progress 已经把「本地/远端里较新的一份」合并写库，
 * 前端还要决定界面要不要马上跟着跳。开书时的自动同步是异步的，请求期间用户可能已经开始阅读；
 * 旧实现只看 remote.chapter_index !== chapterNum 就 loadChapter，会把正在读的用户拽走。
 * 这里把判定拆成可单测的五条规则，调用方（Reader.svelte 的 webdavSync）只负责取快照与执行。
 */
import type { Progress } from "./types";

/** 云端进度应如何应用：
 *  - none：无需动作（云端无进度 / 不比本地新 / 章号越界 / 已在同一位置）；
 *  - same-chapter：同章内就地落位，不重载章节；
 *  - load-chapter：异章，按章节恢复；
 *  - notify-only：云端较新但用户已开始阅读，只提示、不导航。 */
export type DavApplyPlan = "none" | "same-chapter" | "load-chapter" | "notify-only";

export interface DavApplyInput {
  /** 云端进度（后端合并后的结果）；null 表示云端无进度。 */
  remote: Pick<Progress, "chapter_index" | "chapter_ratio" | "updated_at"> | null;
  /** 发请求前本地进度的 updated_at；0 表示本地无进度。 */
  localUpdatedAt: number;
  /** 目录长度（章节数）。 */
  chapterCount: number;
  /** 请求返回时的实际章号（不是发请求前的快照）。 */
  currentChapter: number;
  /** 请求返回时的实际章内比例。 */
  currentRatio: number;
  /** 请求期间用户是否已经开始阅读（真实交互）。 */
  userMoved: boolean;
  /** 「同章同位置」判定的章内比例容差，默认 0.002。 */
  ratioEpsilon?: number;
}

/**
 * 按优先级给出应用计划：
 *  1) 云端无进度或 updated_at <= 本地 → none；
 *  2) 章号越界 → none；
 *  3) 同章且比例差 <= 容差 → none（已在目标位置附近）；
 *  4) 用户已移动 → notify-only（不拽走，只提示）；
 *  5) 未移动：同章 same-chapter，异章 load-chapter。
 */
export function planDavApply(input: DavApplyInput): DavApplyPlan {
  const { remote, localUpdatedAt, chapterCount, currentChapter, currentRatio, userMoved } = input;
  const epsilon = input.ratioEpsilon ?? 0.002;
  // 1) 云端没有进度，或不比本地新：没有可应用的内容。
  if (!remote || remote.updated_at <= localUpdatedAt) return "none";
  // 2) 坏数据/书已变（目录变短）：不导航。
  if (remote.chapter_index < 0 || remote.chapter_index >= chapterCount) return "none";
  const sameChapter = remote.chapter_index === currentChapter;
  // 3) 已经是同一章同一位置附近：不必动。
  if (sameChapter && Math.abs(remote.chapter_ratio - currentRatio) <= epsilon) return "none";
  // 4) 用户已开始阅读：自动跳转会把人拽走，只提示。
  if (userMoved) return "notify-only";
  // 5) 用户未移动：同章就地落位，异章按章节恢复。
  return sameChapter ? "same-chapter" : "load-chapter";
}
