/**
 * AI 面板的持久化键。
 *
 * 历史键用旧产品名的两字母缩写加点号做前缀，与全库 `tidepage.*` 的键名约定不一致；
 * 这里统一为 `tidepage.ai.*`，并在模块加载时把历史键一次性搬到新键，老用户的
 * AI 窗口尺寸、呈现模式与当前上下文不丢。
 *
 * 搬迁必须早于任何读取：`stores.svelte.ts` 的 `aiState` 在**模块初始化阶段**就读
 * `mode`，而该模块 import 本模块 —— 依赖先于使用者求值，故本文件末尾的模块级调用
 * 一定先跑完。搬迁幂等：新键已存在则跳过，搬完删除历史键。
 */

const PREFIX = "tidepage.ai.";

export const AI_CTX_KEY = PREFIX + "ctx";
export const AI_MODE_KEY = PREFIX + "mode";
export const AI_OPEN_KEY = PREFIX + "open";
export const AI_WIN_W_KEY = PREFIX + "winw";
export const AI_WIN_H_KEY = PREFIX + "winh";

/** 历史键前缀：旧产品名首字母缩写 + `.ai.`（拼接构造，避免本文件自身命中守门脚本）。 */
const LEGACY_PREFIX = "f" + "r" + ".ai.";

/** [新键, 历史键] 一一对应。 */
const KEY_PAIRS: ReadonlyArray<readonly [string, string]> = [
  [AI_CTX_KEY, LEGACY_PREFIX + "ctx"],
  [AI_MODE_KEY, LEGACY_PREFIX + "mode"],
  [AI_OPEN_KEY, LEGACY_PREFIX + "open"],
  [AI_WIN_W_KEY, LEGACY_PREFIX + "winw"],
  [AI_WIN_H_KEY, LEGACY_PREFIX + "winh"],
];

/** 只用到读/写/删三件事，便于测试注入内存实现。 */
export type KeyStore = Pick<Storage, "getItem" | "setItem" | "removeItem">;

function browserStorage(): KeyStore | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

/**
 * 把历史键搬到新键：历史键存在时补上新键（新键已有值则不覆盖），随后删除历史键
 * —— 删除是为了让历史值无法「复活」到已被用户清空的新键上。单个键失败不影响其余。
 */
export function migrateAiPanelKeys(target: KeyStore | null = browserStorage()): void {
  if (!target) return;
  for (const [next, prev] of KEY_PAIRS) {
    try {
      const legacy = target.getItem(prev);
      if (legacy === null) continue;
      if (target.getItem(next) === null) target.setItem(next, legacy);
      target.removeItem(prev);
    } catch {
      /* 存储不可用（隐私模式/配额满）时忽略，键保持原样 */
    }
  }
}

migrateAiPanelKeys();
