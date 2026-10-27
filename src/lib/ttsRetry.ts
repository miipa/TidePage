// TTS 合成/播放重试与会话匹配的纯决策函数。
// 背景：旧实现 retryAudio 内层 3 次 × playOnline 外层 3 次 = 同一句合成最坏 9 次，
// 既放大请求又拖长失败反馈。合成统一最多 2 次尝试；播放/解码失败保持独立有限重试
// （不把网络播放失败变成一次就放弃），二者不再相乘。

export const SYNTH_MAX_ATTEMPTS = 2;
export const PLAYBACK_MAX_ATTEMPTS = 2;

/** attempt 从 0 开始；返回是否允许再发起一次合成请求。非法值保守拒绝。 */
export function synthAttemptsAllowed(attempt: number, limit: number = SYNTH_MAX_ATTEMPTS): boolean {
  if (!Number.isFinite(attempt) || !Number.isFinite(limit)) return false;
  return Math.floor(attempt) >= 0 && Math.floor(attempt) < Math.floor(limit);
}

/** 播放/解码失败的重试上限（与合成次数独立，避免叠乘）。 */
export function playbackAttemptsAllowed(attempt: number, limit: number = PLAYBACK_MAX_ATTEMPTS): boolean {
  if (!Number.isFinite(attempt) || !Number.isFinite(limit)) return false;
  return Math.floor(attempt) >= 0 && Math.floor(attempt) < Math.floor(limit);
}

/** 会话代际是否仍有效（迟到回调/旧预取不得触碰新会话状态）。 */
export function sessionMatches(g: number, gen: number): boolean {
  return Number.isInteger(g) && Number.isInteger(gen) && g === gen;
}
