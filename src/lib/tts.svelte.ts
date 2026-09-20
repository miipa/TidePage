import { config } from "./stores.svelte";

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
export function splitSegments(text: string, maxLen = 160): FlatSegment[] {
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
    // 过长片段按软标点再切
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
let gen = 0;

let onSegmentCb: ((seg: FlatSegment | null, index: number) => void) | null = null;
let onEndCb: (() => void) | null = null;

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
}

export function ttsSegments(): FlatSegment[] {
  return segments;
}

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

function playAt(i: number) {
  if (!supported) return;
  const synth = window.speechSynthesis;
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
  const g = gen;
  u.onend = () => {
    if (g !== gen) return;
    playAt(i + 1);
  };
  u.onerror = () => {
    if (g !== gen) return;
    pageAdvanceGuard();
    playAt(i + 1);
  };
  utter = u;
  synth.speak(u);
}

// 播放中途若浏览器报错（如语音不支持），仍然继续下一句，避免卡死
function pageAdvanceGuard() {
  /* no-op hook，保留给未来错误统计 */
}

function stopState() {
  ttsState.playing = false;
  ttsState.paused = false;
  ttsState.index = -1;
  utter = null;
  onSegmentCb?.(null, -1);
}

export function ttsStart(index = 0) {
  if (!supported || !segments.length) return;
  window.speechSynthesis.cancel();
  gen++;
  ttsState.playing = true;
  ttsState.paused = false;
  playAt(Math.max(0, Math.min(index, segments.length - 1)));
}

export function ttsStop() {
  if (!supported) return;
  gen++;
  window.speechSynthesis.cancel();
  stopState();
}

export function ttsPauseResume() {
  if (!supported || !ttsState.playing) return;
  const synth = window.speechSynthesis;
  if (ttsState.paused) {
    synth.resume();
    ttsState.paused = false;
  } else {
    synth.pause();
    ttsState.paused = true;
  }
}

export function ttsNext() {
  if (!supported || !ttsState.playing) return;
  gen++;
  window.speechSynthesis.cancel();
  playAt(ttsState.index + 1);
}

export function ttsPrev() {
  if (!supported || !ttsState.playing) return;
  gen++;
  window.speechSynthesis.cancel();
  playAt(ttsState.index - 1);
}

export function ttsPlayIndex(i: number) {
  if (!supported) return;
  gen++;
  window.speechSynthesis.cancel();
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
