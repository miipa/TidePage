import { activeTtsService, config, saveConfig, toast } from "./stores.svelte";
import { api } from "./api";
import { t } from "./i18n/index.svelte";

export interface FlatSegment {
  /** 用于朗读的文本（已去首尾空白） */
  text: string;
  /** 在章节纯文本中的字符起止偏移（用于高亮定位） */
  start: number;
  end: number;
}

/* ---------------- 文本切句 ---------------- */

// i18n-allow: 算法输入：中文句末标点集合是切句算法的输入，与界面语言无关。
const SENT_END = "。！？!?…";
// i18n-allow: 算法输入：中文闭合标点集合是切句算法的输入，与界面语言无关。
const CLOSERS = "」』”\"'）)】》〉」";
// i18n-allow: 算法输入：中文软断句标点集合是切句算法的输入，与界面语言无关。
const SOFT_BREAK = "，,、；;：:";

/** 把章节纯文本切成适合朗读的句子片段（保留原文偏移）。 */
export function splitSegments(text: string, maxLen = 60): FlatSegment[] {
  const out: FlatSegment[] = [];
  const n = text.length;
  let start = 0;

  const push = (s: number, e: number) => {
    const raw = text.slice(s, e);
    const trimmed = raw.trim();
    if (!trimmed) return;
    const lead = raw.length - raw.trimStart().length;
    const s2 = s + lead;
    const e2 = s2 + trimmed.length;
    if (trimmed.length <= maxLen) {
      out.push({ text: trimmed, start: s2, end: e2 });
      return;
    }
    let segStart = s2;
    for (let i = 0; i < trimmed.length; i++) {
      const isSoft = SOFT_BREAK.includes(trimmed[i]);
      if ((isSoft && i - (segStart - s2) >= 40) || i - (segStart - s2) >= maxLen) {
        out.push({ text: trimmed.slice(segStart - s2, i + 1).trim(), start: segStart, end: s2 + i + 1 });
        segStart = s2 + i + 1;
      }
    }
    if (segStart < e2) out.push({ text: trimmed.slice(segStart - s2).trim(), start: segStart, end: e2 });
  };

  for (let i = 0; i < n; i++) {
    const ch = text[i];
    if (SENT_END.includes(ch)) {
      let j = i + 1;
      while (j < n && CLOSERS.includes(text[j])) j++;
      push(start, j);
      start = j;
      i = j - 1;
    } else if (ch === "\n") {
      push(start, i);
      start = i + 1;
    }
  }
  if (start < n) push(start, n);
  return out;
}

export function segIndexAt(segs: FlatSegment[], offset: number): number {
  for (let i = 0; i < segs.length; i++) {
    if (offset < segs[i].end) return i;
  }
  return Math.max(0, segs.length - 1);
}

/* ---------------- 引擎状态 ---------------- */

const supported = typeof window !== "undefined" && "speechSynthesis" in window;

/** 在线语音服务返回的音色。`type==="profile"` 表示服务端保存的固定克隆音色，
 *  使用该音色可保证整章说话人一致。 */
export interface OnlineVoice {
  id: string;
  name: string;
  type: string;
  language: string;
}

export const ttsState = $state({
  supported,
  playing: false,
  paused: false,
  index: -1,
  total: 0,
  voices: [] as SpeechSynthesisVoice[],
  /** 在线语音：从服务 /v1/voices 读取到的音色列表 */
  onlineVoices: [] as OnlineVoice[],
  onlineVoiceLoading: false,
  /** 在线语音：最近一次读取音色列表的错误（空=无错误） */
  onlineVoiceError: "",
  /** 在线语音：当前句音频尚未就绪（缓冲中） */
  buffering: false,
});

/** 非响应式：片段数组较大，避免深层代理开销 */
let segments: FlatSegment[] = [];
let utter: SpeechSynthesisUtterance | null = null;
let audioEl: HTMLAudioElement | null = null;
let usingAudio = false;
let gen = 0;
let prefetchGen = 0;

let onSegmentCb: ((seg: FlatSegment | null, index: number) => void) | null = null;
let onEndCb: (() => void) | null = null;

const audioCache = new Map<string, string>();
const AUDIO_CACHE_MAX = 60;
let onlineWarned = false;

export function ttsSetCallbacks(cb: {
  onSegment?: (seg: FlatSegment | null, index: number) => void;
  onEnd?: () => void;
}) {
  onSegmentCb = cb.onSegment ?? null;
  onEndCb = cb.onEnd ?? null;
}

