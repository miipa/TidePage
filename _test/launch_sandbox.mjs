// 通过受守卫启动器启动沙箱。守卫失败 → 不产生任何进程。
import path from "node:path";
import fs from "node:fs";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { buildSandboxEnv } from "./isolation_guard.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
export const REPO = path.resolve(HERE, "..");

export function launchSandbox({
  env = process.env,
  cdpPort,
  logFile = path.join(path.dirname(process.env.LOCALAPPDATA || ""), "Temp", "fr_test_profiles", "sandbox_dev.log"),
  spawnImpl = spawn,
} = {}) {
  // ↓ 校验失败即 throw，spawnImpl 不可达（必改1#4/#5）
  const finalEnv = buildSandboxEnv(env, { cdpPort });
  // 写批处理到沙箱根内；经 powershell Start-Process(ShellExecute) 分离启动——已实测该方式
  // 能脱离本工具进程树存活；finalEnv（已验证 WEBVIEW2_USER_DATA_FOLDER + 注入端口）全链继承。
  const bat = path.join(path.dirname(logFile), "sandbox_dev.cmd");
  fs.mkdirSync(path.dirname(bat), { recursive: true });
  fs.writeFileSync(bat, `@echo off\r\ncd /d "${REPO}"\r\nnpm.cmd run tauri dev > "${logFile}" 2>&1\r\n`);
  const ps = `Start-Process cmd.exe -WindowStyle Hidden -ArgumentList '/c','"${bat}"'`;
  const encoded = Buffer.from(ps, "utf16le").toString("base64");
  const child = spawnImpl("powershell.exe", ["-NoProfile", "-NonInteractive", "-EncodedCommand", encoded], {
    cwd: REPO,
    env: finalEnv,
    stdio: "ignore",
    windowsHide: true,
  });
  return { child, env: finalEnv, cdp: `http://127.0.0.1:${cdpPort}` };
}
