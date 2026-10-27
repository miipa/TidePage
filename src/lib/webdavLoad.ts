/**
 * F5-H2：WebDAV 目录加载的纯判定逻辑。
 *
 * 组件 load() 会并发排队（快速连点多个目录）：只有「序号最新」的请求可以写
 * entries/path/loading/error；失败时保留旧列表（不要把界面清空），路径只在成功时提交。
 */
export interface LoadWrite<T> {
  apply: boolean;
  entries: T[];
  path: string;
}

/** 最新请求成功才应用新列表与新路径；其余情况一律不写（保留旧 entries/path）。 */
export function resolveLoadWrite<T>(
  latestSeq: number,
  reqSeq: number,
  ok: boolean,
  items: T[],
  prevEntries: T[],
  prevPath: string,
  targetPath: string,
): LoadWrite<T> {
  if (reqSeq !== latestSeq || !ok) {
    return { apply: false, entries: prevEntries, path: prevPath };
  }
  return { apply: true, entries: items, path: targetPath };
}

/** F5：WebDAV 配置由未就绪变就绪时，是否需要自动重连一次。
 * 条件：false→true、当前不在加载中、且（从未尝试过或上次失败）；成功过不重复拉取。 */
export function shouldReloadAfterConfig(
  wasReady: boolean,
  isReady: boolean,
  attempted: boolean,
  error: string,
  loading: boolean,
): boolean {
  if (wasReady || !isReady || loading) return false;
  return !attempted || !!error;
}

/** F5：下载按钮/下载入口的并发门：同时下载数未达上限，且同一远端路径不在途。 */
export function downloadGate(
  activeDownloads: number,
  inFlight: ReadonlySet<string>,
  remote: string,
  maxActive = 3,
): boolean {
  return activeDownloads < maxActive && !inFlight.has(remote);
}
