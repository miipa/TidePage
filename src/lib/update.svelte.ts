// 检查更新（轻量方案）：启动时静默查询 GitHub Releases 的最新版本，有新版则记录到
// `updateState`，由界面显示红点与版本号；全程不弹窗、失败不打扰。
import { api } from "./api";
import { APP_VERSION } from "./stores.svelte";
import { versionNewer } from "./version";

/** 有更新时的最新版本号与下载地址；无更新时为 null。 */
export const updateState = $state<{ latest: string | null; url: string | null }>({
  latest: null,
  url: null,
});

/** `checkForUpdates` 的结果，供调用方决定是否给用户提示。 */
export type UpdateCheckResult =
  | { status: "newer"; latest: string }
  | { status: "latest" }
  | { status: "none" }
  | { status: "error"; err: string };

/** 查询最新版本并同步 `updateState`。启动时静默调用；设置页手动调用后按结果提示。 */
export async function checkForUpdates(): Promise<UpdateCheckResult> {
  try {
    const info = await api.checkUpdate();
    if (!info) {
      updateState.latest = null;
      updateState.url = null;
      return { status: "none" };
    }
    if (versionNewer(info.latest, APP_VERSION)) {
      updateState.latest = info.latest;
      updateState.url = info.url;
      return { status: "newer", latest: info.latest };
    }
    updateState.latest = null;
    updateState.url = null;
    return { status: "latest" };
  } catch (e) {
    return { status: "error", err: String(e) };
  }
}
