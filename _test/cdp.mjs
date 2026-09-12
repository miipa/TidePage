// 极简 CDP 客户端（Node 24 内置 WebSocket）。禁止连接端口 9222（历史污染端口）。
const BANNED = "9222";
export async function connectTarget(port, pred) {
  if (String(port) === BANNED) throw new Error(`CDP 端口 ${BANNED} 为历史污染端口，拒绝连接`);
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  let lastErr;
  for (let i = 0; i < 40; i++) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
      const t = list.find(pred);
      if (t) return await open(t);
      lastErr = new Error("target not listed yet");
    } catch (e) { lastErr = e; }
    await sleep(500);
  }
  throw new Error("connectTarget: " + lastErr?.message);
}
async function open(t) {
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const ws = new WebSocket(t.webSocketDebuggerUrl);
  let seq = 0; const pending = new Map();
  await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
  ws.onmessage = ({ data }) => {
    const m = JSON.parse(data); const p = pending.get(m.id); if (!p) return;
    pending.delete(m.id); m.error ? p.reject(new Error(m.error.message)) : p.resolve(m.result);
  };
  const call = (method, params = {}) => new Promise((resolve, reject) => { const id = ++seq; pending.set(id, { resolve, reject }); ws.send(JSON.stringify({ id, method, params })); });
  await call("Runtime.enable");
  async function evalJs(expression) {
    const r = await call("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
    return r.result.value;
  }
  return { evalJs, call, close: () => ws.close(), url: t.url };
}
