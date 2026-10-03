// 种子名本地化：**落盘值恒为中文，只在显示侧查表** —— 与 schemeLabel() / SEED_SCHEME_NAME_KEYS 同一范式。
//
// 为什么不能改落盘值：
//   1) uniqueEndpointName() / uniqueTtsServiceName() 靠中文字面串做去重（names.has("新端点")、新端点{n} 递增），
//      落盘值一旦随语言变化，去重与存量比对同时失效；
//   2) config 是 localStorage 明文 JSON，存量用户的名字已经在那儿，改写等于一次数据迁移。
// 所以：新建端点在英文界面下仍然落盘「新端点」，只是显示成 New endpoint；用户改名成 foo 后显示 foo。
//
// 三段分流（与 schemeLabel() 同形）：
//   空名 → 调用点给的 emptyLabel（沿用既有 `x.name || t(...)` 语义，键不被孤立）；
//   命中种子名 → 查 dict.<kind>.* 译文；
//   其余 → 原样返回用户自己的名字，绝不回落成别的名字。
//
// 硬约束：查表必须以**字面量** kind 调 labelOfId()/hasLabelOf()（见下方注释），
// 否则守卫的「动态拼键入口整族覆盖」识别不到，这批 dict.<kind>.* 会全被当成死键。
import { hasLabelOf, labelOfId, t } from "./i18n/index.svelte";

/** AI 端点种子名 → dict.seedEndpoint.* 后缀。
 *  「新端点2/3…」与时间戳兜底是运行期唯一值，不进静态表，由 numbered 正则 + 插值键覆盖。 */
const SEED_ENDPOINT_NAMES: Record<string, string> = {
  新端点: "plain",
  默认: "legacy",
};

/** TTS 服务种子名 → dict.seedTts.* 后缀。 */
const SEED_TTS_NAMES: Record<string, string> = {
  新服务: "plain",
  默认语音服务: "default",
};

/** AI 提示词种子档名 → dict.seedPrompt.* 后缀（含旧版 customPrompt 迁移出的「自定义」）。
 *  这批名字的用户可编辑性最强：改名后立即原样显示，只有仍是种子名时才查表。 */
const SEED_PROMPT_NAMES: Record<string, string> = {
  剧情总结: "summary",
  翻译为中文: "translate",
  精校润色: "polish",
  人物情节梳理: "characters",
  自定义: "custom",
};

/** AI 端点显示名。输入框场景不传 emptyLabel —— 空名必须保持空，
 *  否则用户一打字就得先删掉兜底文案（既有 `||` 兜底只用于「必须显示名字」的场合）。 */
export function seedEndpointLabel(stored: string, emptyLabel?: string): string {
  const name = (stored || "").trim();
  if (!name) return emptyLabel ?? "";
  const suffix = SEED_ENDPOINT_NAMES[name];
  if (suffix !== undefined && hasLabelOf("seedEndpoint", suffix)) return labelOfId("seedEndpoint", suffix);
  const numbered = /^新端点(\d+)$/.exec(name);
  if (numbered && hasLabelOf("seedEndpoint", "numbered")) return t("dict.seedEndpoint.numbered", { n: numbered[1] });
  return stored;
}

/** TTS 服务显示名（分流规则同上）。 */
export function seedTtsLabel(stored: string, emptyLabel?: string): string {
  const name = (stored || "").trim();
  if (!name) return emptyLabel ?? "";
  const suffix = SEED_TTS_NAMES[name];
  if (suffix !== undefined && hasLabelOf("seedTts", suffix)) return labelOfId("seedTts", suffix);
  const numbered = /^新服务(\d+)$/.exec(name);
  if (numbered && hasLabelOf("seedTts", "numbered")) return t("dict.seedTts.numbered", { n: numbered[1] });
  return stored;
}

/** AI 提示词档名显示名（分流规则同上）。 */
export function seedPromptLabel(stored: string, emptyLabel?: string): string {
  const name = (stored || "").trim();
  if (!name) return emptyLabel ?? "";
  const suffix = SEED_PROMPT_NAMES[name];
  if (suffix !== undefined && hasLabelOf("seedPrompt", suffix)) return labelOfId("seedPrompt", suffix);
  return stored;
}