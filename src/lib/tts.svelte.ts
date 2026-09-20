import { config, toast } from "./stores.svelte";
import { api } from "./api";

export interface FlatSegment {
  /** 用于朗读的文本（已去首尾空白） */
  text: string;
  /** 在章节纯文本中的字符起止偏移（用于高亮定位） */
  start: number;
  end: number;
}

/* ---------------- 文本切句 ---------------- */

const SENT_END = "。！？!?…";
const CLOSERS = "」』”\"'）)】》〉」";
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

export const ttsState = $state({
  supported,
  playing: false,
  paused: false,
  index: -1,
  total: 0,
  voices: [] as SpeechSynthesisVoice[],
});

/** 非响应式：片段数组较大，避免深层代理开销 */
let segments: FlatSegment[] = [];
let utter: SpeechSynthesisUtterance | null = null;
let audioEl: HTMLAudioElement | null = null;
let usingAudio = false;
let gen = 0;

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

interface TtsEndpoint {
  provider: string;
  baseUrl: string;
  apiKey: string;
  model: string;
  deployment: string;
  apiVersion: string;
  proxyMode: string;
  proxyUrl: string;
  tag: string;
}

/** 解析在线语音要用的端点：自定义地址，或某个 AI 端点。 */
function resolveEndpoint(): TtsEndpoint | null {
  if (config.tts.onlineSource === "custom") {
    const base = (config.tts.customBaseUrl || "").trim();
    if (!base) return null;
    return {
      provider: "openai",
      baseUrl: base,
      apiKey: config.tts.customApiKey || "",
      model: config.tts.onlineModel || "tts-1",
      deployment: "",
      apiVersion: "",
      proxyMode: "auto",
      proxyUrl: "",
      tag: `custom:${base}`,
    };
  }
  const list = config.ai.profiles;
  if (!list.length) return null;
  const p =
    list.find((x) => x.id === config.tts.onlineProfileId) ??
    list[Math.min(Math.max(config.ai.active, 0), list.length - 1)] ??
    list[0];
  if (!p) return null;
  return {
    provider: p.provider,
    baseUrl: p.baseUrl,
    apiKey: p.apiKey,
    model: config.tts.onlineModel || p.model,
    deployment: p.deployment,
    apiVersion: p.apiVersion,
    proxyMode: p.proxyMode,
    proxyUrl: p.proxyUrl,
    tag: `profile:${p.id}`,
  };
}

function cacheKey(text: string): string {
  const ep = resolveEndpoint();
  return [
    ep?.tag ?? "none",
    config.tts.onlineModel,
    config.tts.onlineVoice,
    config.tts.onlineFormat,
    text,
  ].join("\u0001");
}

function base64ToObjectUrl(b64: string, fmt: string): string {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  const mime =
    fmt === "wav" ? "audio/wav"
      : fmt === "opus" ? "audio/ogg"
        : fmt === "aac" ? "audio/aac"
          : fmt === "flac" ? "audio/flac"
            : "audio/mpeg";
  return URL.createObjectURL(new Blob([bytes], { type: mime }));
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

async function loadAudio(text: string): Promise<string> {
  const key = cacheKey(text);
  const hit = audioCache.get(key);
  if (hit) return hit;
  const ep = resolveEndpoint();
  if (!ep) throw new Error("未配置在线语音端点");
  const fmt = config.tts.onlineFormat || "mp3";
  const b64 = await api.ttsSynth({
    provider: ep.provider,
    baseUrl: ep.baseUrl,
    apiKey: ep.apiKey,
    model: ep.model,
    deployment: ep.deployment,
    apiVersion: ep.apiVersion,
    proxyMode: ep.proxyMode,
    proxyUrl: ep.proxyUrl,
    voice: config.tts.onlineVoice || "alloy",
    format: fmt,
    text,
  });
  const url = base64ToObjectUrl(b64, fmt);
  putCache(key, url);
  return url;
}

function playAudioUrl(url: string, seg: FlatSegment, i: number, g: number) {
  if (!audioEl) audioEl = new Audio();
  usingAudio = true;
  audioEl.onended = () => {
    if (g !== gen) return;
    playAt(i + 1);
  };
  audioEl.onerror = () => {
    if (g !== gen) return;
    playAt(i + 1);
  };
  audioEl.volume = Math.max(0, Math.min(1, config.tts.volume ?? 1));
  audioEl.playbackRate = Math.max(0.5, Math.min(4, config.tts.rate || 1));
  audioEl.src = url;
  audioEl
    .play()
    .catch(() => {
      if (g !== gen) return;
      // 自动播放被拦截或解码失败：回退系统语音，保证连续朗读
      speakSystem(seg, i, g);
    });
}

async function playOnline(seg: FlatSegment, i: number, g: number) {
  try {
    const url = await loadAudio(seg.text);
    if (g !== gen) return;
    playAudioUrl(url, seg, i, g);
    void prefetch(i + 1, g);
  } catch (e) {
    if (g !== gen) return;
    if (!onlineWarned) {
      onlineWarned = true;
      const msg = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : String(e);
      toast(`在线语音失败，已回退系统语音：${msg}`, "error");
    }
    speakSystem(seg, i, g);
  }
}

async function prefetch(from: number, g: number) {
  // 在线语音较慢：多预取几句，保证播放不断档（RTF<1 时可持续领先）
  const depth = config.tts.engine === "online" ? 6 : 1;
  for (let k = 0; k < depth; k++) {
    const j = from + k;
    if (g !== gen || j >= segments.length) return;
    const text = segments[j].text;
    if (audioCache.has(cacheKey(text))) continue;
    try {
      await loadAudio(text);
    } catch {
      return;
    }
  }
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
  utter = null;
  onSegmentCb?.(null, -1);
}

function haltAudio() {
  if (!audioEl) return;
  try {
    audioEl.onended = null;
    audioEl.onerror = null;
    audioEl.pause();
    audioEl.removeAttribute("src");
    audioEl.load();
  } catch {
    /* ignore */
  }
  usingAudio = false;
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
  if (config.tts.engine === "online" && usingAudio && audioEl) {
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

if (supported) {
  refreshVoices();
  try {
    window.speechSynthesis.onvoiceschanged = () => refreshVoices();
  } catch {
    /* ignore */
  }
}
