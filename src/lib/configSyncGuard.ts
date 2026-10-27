/**
 * F9：便携/子窗配置同步保护（纯函数）。
 *
 * 背景：便携模式下 JS 运行期创建的独立窗口可能使用与主窗不同的 WebView2 用户数据目录
 * （Tauri 的 data_directory 只作用于主窗），导致子窗 localStorage 里没有主窗配置；
 * 若子窗此时保存，会以默认配置整体覆写共享的 secret.config 并广播默认配置。
 *
 * 约定：主窗永不抑制；子窗在「标签未解析（含 2s 超时）」或「本地无配置且尚未收到主窗配置」
 * 时抑制写盘/广播；一旦收到主窗配置（或本地本来就有配置）恢复写入。
 */
export interface ConfigSyncState {
  /** webview label 是否已解析（异步解析，2s 超时后按保守处理置 true）。 */
  labelResolved: boolean;
  /** 是否主窗口（label === "main"）。 */
  isMain: boolean;
  /** 本窗 localStorage 是否已有配置。 */
  hasLocalConfig: boolean;
  /** 子窗是否已从主窗拿到配置（或本地已有配置）。 */
  configSynced: boolean;
}

/** 是否应抑制本次配置写盘/广播。 */
export function shouldSuppressConfigWrite(o: ConfigSyncState): boolean {
  if (o.isMain) return false;
  if (!o.labelResolved) return true;
  if (!o.hasLocalConfig && !o.configSynced) return true;
  return false;
}

/** F9：密钥是否已从加密存储回填完成。未完成时不得写 secret.config（否则会用空值覆盖）。 */
export function canWriteSecrets(secretsHydrated: boolean): boolean {
  return secretsHydrated;
}
