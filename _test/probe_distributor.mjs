// distributor 协议边界探针（用户手动执行）：仅发送“回复OK”，绝不携带章节正文；密钥只从环境变量读，不落盘不回显。
// 用法: $env:DIST_BASE="http://x"; $env:DIST_MODEL="TARGET"; $env:DIST_KEY="..."; node _test/probe_distributor.mjs
// 结果决定后续实现：chat=接受 /v1/chat/completions（现有 adapter 够用）；completions/model-complete=需要新增 adapter。
const BASE = (process.env.DIST_BASE || "").replace(/\/+$/, "");
const MODEL = process.env.DIST_MODEL || "";
const KEY = process.env.DIST_KEY || "";
if (!BASE || !MODEL) { console.log("必须设置 DIST_BASE 与 DIST_MODEL（DIST_KEY 可选）；本工具不猜端点。"); process.exit(1); }
if (!/^(https?:\/\/(127\.0\.0\.1|localhost)(:\d+)?|https:\/\/)/i.test(BASE)) { console.log("拒绝：DIST_BASE 必须是 http://127.0.0.1|localhost 或 https。"); process.exit(1); }
const OK = "只回复OK";
const hdrs = { "content-type": "application/json" };
if (KEY) hdrs.authorization = "Bearer <redacted>"; // 实际请求带真 key，日志只记存在
const real = { ...hdrs };
if (KEY) real.authorization = `Bearer ${KEY}`;

async function probe(label, url, body) {
  const t0 = Date.now();
  try {
    const res = await fetch(url, { method: "POST", headers: real, body: JSON.stringify(body) });
    const ct = res.headers.get("content-type") || "";
    let firstData = null, keys = null, textShape = false, deltaShape = false;
    if (/event-stream|ndjson|json/.test(ct)) {
      const raw = await res.text();
      const line = raw.split(/\r?\n/).find((l) => l.startsWith("data:") || l.trim().startsWith("{"));
      try {
        const j = JSON.parse((line || "").replace(/^data:\s*/, ""));
        keys = Object.keys(j);
        deltaShape = !!j?.choices?.[0]?.delta?.content || !!j?.choices?.[0]?.delta;
        textShape = j?.choices?.[0]?.text !== undefined || typeof j?.text === "string" || !!j?.completion;
        firstData = JSON.stringify(j).slice(0, 160);
      } catch { firstData = (line || raw).slice(0, 120); }
    }
    console.log(JSON.stringify({ probe: label, url, status: res.status, ct, deltaShape, textShape, keys, sample: firstData, ms: Date.now() - t0 }));
    return res.status;
  } catch (e) {
    console.log(JSON.stringify({ probe: label, url, error: String(e.message).slice(0, 120) }));
    return -1;
  }
}

// 三候选协议，全部 model 字段=纯模型ID，think 注入只放各自该放的位置
await probe("chat+json-think", `${BASE}/v1/chat/completions`, { model: MODEL, messages: [{ role: "user", content: OK }], stream: true, think: false });
await probe("completions+json-think", `${BASE}/v1/completions`, { model: MODEL, prompt: OK, stream: true, think: false });
await probe("model-path-complete", `${BASE}/v1/models/${encodeURIComponent(MODEL)}/complete`, { prompt: OK, stream: true });
console.log("解读：status=200 且 deltaShape=true → chat adapter 可用(json-think-false 有效)；textShape=true → 需要 completions/model-complete adapter；全部非200 → 保持「自动」并反馈 distributor 文档。");
