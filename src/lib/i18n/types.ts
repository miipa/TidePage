import type { zhCN } from "./zh-CN";

/** 全部界面文案键（扁平点号串）。中文表是唯一来源，故直接由 zh-CN.ts 推导。 */
export type MessageKey = keyof typeof zhCN;