export function ttsSetSegments(segs: FlatSegment[]) {
  segments = segs;
  ttsState.total = segs.length;
  clearAudioCache();
}

export function ttsSegments(): FlatSegment[] {
  return segments;
}

export function isOnlineEngine(): boolean {
  return config.tts.engine === "online";
}

/* ---------------- 系统语音 ---------------- */

function resolveVoice(): SpeechSynthesisVoice | null {
  const list = ttsState.voices;
  if (!list.length) return null;
  const want = config.tts.voice;
  if (want) {
    const hit = list.find((v) => v.name === want || v.voiceURI === want);
    if (hit) return hit;
  }
  const lang = (config.tts.lang || "zh-CN").toLowerCase();
  const pre = lang.slice(0, 2);
  return (
    list.find((v) => v.lang && v.lang.toLowerCase().startsWith(pre)) ||
    list.find((v) => v.default) ||
    list[0]
  );
}

function speakSystem(seg: FlatSegment, i: number, g: number) {
  if (!supported) {
    playAt(i + 1);
    return;
  }
  usingAudio = false;
  const synth = window.speechSynthesis;
  const u = new SpeechSynthesisUtterance(seg.text);
  const v = resolveVoice();
  if (v) {
    u.voice = v;
    u.lang = v.lang;
  } else {
    u.lang = config.tts.lang || "zh-CN";
  }
  u.rate = Math.max(0.5, Math.min(2, config.tts.rate || 1));
  u.pitch = Math.max(0, Math.min(2, config.tts.pitch ?? 1));
  u.volume = Math.max(0, Math.min(1, config.tts.volume ?? 1));
  u.onend = () => {
    if (g !== gen) return;
    playAt(i + 1);
  };
  u.onerror = () => {
    if (g !== gen) return;
    playAt(i + 1);
  };
  utter = u;
  synth.speak(u);
}

/* ---------------- 在线语音 ---------------- */

/** OpenAI 标准音色别名：部分克隆服务会将其映射到服务端默认音色，
 *  该默认音色无固定说话人，逐句请求结果可能不一致。 */
const OPENAI_VOICE_ALIASES = new Set([
  "alloy", "echo", "fable", "onyx", "nova", "shimmer", "coral", "sage",
]);

/** 待朗读内容的目标语言名（VoiceStudio 用英文语言名）。优先按正文判断。 */
function targetLanguageName(): string {
  let hasCjk = false;
  let hasLatin = false;
  for (let i = 0; i < segments.length && i < 60; i++) {
    const t = segments[i].text;
    if (/[\u4e00-\u9fff]/.test(t)) hasCjk = true;
    else if (/[a-zA-Z]/.test(t)) hasLatin = true;
  }
  if (hasCjk) return "Chinese";
  const l = (config.tts.lang || "").toLowerCase();
  if (l.startsWith("zh")) return "Chinese";
  if (l.startsWith("ja") || l.startsWith("jp")) return "Japanese";
  if (l.startsWith("ko")) return "Korean";
  if (l.startsWith("fr")) return "French";
  if (l.startsWith("de")) return "German";
  if (l.startsWith("es")) return "Spanish";
  if (hasLatin || l.startsWith("en")) return "English";
  return "English";
}

/** 固定音色的 language 是否匹配目标语言（避免再出现「英文音色读中文」）。 */
function voiceLangMatches(voice: OnlineVoice, target: string): boolean {
  const l = (voice.language || "").toLowerCase();
  if (!l) return false;
  const t = target.toLowerCase();
  if (t === "chinese") return l.includes("chin") || l.startsWith("zh") || l.includes("mandarin");
  if (t === "english") return l.includes("engl") || l.startsWith("en");
  return l.startsWith(t.slice(0, 4));
}

/** 本次请求实际要发送的音色（当前服务档案的音色；固定音色的选择在朗读前由 ensureFixedVoice 完成）。 */
export function effectiveOnlineVoice(): string {
  return (activeTtsService()?.voice || "").trim();
}

interface TtsEndpoint {
  provider: string;
  baseUrl: string;
  apiKey: string;
  secretKey: string;
  appId: string;
  cluster: string;
  model: string;
  voice: string;
  format: string;
  proxyMode: string;
  proxyUrl: string;
  tag: string;
}

