// 测试硬隔离守卫 —— 任何 CDP/配置/网络测试脚本必须先 import 本模块，
// 在启动应用之前调用 assertLaunchEnv()，在点击任何提示词/发送之前调用
// assertPageEndpointIsLocal(evalJs)。任一条件不满足立即 throw（fail-closed）。
// 禁止 catch 后继续；禁止绕过本模块直接启动应用或点提示词。

const SANDBOX_MARKER = process.env.FR_TEST_SANDBOX || "fr_sandbox";

const LOCAL_ORIGIN = /^https?:\/\/(localhost|127\.0\.0\.1|\.localhost)([:\/]|$)/i;

export function isLocalEndpoint(url) {
  if (typeof url !== "string" || !url) return false;
  let u;
  try { u = new URL(url); } catch { return false; }
  return LOCAL_ORIGIN.test(u.href);
}

// (1) 启动应用前必须调用：WEBVIEW2_USER_DATA_FOLDER 未指向 sandbox 即 throw，
//     应用进程一次都不允许在守卫失败后被 spawn。
export function assertLaunchEnv() {
  const folder = process.env.WEBVIEW2_USER_DATA_FOLDER || "";
  if (!folder) {
    throw new Error(`隔离守卫失败：WEBVIEW2_USER_DATA_FOLDER 未设置。测试禁止使用默认/真实 profile。`);
  }
  const norm = folder.replace(/[\\/]+$/g, "");
  if (!norm.toLowerCase().includes(SANDBOX_MARKER.toLowerCase())) {
    throw new Error(`隔离守卫失败：WEBVIEW2_USER_DATA_FOLDER="${folder}" 不指向 sandbox（必须包含 "${SANDBOX_MARKER}"）。`);
  }
  return norm;
}

// (2) 任何配置写入/网络触发前必须调用：AI profiles 的 baseUrl 必须全部 localhost/127.0.0.1。
export function assertTestConfig(cfgObj) {
  const profiles = cfgObj?.ai?.profiles;
  if (!Array.isArray(profiles) || !profiles.length) {
    throw new Error("隔离守卫失败：配置无 ai.profiles，拒绝继续（可能读到非测试 profile）。");
  }
  for (const p of profiles) {
    if (!isLocalEndpoint(p.baseUrl)) {
      throw new Error(`隔离守卫失败：测试端点 baseUrl="${p.baseUrl}" 不是 localhost/127.0.0.1，禁止任何会发请求的操作。`);
    }
  }
}

// (3) 点击提示词/发送前必须调用：从被测页面实时读 localStorage 配置复核（不信任内存里的值）。
//     pageEval 必须是已连接的 CDP Runtime.evaluate 封装。
export async function assertPageEndpointIsLocal(pageEval) {
  let raw;
  try {
    raw = await pageEval(`localStorage.getItem("flowread.config.v1") || ""`);
  } catch (e) {
    throw new Error(`隔离守卫失败：无法从页面读取配置，拒绝发送任何 AI 请求：${e.message}`);
  }
  if (!raw) throw new Error("隔离守卫失败：页面 localStorage 无 flowread.config.v1，profile 状态未知，拒绝发送。");
  const cfg = JSON.parse(raw);
  assertTestConfig(cfg);
  return cfg;
}

// (4) 组合入口：测试脚本统一用这个包住“点击提示词/触发发送”的动作。
export async function withSendGuard(pageEval, action) {
  await assertPageEndpointIsLocal(pageEval);
  return action();
}
