// i18n 运行时（第一阶段：基础设施 + Library 试点）。
//
// 不变量：
//   1) zh-CN.ts 是唯一源；en.ts 用 Record<MessageKey, string> 做编译期全量校验。
//   2) t() 必须经 locale() 读响应式载体，模板里调用才会随语言切换重渲染。
//   3) config.language 只存 "auto" | "zh-CN" | "en"；其余值一律归一为 "auto"（老配置不受影响）。
//   4) 语言相关的日期/数字格式化用 dateLocale()，不再写死 "zh-CN"。
import { zhCN } from "./zh-CN";
import { en } from "./en";
import type { MessageKey } from "./types";
import { config, saveConfig } from "../stores.svelte";

export type { MessageKey } from "./types";

/** 已支持的界面语言。 */
export type Locale = "zh-CN" | "en";
/** 配置项 AppConfig.language 的取值：auto=跟随系统。 */
export type LanguageSetting = "auto" | Locale;

const TABLES: Record<Locale, Record<MessageKey, string>> = { "zh-CN": zhCN, en };

/** 语言不支持时的最终回退目标。 */
export const FALLBACK_LOCALE: Locale = "en";

/** 归一化 AppConfig.language：非法值（老配置里的空串/未来值）一律回落 auto，不崩。 */
export function normLanguage(v: unknown): LanguageSetting {
  return v === "zh-CN" || v === "en" ? v : "auto";
}

/** 系统语言候选列表：navigator.language 优先，再并读 Intl 的 resolved locale（两者可能不一致）。 */
export function systemLocales(): string[] {
  const out: string[] = [];
  try {
    if (typeof navigator !== "undefined" && typeof navigator.language === "string" && navigator.language) {
      out.push(navigator.language);
    }
  } catch {
    /* 非浏览器环境 */
  }
  try {
    const l = Intl.DateTimeFormat().resolvedOptions().locale;
    if (l && !out.includes(l)) out.push(l);
  } catch {
    /* ignore */
  }
  return out;
}

/** auto → 精确匹配 → 只比语言前缀（zh-* → zh-CN，en-* → en）→ 回退 en。非 auto 用用户显式选择。 */
export function resolveLocale(setting: LanguageSetting, sys: readonly string[] = systemLocales()): Locale {
  if (setting !== "auto") return setting;
  for (const raw of sys) {
    if (!raw) continue;
    const tag = raw.toLowerCase();
    if (tag === "zh-cn") return "zh-CN";
    if (tag === "en") return "en";
  }
  for (const raw of sys) {
    const lang = (raw || "").toLowerCase().split(/[-_]/)[0];
    if (lang === "zh") return "zh-CN";
    if (lang === "en") return "en";
  }
  return FALLBACK_LOCALE;
}

/** 当前生效语言的响应式载体。
 *  形态说明：Svelte 编译期禁止从模块导出「会被重新赋值」的 $state，也禁止导出模块级 $derived
 *  （state_invalid_export / derived_invalid_export，均为直接构建失败）。所以把 $state 包在对象里：
 *  只改属性、不重新赋值绑定，才能安全导出；对外用 locale() 读取。
 *  t() 经 locale() 读它即建立响应式依赖 → 模板里调用 t() 会随语言切换重渲染。 */
export const localeState = $state<{ locale: Locale }>({
  locale: resolveLocale(normLanguage(config.language)),
});

/** 当前生效语言（"zh-CN" / "en"）。 */
export function locale(): Locale {
  return localeState.locale;
}

/** 语言设置项：读写 config.language 并落盘。设置面板「界面语言」下拉直接用它。 */
export const localeSetting = {
  get: (): LanguageSetting => normLanguage(config.language),
  set: (v: LanguageSetting) => {
    config.language = normLanguage(v);
    saveConfig();
  },
};

/** 设置面板 onchange 入口（select 只能给 string）：写 config.language + 落盘，
 *  locale 由下方根 effect 立即跟进，无需重启。 */
export function setLocale(v: string) {
  localeSetting.set(normLanguage(v));
}