/** 解析在线语音要用的端点：当前语音服务配置档案。无地址且非 Edge 返回 null。 */
function resolveEndpoint(): TtsEndpoint | null {
  const s = activeTtsService();
  if (!s) return null;
  const base = (s.baseUrl || "").trim();
  const provider = s.provider || "openai";
  if (!base && provider !== "edge") return null;
  return {
    provider,
    baseUrl: base,
    apiKey: s.apiKey || "",
    secretKey: s.secretKey || "",
    appId: s.appId || "",
    cluster: s.cluster || "",
    model: (s.model || "").trim(),
    voice: (s.voice || "").trim(),
    format: s.format || "mp3",
    proxyMode: "auto",
    proxyUrl: "",
    tag: `service:${s.id}:${provider}:${base}`,
  };
}

/** 在线语音服务是否可用：引擎为在线，且当前档案填了地址或为 Edge（固定接口）。 */
function isCustomVoiceService(): boolean {
  const s = activeTtsService();
  if (!s) return false;
  return config.tts.engine === "online" && (!!(s.baseUrl || "").trim() || s.provider === "edge");
}

function cacheKey(text: string): string {
  const ep = resolveEndpoint();
  return [
    ep?.tag ?? "none",
    ep?.model ?? "",
    effectiveOnlineVoice(),
    ep?.format ?? "",
    text,
  ].join("\u0001");
}

/** 按音频字节魔数判断真实格式，避免服务返回格式与设置不一致导致无法解码。 */
function sniffAudioMime(b: Uint8Array): string {
  if (b.length >= 12 && b[0] === 0x52 && b[1] === 0x49 && b[2] === 0x46 && b[3] === 0x46) return "audio/wav"; // RIFF
  if (b.length >= 4 && b[0] === 0x4f && b[1] === 0x67 && b[2] === 0x67 && b[3] === 0x53) return "audio/ogg"; // OggS
  if (b.length >= 4 && b[0] === 0x66 && b[1] === 0x4c && b[2] === 0x61 && b[3] === 0x43) return "audio/flac"; // fLaC
  if (b.length >= 3 && b[0] === 0x49 && b[1] === 0x44 && b[2] === 0x33) return "audio/mpeg"; // ID3
  if (b.length >= 2 && b[0] === 0xff && (b[1] & 0xe0) === 0xe0) return "audio/mpeg"; // MP3 frame
  if (b.length >= 2 && b[0] === 0xff && (b[1] & 0xf6) === 0xf0) return "audio/aac"; // ADTS
  return "audio/mpeg";
}

function base64ToObjectUrl(b64: string): string {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return URL.createObjectURL(new Blob([bytes], { type: sniffAudioMime(bytes) }));
}

function putCache(key: string, url: string) {
  if (audioCache.size >= AUDIO_CACHE_MAX) {
    const first = audioCache.keys().next().value;
    if (first !== undefined) {
      const old = audioCache.get(first);
      if (old) URL.revokeObjectURL(old);
      audioCache.delete(first);
    }
  }
  audioCache.set(key, url);
}

function clearAudioCache() {
  for (const u of audioCache.values()) URL.revokeObjectURL(u);
  audioCache.clear();
}

/** 同一文本同一参数只发一次请求；并发调用共享同一个 Promise，避免重复合成。 */
const loading = new Map<string, Promise<string>>();

async function fetchAudio(text: string): Promise<string> {
  const ep = resolveEndpoint();
  if (!ep) throw new Error(t("err.ttsNotConfigured"));
  const fmt = ep.format || "mp3";
  const b64 = await api.ttsSynth({
    provider: ep.provider,
    baseUrl: ep.baseUrl,
    apiKey: ep.apiKey,
    secretKey: ep.secretKey,
    appId: ep.appId,
    cluster: ep.cluster,
    model: ep.model,
    voice: effectiveOnlineVoice(),
    format: fmt,
    text,
    lang: config.tts.lang || "",
    proxyMode: ep.proxyMode,
    proxyUrl: ep.proxyUrl,
  });
  return base64ToObjectUrl(b64);
}

function getAudio(text: string): Promise<string> {
  const key = cacheKey(text);
  const hit = audioCache.get(key);
  if (hit) return Promise.resolve(hit);
  let p = loading.get(key);
  if (!p) {
    p = fetchAudio(text)
      .then((url) => {
        putCache(key, url);
        return url;
      })
      .finally(() => {
        loading.delete(key);
      });
    loading.set(key, p);
  }
  return p;
}

