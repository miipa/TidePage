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
import type { AiPiece } from "../types";
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

/** 把解析出的语言同步到 <html lang>：影响读屏发音、CSS :lang() 选择器与浏览器的断词/断行规则。
 *  app.html 里写死 lang="en" 只是首帧兜底；这里在语言确定后（含 config.language 被外部改动时）纠正它。 */
function syncHtmlLang(l: Locale): void {
  if (typeof document === "undefined") return;
  if (document.documentElement.lang !== l) document.documentElement.lang = l;
}

// config.language 可能被三处改动：设置面板下拉、跨窗广播 applyConfigJson、外部导入。
// 根 effect 统一收敛到 localeState.locale，保证三者的表现完全一致。
$effect.root(() => {
  $effect(() => {
    const next = resolveLocale(normLanguage(config.language));
    if (next !== localeState.locale) localeState.locale = next;
    syncHtmlLang(localeState.locale);
  });
});

if (typeof document !== "undefined") syncHtmlLang(localeState.locale);

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
 *  前端自己抛出的中文错误（stores/tts）也不进这张表 —— 它们在抛出点直接 t() 生成。
 *  第二类来源是 AiError.code（`ai.*`）：Rust 侧静态中文已搬进文案表，
 *  message 只保留动态尾巴（无固定说法时 code 为空串，整条原样透传）。 */
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
  // AiError.code（src-tauri/src/ai.rs）：静态中文已搬进文案表，message 只留动态尾巴。
  // 一律追加在表尾：这张表按 startsWith 匹配，插在中间可能遮蔽后面的条目。
  // 译文本身已是完整句子（含 {msg} 时由 errText/aiErrDetail 填入后端动态部分）。
  ["ai.requestTimeout", "err.aiRequestTimeout"],
  ["ai.networkFailed", "err.aiNetworkFailed"],
  ["ai.invalidHeaderValue", "err.aiInvalidHeaderValue"],
  ["ai.protocolMismatchBody", "err.aiProtocolMismatchBody"],
  ["ai.protocolMismatchStream", "err.aiProtocolMismatchStream"],
  ["ai.protocolMismatchEmpty", "err.aiProtocolMismatchEmpty"],
  ["ai.dashscopeError", "err.aiDashscopeError"],
  ["ai.audioBase64Decode", "err.aiAudioBase64Decode"],
  ["ai.responseParse", "err.aiResponseParse"],
  ["ai.minimaxFailed", "err.aiMinimaxFailed"],
  ["ai.volcanoError", "err.aiVolcanoError"],
  ["ai.baiduSynthFailed", "err.aiBaiduSynthFailed"],
  ["ai.baiduTokenFailed", "err.aiBaiduTokenFailed"],
  ["ai.edgeBuild", "err.aiEdgeBuild"],
  ["ai.edgeConnect", "err.aiEdgeConnect"],
  ["ai.edgeSend", "err.aiEdgeSend"],
  ["ai.edgeReceive", "err.aiEdgeReceive"],
  ["ai.edgeProxyUnsupported", "err.aiEdgeProxyUnsupported"],
  // davsync.rs 稳定数据同步错误码（G18）；前缀匹配：jsonBroken/notFile 共用文件无效文案。
  ["dataSync.jsonBroken", "err.dataSyncFileInvalid"],
  ["dataSync.notFile", "err.dataSyncFileInvalid"],
  ["dataSync.versionUnsupported", "err.dataSyncVersionUnsupported"],
  ["ai.noApiAddr", "err.aiNoApiAddr"],
  ["ai.invalidModelId", "err.aiInvalidModelId"],
  ["ai.azureNeedDeployment", "err.aiAzureNeedDeployment"],
  ["ai.azureInvalidDeployment", "err.aiAzureInvalidDeployment"],
  ["ai.emptyAudio", "err.aiEmptyAudio"],
  ["ai.dashscopeNeedKey", "err.aiDashscopeNeedKey"],
  ["ai.audioDownloadFailed", "err.aiAudioDownloadFailed"],
  ["ai.audioDownloadEmpty", "err.aiAudioDownloadEmpty"],
  ["ai.dashscopeNoAudio", "err.aiDashscopeNoAudio"],
  ["ai.noClonedVoice", "err.aiNoClonedVoice"],
  ["ai.voiceMissingId", "err.aiVoiceMissingId"],
  ["ai.refAudioReadFailed", "err.aiRefAudioReadFailed"],
  ["ai.refAudioEmpty", "err.aiRefAudioEmpty"],
  ["ai.gatewayNoAudio", "err.aiGatewayNoAudio"],
  ["ai.breezeNoAddr", "err.aiBreezeNoAddr"],
  ["ai.noServiceAddr", "err.aiNoServiceAddr"],
  ["ai.azureVoiceNeedDeployment", "err.aiAzureVoiceNeedDeployment"],
  ["ai.volcanoNeedAppId", "err.aiVolcanoNeedAppId"],
  ["ai.volcanoNeedToken", "err.aiVolcanoNeedToken"],
  ["ai.volcanoNoAudio", "err.aiVolcanoNoAudio"],
  ["ai.baiduNeedKey", "err.aiBaiduNeedKey"],
  ["ai.baiduNeedSecret", "err.aiBaiduNeedSecret"],
  ["ai.baiduEmptyAudio", "err.aiBaiduEmptyAudio"],
  ["ai.edgeNoAudio", "err.aiEdgeNoAudio"],
  // 代理地址（normalize_proxy_url / proxy_with_bypass，src-tauri/src/ai.rs）
  ["ai.proxyAddrEmpty", "err.aiProxyAddrEmpty"],
  ["ai.proxyAddrInvalid", "err.aiProxyAddrInvalid"],
  // 连接测试的结构化片段（AiPiece）：线路名 / 诊断附注 / 代理来源 / 失败行。
  // ⚠ 同一张表按 startsWith 匹配，故**长的排前面**：`ai.routeDirect` 是 `ai.routeDirectNoPac`
  // 与 `ai.routeDirectFallback` 的前缀，插错位置会把它们截断成「直连」。
  ["ai.routeDirectNoPac", "ai.routeDirectNoPac"],
  ["ai.routeDirectFallback", "ai.routeDirectFallback"],
  ["ai.routeCustom", "ai.routeCustom"],
  ["ai.routeDirect", "ai.routeDirect"],
  ["ai.routeAuto", "ai.routeAuto"],
  ["ai.lineFailed", "ai.lineFailed"],
  ["ai.noteProxyUnavailable", "ai.noteProxyUnavailable"],
  ["ai.noteDirectClientFailed", "ai.noteDirectClientFailed"],
  ["ai.noteAutoProxyUnavailable", "ai.noteAutoProxyUnavailable"],
  ["ai.noteSystemProxyDegraded", "ai.noteSystemProxyDegraded"],
  ["ai.notePac", "ai.notePac"],
  ["ai.proxySourceEnv", "ai.proxySourceEnv"],
  ["ai.proxySourceWindows", "ai.proxySourceWindows"],
  ["ai.proxySourcePacOnly", "ai.proxySourcePacOnly"],
  ["ai.proxySourceNone", "ai.proxySourceNone"],
  // 前端 pdf.ts 抛出的稳定 ASCII 码（原生渲染通道 / 超大 PDF）：
  // 这些文案必须在表里，否则用户看到的是错误码本身。
  ["pdf_too_large", "err.pdfTooLarge"],
  ["pdf_native_failed", "err.pdfNativeFailed"],
  ["pdf_native_unsupported", "err.pdfNativeUnsupported"],
  ["pdf_native_abandoned", "err.pdfNativeAbandoned"],
  ["pdf_no_cover", "err.pdfNoCover"],
];

