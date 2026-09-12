// 本地回环 mock：Ollama /api/chat(NDJSON) 与 OpenAI /v1/chat/completions(SSE)。
// 只返回纯正文帧（无任何 reasoning/thinking 字段），除 model=="mock-thinking"。
// model=="mock-thinking" 时：无视 think:false 仍返回非空 message.thinking（测 off 防线）。
// 请求体 apiKey 正文不落盘（只记长度），Authorization 头只记存在性。
import http from "node:http";
import fs from "node:fs";
import path from "node:path";

const LOG = path.join(process.env.LOCALAPPDATA || "C:\\Users\\A\\AppData\\Local", "Temp", "fr_test_profiles", "mock_reqs.jsonl");
fs.mkdirSync(path.dirname(LOG), { recursive: true });
fs.writeFileSync(LOG, "");
const CHUNK = "，。！？（）《》，。！？（）《》，。！？（）《》，。！？（）《》，。！？（）《》";
const N = 30;

const server = http.createServer((req, res) => {
  let raw = "";
  req.on("data", (c) => (raw += c));
  req.on("end", async () => {
    let body; try { body = JSON.parse(raw); } catch { body = raw; }
    const red = body && typeof body === "object" ? { ...body, messages: (body.messages || []).map((m) => ({ role: m.role, len: (m.content || "").length })), prompt: typeof body.prompt === "string" ? { len: body.prompt.length, head: body.prompt.slice(0, 40) } : body.prompt } : body;
    // 捕获出站形状：完整 path(含 query)、method、白名单 header（不记 Authorization 正文，只记存在）
    const capture = {
      t: Date.now(),
      url: req.url,
      path: req.url,
      method: req.method,
      auth: req.headers.authorization ? "present" : "none",
      thinkHeader: req.headers["x-think-enabled"] ?? null,
      body: red,
    };
    fs.appendFileSync(LOG, JSON.stringify(capture) + "\n");
    const sk = body?.model;
    const thinking = sk === "mock-thinking";
    if (req.url === "/api/chat") {
      res.writeHead(200, { "content-type": "application/x-ndjson", "cache-control": "no-cache" });
      if (thinking) res.write(JSON.stringify({ model: sk, message: { role: "assistant", thinking: "推理中……", content: "" }, done: false }) + "\n");
      for (let i = 0; i < N; i++) {
        res.write(JSON.stringify({ model: sk, message: { role: "assistant", content: CHUNK }, done: false }) + "\n");
        await new Promise((r) => setTimeout(r, 90));
      }
      res.end(JSON.stringify({ model: sk, message: { role: "assistant", content: "" }, done: true }) + "\n");
      return;
    }
    if (req.url === "/v1/chat/completions") {
      res.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache" });
      for (let i = 0; i < N; i++) { res.write(`data: ${JSON.stringify({ choices: [{ delta: { content: CHUNK } }] })}\n\n`); await new Promise((r) => setTimeout(r, 90)); }
      res.write("data: [DONE]\n\n"); res.end();
      return;
    }
    res.writeHead(404).end();
  });
});
const PORT = Number(process.env.FR_MOCK_PORT || 18081);
if (PORT === 9222) throw new Error("mock 端口不得用 9222");
server.listen(PORT, "127.0.0.1");
console.log("mock on", PORT, "log", LOG);