/** 在线语音预取：拉起后续若干句，尽量缩短句间空档。用 generation 标记忽略旧会话结果。 */
const PREFETCH_DEPTH = 6;

function prefetchFrom(from: number, g: number) {
  if (config.tts.engine !== "online") return;
  prefetchGen = g;
  for (let k = 0; k < PREFETCH_DEPTH; k++) {
    const j = from + k;
    if (j >= segments.length) break;
    getAudio(segments[j].text).catch(() => {});
  }
}

/** 释放当前 Audio 元素，防止 WebView2 中连续复用导致增益衰减/状态异常。 */
function disposeAudio() {
  if (!audioEl) return;
  try {
    audioEl.onended = null;
    audioEl.onerror = null;
    audioEl.onloadedmetadata = null;
    audioEl.pause();
    audioEl.removeAttribute("src");
    audioEl.load();
  } catch {
    /* ignore */
  }
  audioEl = null;
  usingAudio = false;
}

/** 播放单个 ObjectURL。每句都新建 Audio 元素，并在元数据加载时校验时长。 */
function playAudioUrl(url: string, seg: FlatSegment, i: number, g: number): Promise<void> {
  return new Promise((resolve, reject) => {
    disposeAudio();
    audioEl = new Audio();
    usingAudio = true;
    ttsState.buffering = false;

    let settled = false;
    const cleanup = () => {
      if (settled) return;
      settled = true;
      disposeAudio();
    };

    audioEl.onloadedmetadata = () => {
      if (g !== gen) {
        cleanup();
        resolve();
        return;
      }
      const dur = audioEl?.duration ?? 0;
      if (!audioEl || !isFinite(dur) || dur <= 0.05) {
        cleanup();
        reject(new Error(t("err.audioZeroDuration")));
      }
    };

    audioEl.onended = () => {
      if (g !== gen) {
        cleanup();
        resolve();
        return;
      }
      cleanup();
      playAt(i + 1);
      resolve();
    };

    audioEl.onerror = () => {
      if (g !== gen) {
        cleanup();
        resolve();
        return;
      }
      cleanup();
      // 在线语音解码失败：绝不回退系统语音（会让音色/音量突变），提示并停止。
      reject(new Error(t("err.audioDecode")));
    };

    audioEl.volume = Math.max(0, Math.min(1, config.tts.volume ?? 1));
    audioEl.playbackRate = Math.max(0.5, Math.min(4, config.tts.rate || 1));
    audioEl.src = url;
    audioEl.play().catch((e) => {
      if (g !== gen) {
        cleanup();
        resolve();
        return;
      }
      cleanup();
      reject(e);
    });
  });
}

/** 拉取并缓存单句音频，失败自动重试（最多 3 次）。 */
async function retryAudio(text: string, g: number): Promise<string | null> {
  let lastErr: unknown = null;
  for (let attempt = 0; attempt < 3; attempt++) {
    if (g !== gen) return null;
    try {
      return await getAudio(text);
    } catch (e) {
      lastErr = e;
      if (g !== gen) return null;
      await new Promise((r) => setTimeout(r, 250 * (attempt + 1)));
    }
  }
  if (g !== gen) return null;
  throw lastErr;
}

function failOnline(err: unknown) {
  const msg =
    err && typeof err === "object" && "message" in err
      ? String((err as { message: unknown }).message)
      : String(err);
  if (!onlineWarned) {
    onlineWarned = true;
    toast(t("tts.onlineFailed", { msg }), "error");
  }
  ttsStop();
}

async function playOnline(seg: FlatSegment, i: number, g: number) {
  ttsState.buffering = true;
  // 首次在线朗读前读取音色列表；需要固定音色的服务在此补齐同语言音色。
  if (isCustomVoiceService()) {
    await ensureOnlineVoices();
    if (g !== gen) return;
    await ensureFixedVoice();
    if (g !== gen) return;
  }
  let lastErr: unknown = null;
  for (let attempt = 0; attempt < 3; attempt++) {
    if (g !== gen) return;
    try {
      const url = await retryAudio(seg.text, g);
      if (g !== gen || !url) return;
      await playAudioUrl(url, seg, i, g);
      prefetchFrom(i + 1, g);
      return;
    } catch (e) {
      lastErr = e;
      if (g !== gen) return;
      disposeAudio();
      await new Promise((r) => setTimeout(r, 250 * (attempt + 1)));
    }
  }
  if (g !== gen) return;
  failOnline(lastErr);
}