/** toast(String(e), "error") 的本地化替代：已知错误码给译文，未知原样透传。
 *  params 可选：带 `{msg}` 占位符的译文（AiError 的 `ai.*` 码）用它填动态尾巴；
 *  不传时占位符原样保留，故现有调用点无需改动。 */
export function errText(e: unknown, params?: Record<string, string | number>): string {
  // 前端 stores 抛出的错误会带稳定 ASCII `code`（见 stores.svelte.ts 备份入口）：
  // 优先按 code 查表，这样不必把中文原文塞进 Error.message。
  // 调用方没显式给 params 时，从错误对象上取一次（pdf.ts 的 {mb} 这类动态值就走这条）。
  const own = (e as { params?: Record<string, string | number> } | null)?.params;
  const p = params ?? own;
  const code = (e as { code?: unknown } | null)?.code;
  if (typeof code === "string") {
    for (const [c, key] of ERR_CODES) if (code === c || code.startsWith(c)) return t(key, p);
  }
  const raw = String(e);
  for (const [c, key] of ERR_CODES) {
    if (raw === c || raw.startsWith(c)) return t(key);
  }
  return raw;
}

/** `AiError.code` / `AiPiece.code` → 文案键。**精确匹配**：`ai.*` 码互不为前缀，
 *  但 `ai.routeDirect` 是 `ai.routeDirectNoPac` 的前缀，用 startsWith 会把长码截断。 */
