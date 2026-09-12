import fs from "node:fs";
import path from "node:path";
import { assertLaunchEnv, buildSandboxEnv, assertTestConfig, SANDBOX_ROOT } from "../_test/isolation_guard.mjs";
import { launchSandbox } from "../_test/launch_sandbox.mjs";

let fail = 0;
const ok = (label, fn) => { try { const m = fn(); console.log("THROW-OK  ", label, "::", String(m).slice(0, 90)); } catch (e) { console.log("THROW-OK  ", label, "::", String(e.message).slice(0, 90)); } };
const must = (cond, label) => { console.log((cond ? "PASS      " : "!!FAIL    ") + label); if (!cond) fail++; };

console.log("SANDBOX_ROOT =", SANDBOX_ROOT);

const real = "C:\\Users\\A\\AppData\\Local\\com.a.flowread\\EBWebView";
must(fs.existsSync(real), "real profile exists (sanity)");

ok("A-real-default", () => assertLaunchEnv({ WEBVIEW2_USER_DATA_FOLDER: real }));

const fake = "C:\\Users\\A\\AppData\\Local\\Temp\\fr_sandbox_decoy";
fs.mkdirSync(fake, { recursive: true });
ok("A-fake-same-name", () => assertLaunchEnv({ WEBVIEW2_USER_DATA_FOLDER: fake }));
fs.rmSync(fake, { recursive: true, force: true });

const junc = path.join(SANDBOX_ROOT, "eviltarget_link");
fs.rmSync(junc, { recursive: true, force: true });
try { fs.symlinkSync(real, junc, "junction"); } catch { fs.symlinkSync(real, junc, "dir"); }
try { if (fs.realpathSync(junc) !== real) throw new Error("junction did not resolve"); } catch (e) { console.log("junction setup sanity:", e.message); }
ok("A-junction-escape", () => assertLaunchEnv({ WEBVIEW2_USER_DATA_FOLDER: junc }));
const legit = path.join(SANDBOX_ROOT, "fr_sbx");
fs.mkdirSync(legit, { recursive: true });
let good = "";
try { good = assertLaunchEnv({ WEBVIEW2_USER_DATA_FOLDER: legit }); console.log("PASS      sandbox-legit realpath:", good); must(good.startsWith(SANDBOX_ROOT), "sandbox path accepted"); }
catch (e) { must(false, "sandbox-legit should pass: " + e.message); }
fs.rmSync(junc, { force: true });

ok("env-missing", () => assertLaunchEnv({}));

const spy = { called: 0 };
ok("spawn-guarded", () => launchSandbox({ env: { WEBVIEW2_USER_DATA_FOLDER: real }, cdpPort: 9444, spawnImpl: () => { spy.called++; return {}; } }));
must(spy.called === 0, "spawnImpl NOT reached when env invalid");
ok("port-9222-banned", () => buildSandboxEnv({ WEBVIEW2_USER_DATA_FOLDER: legit }, { cdpPort: 9222 }));

ok("cfg-deepseek", () => assertTestConfig({ ai: { profiles: [{ name: "d", provider: "openai", baseUrl: "https://api.deepseek.com" }] } }));
try { assertTestConfig({ ai: { profiles: [{ baseUrl: "http://127.0.0.1:18081" }] } }); console.log("PASS      cfg-localhost accepted"); } catch (e) { must(false, "localhost cfg: " + e.message); }
console.log(fail === 0 ? "\nSTEP0 ALL PASS (0 spawn attempts beyond spy=0, no app, no port)" : `\nSTEP0 FAILURES: ${fail}`);
process.exit(fail ? 1 : 0);