/* ---------------- 播放控制 ---------------- */

function playAt(i: number) {
  if (i < 0) {
    stopState();
    return;
  }
  if (i >= segments.length) {
    ttsState.playing = false;
    ttsState.paused = false;
    ttsState.index = -1;
    onSegmentCb?.(null, -1);
    onEndCb?.();
    return;
  }
  ttsState.index = i;
  ttsState.paused = false;
  const seg = segments[i];
  onSegmentCb?.(seg, i);
  const g = gen;
  if (config.tts.engine === "online") void playOnline(seg, i, g);
  else speakSystem(seg, i, g);
}

function stopState() {
  ttsState.playing = false;
  ttsState.paused = false;
  ttsState.index = -1;
  ttsState.buffering = false;
  utter = null;
  onSegmentCb?.(null, -1);
}

function haltAudio() {
  disposeAudio();
}

export function ttsStart(index = 0) {
  if (!segments.length) return;
  if (config.tts.engine === "system" && !supported) return;
  window.speechSynthesis?.cancel();
  haltAudio();
  gen++;
  onlineWarned = false;
  ttsState.playing = true;
  ttsState.paused = false;
  playAt(Math.max(0, Math.min(index, segments.length - 1)));
}

export function ttsStop() {
  gen++;
  window.speechSynthesis?.cancel();
  haltAudio();
  stopState();
}

export function ttsPauseResume() {
  if (!ttsState.playing) return;
  if (config.tts.engine === "online") {
    // 缓冲中（当前句音频还没就绪）无法暂停，等音频开始后再操作。
    if (!usingAudio || !audioEl) return;
    if (ttsState.paused) {
      audioEl.play().catch(() => undefined);
      ttsState.paused = false;
    } else {
      audioEl.pause();
      ttsState.paused = true;
    }
    return;
  }
  const synth = window.speechSynthesis;
  if (!synth) return;
  if (ttsState.paused) {
    synth.resume();
    ttsState.paused = false;
  } else {
    synth.pause();
    ttsState.paused = true;
  }
}

export function ttsNext() {
  if (!ttsState.playing) return;
  gen++;
  window.speechSynthesis?.cancel();
  haltAudio();
  playAt(ttsState.index + 1);
}

export function ttsPrev() {
  if (!ttsState.playing) return;
  gen++;
  window.speechSynthesis?.cancel();
  haltAudio();
  playAt(ttsState.index - 1);
}

export function ttsPlayIndex(i: number) {
  gen++;
  window.speechSynthesis?.cancel();
  haltAudio();
  ttsState.playing = true;
  ttsState.paused = false;
  playAt(i);
}

export function refreshVoices() {
  if (!supported) return;
  try {
    ttsState.voices = window.speechSynthesis.getVoices();
  } catch {
    /* ignore */
  }
}

/**
 * 从自定义在线 TTS 服务读取可用音色（OpenAI 兼容走 /v1/audio/voices，
 * Breeze-TTS-2 走 /api/voices）。失败返回空数组并静默。
 */
let voiceListPromise: Promise<OnlineVoice[]> | null = null;

/** 确保在线自定义服务的音色列表已加载（失败静默）。 */
async function ensureOnlineVoices(): Promise<void> {
  if (!isCustomVoiceService()) return;
  if (ttsState.onlineVoices.length) return;
  if (!voiceListPromise) {
    voiceListPromise = refreshOnlineVoices().finally(() => {
      voiceListPromise = null;
    });
  }
  await voiceListPromise;
}

let voiceAutoFixed = false;

/** 只选择「固定音色」，且语言必须匹配正文；绝不拿英文音色顶替中文。 */
function selectFixedVoice(list: OnlineVoice[]) {
  const s = activeTtsService();
  if (!s) return;
  const cur = (s.voice || "").trim();
  const isAlias = !cur || OPENAI_VOICE_ALIASES.has(cur.toLowerCase());
  if (!isAlias) return;
  const profiles = list.filter((v) => v.type === "profile");
  if (!profiles.length) return;
  const target = targetLanguageName();
  const match = profiles.find((v) => voiceLangMatches(v, target));
  if (!match) return; // 没有同语言固定音色：保持现状，由 ensureFixedVoice 创建
  s.voice = match.id;
  saveConfig();
  if (!voiceAutoFixed) {
    voiceAutoFixed = true;
    toast(t("tts.switchedClonedVoice", { name: match.name || match.id }));
  }
}

