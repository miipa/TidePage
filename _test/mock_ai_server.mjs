// 本地回环 mock（auto 行为）：Ollama /api/chat(NDJSON) 与 OpenAI /v1/chat/completions(SSE)。
// 只返回纯正文帧，无任何 thinking/reasoning 字段。“关闭思考”功能已移除，mock 同步精简。
// 记录出站形状：完整 url(含 query)、method、白名单 header、body 键与脱敏副本（不记 Authorization 正文）。
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
    const red = body && typeof body === "object"
      ? { ...body, messages: (body.messages || []).map((m) => ({ role: m.role, len: (m.content || "").length })) }
      : body;
    fs.appendFileSync(LOG, JSON.stringify({
      t: Date.now(), url: req.url, path: req.url, method: req.method,
      auth: req.headers.authorization ? "present" : "none",
      keys: body && typeof body === "object" ? Object.keys(body) : [],
      body: red,
    }) + "\n");
    if (req.url === "/api/chat") {
      res.writeHead(200, { "content-type": "application/x-ndjson", "cache-control": "no-cache" });
      for (let i = 0; i < N; i++) {
        res.write(JSON.stringify({ model: body?.model, message: { role: "assistant", content: CHUNK }, done: false }) + "\n");
        await new Promise((r) => setTimeout(r, 90));
      }
      res.end(JSON.stringify({ model: body?.model, message: { role: "assistant", content: "" }, done: true }) + "\n");
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