// config.language 可能被三处改动：设置面板下拉、跨窗广播 applyConfigJson、外部导入。
// 根 effect 统一收敛到 localeState.locale，保证三者的表现完全一致。
$effect.root(() => {
  $effect(() => {
    const next = resolveLocale(normLanguage(config.language));
    if (next !== localeState.locale) localeState.locale = next;
  });
});

/** 日期/数字格式化的 locale：跟随当前语言，禁止在调用点写死 "zh-CN"。 */
export function dateLocale(): Locale {
  return localeState.locale;
}

/** 同 key 只警告一次，避免模板里高频调用把控制台刷爆。 */
const warned = new Set<string>();

const PLACEHOLDER = /\{(\w+)\}/g;

/** 取译文并替换 {name} 占位符。查不到时按「当前语言 → en → 键名」回退。 */
export function t(key: MessageKey, params?: Record<string, string | number>): string {
  const cur = TABLES[locale()];
  let s: string | undefined = cur[key];
  if (s === undefined) s = TABLES[FALLBACK_LOCALE][key];
  if (s === undefined) {
    if (!warned.has(key)) {
      warned.add(key);
      console.warn(`[i18n] 缺少译文：${key}`);
    }
    return key;
  }
  if (!params) return s;
  return s.replace(PLACEHOLDER, (m, name: string) =>
    Object.prototype.hasOwnProperty.call(params, name) ? String(params[name]) : m,
  );
}

/** 已知后端错误码 → 本地化文案。
 *  硬约束：这张表**只收 ASCII 错误码/前缀**（Rust 侧那批 `xxx:N` / `SomeErrorName`）。
 *  Rust 返回的中文报错一律原样透传，绝不靠正则改写后端文案；
 *  前端自己抛出的中文错误（stores/tts）也不进这张表 —— 它们在抛出点直接 t() 生成。 */
const ERR_CODES: ReadonlyArray<readonly [string, MessageKey]> = [
  // db.rs / library.rs 里的稳定 ASCII 错误码（形如 `xxx:N` / `SomeErrorName`）
  ["order_index_too_large", "err.orderIndexTooLarge"],
  ["too_many_ids", "err.tooManyIds"],
  ["MOBIError", "err.mobiParse"],
  ["PalmDocFormatError", "err.palmDoc"],
  ["AWZ3EncryptedError", "err.encrypted"],
  ["ParseTimeoutOrCorrupted", "err.corrupted"],
  ["database is locked", "err.dbLocked"],
  ["database table is locked", "err.dbLocked"],
  ["no such column", "err.dbBadColumn"],
  ["UNIQUE constraint failed", "err.dbUnique"],
  // 前端 stores 抛出的稳定 ASCII 码（备份入口；见 stores.svelte.ts runSettingsBackup）
  ["backup_running", "err.backupRunning"],
  ["backup_need_creds", "err.backupNeedCreds"],
];

/** toast(String(e), "error") 的本地化替代：已知错误码给译文，未知原样透传。 */
export function errText(e: unknown): string {
  // 前端 stores 抛出的错误会带稳定 ASCII `code`（见 stores.svelte.ts 备份入口）：
  // 优先按 code 查表，这样不必把中文原文塞进 Error.message。
  const code = (e as { code?: unknown } | null)?.code;
  if (typeof code === "string") {
    for (const [c, key] of ERR_CODES) if (code === c || code.startsWith(c)) return t(key);
  }
  const raw = String(e);
  for (const [c, key] of ERR_CODES) {
    if (raw === c || raw.startsWith(c)) return t(key);
  }
  return raw;
}

/**
 * 「按 id 查本地化显示名」的唯一入口。
 * 不变量：结构体里持久化的永远是 ASCII `id`（localStorage / config 只存 id），
 * 中文 `name` 是老代码/SettingsModal 的显示源，**不得改动**；译文一律经本函数按 id 取。
 * @param kind 字典域（对应键前缀 `dict.<kind>.`）
 * @param id   结构体的 ASCII id
 */
export function labelOfId(kind: string, id: string): string {
  return t(`dict.${kind}.${id}` as MessageKey);
}