function codeKey(code: string): MessageKey | null {
  if (!code) return null;
  for (const [c, key] of ERR_CODES) if (c === code) return key;
  return null;
}

/** AiError 的结构化展示（src-tauri/src/ai.rs 序列化出的 `{kind, code, message}`）。
 *
 *  规则：**code 命中译文时，译文顶替「kind 标签 + 原始 message」两段** —— 因为码的译文
 *  本身就是完整描述（例如「连接超时：{msg}」已含 timeout 这个类别），再叠一层 errKind
 *  会变成「网络错误：网络失败：…」这种重复。
 * @returns 命中时返回整句译文；未命中（无 code / 码不在表里）返回 null，由调用方沿用
 *          原来的 `errKindLabel(kind)` + 原始 message，那条路径一个字都不改。 */
export function aiErrDetail(err: { kind: string; code?: string; message: string }): string | null {
  const key = codeKey(err.code ?? "");
  return key ? t(key, { msg: err.message }) : null;
}

/** 结构化片段（`AiPiece`，src-tauri/src/ai.rs）：连接测试的线路名、诊断附注、失败行都走这里。
 *
*  规则与 Rust 侧一致：`code` 命中 → 取译文并把素材按序填进 `{p0}` / `{p1}`…；素材取自
 *  `params`（本身也是片段，可再嵌套，如「自动代理（环境变量 HTTPS_PROXY）」里的来源），
 *  没有 `params` 时退化成一个素材 = `param`。`code` 为空或不在表里 → 原样回退 `param`
 *  （代理地址、上游返回文本这类没有固定说法的内容，后端不会给码）。
 *  单素材时**同时**提供 `{msg}`：错误码（`err.ai*`）的译文用的是 `{msg}`，与线路/附注的
 *  `{p0}` 共用这张表，两个占位符都填上，两类码都能命中。 */
export function aiPieceText(piece: AiPiece | null | undefined): string {
  if (!piece) return "";
  const slots = piece.params.length ? piece.params.map(aiPieceText) : piece.param ? [piece.param] : [];
  const key = codeKey(piece.code);
  if (!key) return piece.param || slots.join(" ");
  const params: Record<string, string> = {};
  if (!piece.params.length && piece.param) params.msg = piece.param;
  slots.forEach((v, i) => (params[`p${i}`] = v));
  return t(key, params);
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

/** 字典里是否存在 `dict.<kind>.<id>` 这个键（只认 en 表：它是 Record<MessageKey,string> 的全集）。
 *  用途：结构体里「内置条目」与「用户自建条目」混在一起时，显示前先分流——
 *  内置走 labelOfId() 取译文，用户自建的必须原样显示用户自己的名字（绝不查表）。
 *  例：config.readerSchemes 里内置 5 套有 dict.scheme.* 键，用户自建的没有。 */
export function hasLabelOf(kind: string, id: string): boolean {
  return Object.prototype.hasOwnProperty.call(TABLES[FALLBACK_LOCALE], `dict.${kind}.${id}`);
}