/** 设计音色的固定描述（VoiceStudio 合法 token），用于无参考音频时创建固定音色。 */
const FIXED_VOICE_INSTRUCT = "female, young adult, moderate pitch";
const DESIGNABLE_LANGS = new Set(["Chinese", "English"]);

let fixedVoicePromise: Promise<boolean> | null = null;

/**
 * 朗读前确保存在与正文同语言的固定音色：
 *  - 当前已是固定音色 → 直接使用；
 *  - 列表中存在同语言固定音色 → 选用；
 *  - VoiceStudio 风格服务且缺少同语言固定音色 → 现场创建 design profile。
 * 不匹配语言时保持现状，避免以英文音色朗读中文。
 */
async function ensureFixedVoice(): Promise<boolean> {
  if (!isCustomVoiceService()) return false;
  const s = activeTtsService();
  if (!s) return false;
  // 固定音色创建仅适用于 OpenAI 兼容（VoiceStudio 风格）服务。
  if (s.provider !== "openai" && s.provider !== "voicestudio") return false;
  const cur = (s.voice || "").trim();
  if (cur && !OPENAI_VOICE_ALIASES.has(cur.toLowerCase())) return true;
  const target = targetLanguageName();
  const match = ttsState.onlineVoices
    .filter((v) => v.type === "profile")
    .find((v) => voiceLangMatches(v, target));
  if (match) {
    s.voice = match.id;
    saveConfig();
    return true;
  }
  // 仅当服务暴露 OpenAI 别名（VoiceStudio 这类支持 design profile 的服务）时才现场创建，
  // 避免对不支持创建固定音色的服务（如本地克隆网关）发出无效请求。
  const voiceStudioLike = ttsState.onlineVoices.some((v) => v.type === "openai_alias");
  if (!voiceStudioLike || !DESIGNABLE_LANGS.has(target)) return false;
  if (!fixedVoicePromise) {
    fixedVoicePromise = (async () => {
      try {
        const base = (s.baseUrl || "").trim();
        const res = await api.ttsEnsureFixedVoice(
          base,
          s.apiKey || "",
          target,
          FIXED_VOICE_INSTRUCT,
        );
        await refreshOnlineVoices();
        s.voice = res.id;
        saveConfig();
        if (res.created) {
          toast(t("tts.createdClonedVoice", { target: target === "Chinese" ? t("tts.lang.zh") : target }));
        }
        return true;
      } catch (e) {
        toast(t("tts.createClonedVoiceFailed", { err: String(e) }), "error");
        return false;
      } finally {
        fixedVoicePromise = null;
      }
    })();
  }
  return fixedVoicePromise;
}

export async function refreshOnlineVoices(): Promise<OnlineVoice[]> {
  const s = activeTtsService();
  const provider = s?.provider || "openai";
  const base = (s?.baseUrl || "").trim();
  // Edge 为固定接口，无需地址即可读取；其余协议需地址。
  if (!isCustomVoiceService() || (!base && provider !== "edge")) {
    ttsState.onlineVoices = [];
    return [];
  }
  ttsState.onlineVoiceLoading = true;
  try {
    const list = await api.ttsVoices(
      provider,
      base,
      s?.apiKey || "",
      s?.secretKey || "",
      s?.appId || "",
      s?.cluster || "",
    );
    ttsState.onlineVoices = list;
    ttsState.onlineVoiceError = list.length ? "" : t("tts.noVoiceList");
    selectFixedVoice(list);
    return list;
  } catch (e) {
    ttsState.onlineVoiceError = e instanceof Error ? e.message : String(e);
    ttsState.onlineVoices = [];
    return [];
  } finally {
    ttsState.onlineVoiceLoading = false;
  }
}

if (supported) {
  refreshVoices();
  try {
    window.speechSynthesis.onvoiceschanged = () => refreshVoices();
  } catch {
    /* ignore */
  }
}

// 在线自定义服务：启动即读取音色列表；缺少同语言固定音色时在开始朗读时补建（见 ensureFixedVoice）。
if (config.tts.engine === "online" && isCustomVoiceService()) {
  void refreshOnlineVoices();
}
