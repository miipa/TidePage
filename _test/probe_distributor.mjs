// distributor 协议边界探针（用户手动执行）——按规格发 4 条“只回复OK”请求，逐项输出证据。
// 绝不发送章节正文；密钥仅从环境变量读取，输出中只记存在性。
// 用法: $env:DIST_BASE="https://…"; $env:DIST_MODEL="TARGET"; $env:DIST_KEY="…"; node _test/probe_distributor.mjs
const BASE = (process.env.DIST_BASE || "").replace(/\/+$/, "");
const MODEL = (process.env.DIST_MODEL || "").trim();
const KEY = process.env.DIST_KEY || "";
if (!BASE || !MODEL) { console.log("必须设置 DIST_BASE 与 DIST_MODEL（本工具不猜端点/模型）"); process.exit(1); }
if (!/^https:\/\//i.test(BASE) && !/^http:\/\/(127\.0\.0\.1|localhost)(:\d+)?$/i.test(BASE)) { console.log("拒绝：DIST_BASE 必须是 https 或本机 http 回环"); process.exit(1); }
const OK = "只回复OK";
const enc = (s) => encodeURIComponent(s);
const headers = (extra = {}) => {
  const h = { "content-type": "application/json", ...extra };
  if (KEY) h.authorization = "Bearer " + KEY;
  return h;
};
const redactReq = (h, b) => ({
  headers: Object.fromEntries(Object.entries(h).map(([k, v]) => [k, k.toLowerCase() === "authorization" ? (v ? "<present>" : "") : v])),
  body: { ...b, messages: b.messages?.map((m) => ({ role: m.role, len: String(m.content || "").length })), prompt: b.prompt !== undefined ? String(b.prompt).slice(0, 20) : undefined },
});
function shapeOf(v) {
  const top = v && typeof v === "object" ? Object.keys(v) : [];
  const c0 = v?.choices?.[0];
  const c0keys = c0 && typeof c0 === "object" ? Object.keys(c0) : [];
  const contentField =
    typeof c0?.delta?.content === "string" ? "choices[0].delta.content"
      : typeof c0?.text === "string" ? "choices[0].text"
        : typeof v?.text === "string" ? "text"
          : typeof v?.completion === "string" ? "completion"
            : typeof c0?.message?.content === "string" ? "choices[0].message.content" : null;
  const thinking = [c0?.delta?.reasoning_content, c0?.delta?.thinking, v?.thinking, v?.reasoning_content, c0?.thinking]
    .filter((x) => typeof x === "string" && x.length).map((x) => x.slice(0, 24));
  return { top, c0keys, contentField, thinkingPresent: thinking.length > 0, thinkingSample: thinking[0] ?? null };
}
async function readStream(res) {
  const raw = await res.text();
  const lines = raw.split(/\r?\n/).map((l) => l.trim()).filter((l) => l.startsWith("data:") || l.startsWith("{"));
  const objs = [];
  for (const l of lines) { try { objs.push(JSON.parse(l.replace(/^data:\s*/, ""))); } catch { } }
  const doneMarker = lines.some((l) => /DONE/.test(l)) || objs.some((o) => o?.done === true || o?.choices?.[0]?.finish_reason);
  const content = objs.map((o) => { const s = shapeOf(o); const f = s.contentField; if (!f) return ""; const p = f.split(/[.\[\]]+/).filter(Boolean); let cur = o; for (const k of p) cur = cur?.[k]; return typeof cur === "string" ? cur : ""; }).join("");
  const thinking = objs.flatMap((o) => [o?.choices?.[0]?.delta?.reasoning_content, o?.choices?.[0]?.delta?.thinking, o?.choices?.[0]?.reasoning, o?.choices?.[0]?.reasoning_content, o?.thinking, o?.reasoning_content, o?.choices?.[0]?.thinking].filter((x) => typeof x === "string" && x));
  return {
    frames: objs.length, doneClean: doneMarker, contentLen: content.length, contentHead: content.slice(0, 40),
    topFields: objs.length ? shapeOf(objs[0]).top : [], c0Fields: objs.length ? shapeOf(objs[0]).c0keys : [],
    contentField: objs.length ? shapeOf(objs[0]).contentField : null,
    thinkingFrames: thinking.length, thinkingHead: thinking[0]?.slice(0, 24) ?? null,
  };
}
async function probe(label, url, body, extraHeaders) {
  const h = headers(extraHeaders);
  const out = { probe: label, finalUrl: url, request: redactReq(h, body) };
  try {
    const t0 = Date.now();
    const res = await fetch(url, { method: "POST", headers: h, body: JSON.stringify(body) });
    out.status = res.status;
    out.contentType = res.headers.get("content-type");
    const st = await readStream(res);
    Object.assign(out, st);
    out.ms = Date.now() - t0;
  } catch (e) { out.error = String(e.message).slice(0, 140); }
  console.log(JSON.stringify(out));
  return out;
}

const results = {};
results.chat_off = await probe("1 chat+think:false", `${BASE}/v1/chat/completions`, { model: MODEL, messages: [{ role: "user", content: OK }], stream: true, think: false });
results.completions_off = await probe("2 completions+think:false", `${BASE}/v1/completions`, { model: MODEL, prompt: OK, stream: true, think: false });
results.mc_query_off = await probe("3 models/complete?think=false", `${BASE}/v1/models/${enc(MODEL)}/complete?think=false`, { model: MODEL, prompt: OK, stream: true });
results.mc_header_off = await probe("4 models/complete + header", `${BASE}/v1/models/${enc(MODEL)}/complete`, { model: MODEL, prompt: OK, stream: true }, { "X-Think-Enabled": "false" });

// auto/off 对照（对 200+有正文的协议）
for (const [key, r] of Object.entries(results)) {
  if (r.status === 200 && r.contentLen > 0) {
    const k = key.replace("_off", "_auto");
    const base = { model: MODEL, messages: [{ role: "user", content: OK }], stream: true };
    const pbody = { model: MODEL, prompt: OK, stream: true };
    if (key.startsWith("chat")) results[k] = await probe("auto对照 " + k, r.finalUrl, base);
    else if (key.startsWith("completions")) results[k] = await probe("auto对照 " + k, r.finalUrl, pbody);
    else results[k] = await probe("auto对照 " + k, r.finalUrl.replace("?think=false", ""), pbody, key.includes("header") ? { "X-Think-Enabled": "false" } : {});
    const a = results[k], o = r;
    const at = (a.thinkingFrames || 0) > 0, ot = (o.thinkingFrames || 0) > 0;
    const note = !at && !ot
      ? "auto/off 均无 thinking 字段：接口接受该参数，关闭效果未直接证明"
      : at && !ot ? "auto 有 thinking、off 无：参数确实生效" : "off 仍有 thinking：该协议未生效，勿选用";
    console.log(JSON.stringify({ verdict: key, autoThinkingFrames: a.thinkingFrames ?? 0, offThinkingFrames: o.thinkingFrames ?? 0, note }));
  }
}

const acceptedImplemented = [];
if (results.chat_off.status === 200 && (results.chat_off.contentField || "").includes("delta.content")) acceptedImplemented.push("chat-json-think-false");
if (results.completions_off.status === 200 && (results.completions_off.contentField || "") === "choices[0].text") acceptedImplemented.push("completions-json-think-false");
const mcAccepted = [results.mc_query_off, results.mc_header_off].some((r) => r.status === 200 && r.contentLen > 0);
if (acceptedImplemented.length) {
  console.log("FLOWREAD_PROTOCOL=" + JSON.stringify({ baseUrl: BASE, model: MODEL, strategies: acceptedImplemented, at: Date.now() }));
} else {
  console.log("无可用协议（本版本已实现解析器的 chat/completions 均未被接受）。维持「自动」。");
}
if (mcAccepted) console.log("注意：model-complete 被接受，但其响应字段需先人工核对探针输出后为本程序补充解析器——当前版本仍禁用该策略。");
