// 测试硬隔离守卫 v2 —— fail-closed。
// 沙箱唯一合法位置：仓库外的固定根目录 %LOCALAPPDATA%\Temp\fr_test_profiles
// （必须留在仓库外：vite dev 会监听项目目录树，WebView2 锁文件会让 watcher EBUSY 崩溃）。
// 校验方式：fs.realpathSync（解析 junction/symlink 到真实目标）+ path.relative 后代判断。
// 不再使用任何目录名字符串匹配（同名目录/链接无法绕过）。
import fs from "node:fs";
import path from "node:path";

const SANDBOX_ROOT_RAW = path.join(process.env.LOCALAPPDATA || "C:\\Users\\A\\AppData\\Local", "Temp", "fr_test_profiles");
fs.mkdirSync(SANDBOX_ROOT_RAW, { recursive: true });
export const SANDBOX_ROOT = fs.realpathSync(SANDBOX_ROOT_RAW);

const BANNED_PORT_TOKEN = "9222";

const LOCAL_HOSTS = new Set(["localhost", "127.0.0.1", "::1", "[::1]"]);
export function isLocalEndpoint(url) {
  if (typeof url !== "string" || !url) return false;
  let u;
  try { u = new URL(url); } catch { return false; }
  if (u.protocol !== "http:" && u.protocol !== "https:") return false;
  return LOCAL_HOSTS.has(u.hostname);
}

// (1) 启动应用前必须调用。返回 realpath 后的沙箱路径（调用方必须把返回值放进子进程 env）。
//     任何一步失败立即 throw —— 调用方在 throw 点之后才 spawn，因此失败即零进程零网络。
export function assertLaunchEnv(env = process.env) {
  const raw = env.WEBVIEW2_USER_DATA_FOLDER || "";
  if (!raw) throw new Error(`隔离守卫失败：env.WEBVIEW2_USER_DATA_FOLDER 未设置。测试禁止使用默认/真实 profile。`);
  let real;
  try {
    real = fs.realpathSync(path.resolve(raw));
  } catch {
    throw new Error(`隔离守卫失败：WEBVIEW2_USER_DATA_FOLDER="${raw}" 在文件系统上不存在（realpath 失败），拒绝启动。`);
  }
  const rel = path.relative(SANDBOX_ROOT, real);
  if (rel === "" || rel.startsWith("..") || path.isAbsolute(rel)) {
    throw new Error(`隔离守卫失败："${raw}" → realpath="${real}" 不是沙箱根 "${SANDBOX_ROOT}" 的真实后代（同名目录/junction/symlink/真实 profile 一律拒绝）。`);
  }
  return real;
}

// (2) 构建受守卫的子进程 env：verified 路径必须原样进入 env；CDP 端口必须显式提供且禁止历史污染端口 9222。
export function buildSandboxEnv(env, { cdpPort } = {}) {
  const verified = assertLaunchEnv(env);
  const port = String(cdpPort ?? "");
  if (!/^\d{4,5}$/.test(port)) throw new Error(`隔离守卫失败：必须显式提供 4-5 位 CDP 端口（拒绝隐式/默认）。`);
  if (port === BANNED_PORT_TOKEN) throw new Error(`隔离守卫失败：端口 ${BANNED_PORT_TOKEN} 为历史污染端口，测试禁用。`);
  const out = {
    ...env,
    WEBVIEW2_USER_DATA_FOLDER: verified,
    FR_TEST_CDP_PORT: port,
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
  };
  if (out.WEBVIEW2_USER_DATA_FOLDER !== verified) throw new Error("隔离守卫失败：子进程 env 未携带已验证路径。");
  return out;
}

// (3) 发送/网络触发前：所有 profiles 的 baseUrl 必须是本机回环地址。
export function assertTestConfig(cfgObj) {
  const profiles = cfgObj?.ai?.profiles;
  if (!Array.isArray(profiles) || !profiles.length) {
    throw new Error("隔离守卫失败：配置无 ai.profiles，拒绝继续（profile 状态未知）。");
  }
  for (const p of profiles) {
    if (!isLocalEndpoint(p.baseUrl)) {
      throw new Error(`隔离守卫失败：存在非本机端点 baseUrl="${p.baseUrl}"（name=${p.name}），禁止任何会发请求的操作。`);
    }
  }
}

// (4) 从被测页面实时读 localStorage 复核（不信任内存/不信任调用方传入的 cfg）。
export async function assertPageEndpointIsLocal(pageEval) {
  let raw;
  try {
    raw = await pageEval(`localStorage.getItem("flowread.config.v1") || ""`);
  } catch (e) {
    throw new Error(`隔离守卫失败：无法从页面读取配置，拒绝发送任何 AI 请求：${e.message}`);
  }
  if (!raw) throw new Error("隔离守卫失败：页面 localStorage 无 flowread.config.v1，拒绝发送。");
  const cfg = JSON.parse(raw);
  assertTestConfig(cfg);
  return cfg;
}

// (5) 组合入口：包住任何“可能触发网络”的动作。业务脚本不得绕过本函数直接点击/发送。
export async function withSendGuard(pageEval, action) {
  await assertPageEndpointIsLocal(pageEval);
  return action();
}
