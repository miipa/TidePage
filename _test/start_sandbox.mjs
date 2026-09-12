// 通过受守卫启动器启动沙箱。守卫失败 → 不产生任何进程。
import path from "node:path";
import fs from "node:fs";
import { launchSandbox } from "./launch_sandbox.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";

const sandboxDir = path.join(SANDBOX_ROOT, "fr_sbx");
fs.mkdirSync(sandboxDir, { recursive: true }); // 只允许在已验证的 SANDBOX_ROOT 之下创建
const env = { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sandboxDir };

const { child, env: finalEnv } = launchSandbox({
  env,
  cdpPort: Number(process.env.FR_TEST_CDP_PORT || 9444),
  logFile: path.join(SANDBOX_ROOT, "sandbox_dev.log"),
});
console.log("spawned pid:", child.pid);
console.log("verified WEBVIEW2_USER_DATA_FOLDER in child env:", finalEnv.WEBVIEW2_USER_DATA_FOLDER);
console.log("child env browser args:", finalEnv.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS);
child.unref();
