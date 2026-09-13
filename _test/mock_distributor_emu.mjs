// 本地 distributor 仿真器：仅用于自测 probe_distributor.mjs 的请求/解析/判定链路。
// 它不是真实 distributor——通过它得出的任何结论都只代表“工具能正确工作”，不代表服务端能力。
import http from "node:http";
const CHUNK = "OK";
const sse = (res) => res.writeHead(200, { "content-type": "text/event-stream" });
const server = http.createServer(async (req, res) => {
  let raw = ""; req.on("data", (c) => (raw += c));
  req.on("end", async () => {
    const b = (() => { try { return JSON.parse(raw); } catch { return {}; } })();
    const suppressed = b.think === false || req.headers["x-think-enabled"] === "false" || req.url.includes("think=false");
    const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
    if (req.method === "POST" && req.url === "/v1/chat/completions") {
      sse(res);
      if (!suppressed) res.write(`data: ${JSON.stringify({ choices: [{ delta: { reasoning_content: "推理……", content: "" } }] })}\n\n`);
      for (let i = 0; i < 3; i++) { res.write(`data: ${JSON.stringify({ choices: [{ delta: { content: CHUNK } }] })}\n\n`); await sleep(30); }
      res.write("data: [DONE]\n\n"); res.end(); return;
    }
    if (req.method === "POST" && req.url === "/v1/completions") {
      sse(res);
      if (!suppressed) res.write(`data: ${JSON.stringify({ choices: [{ text: "", reasoning: "推理……" }] })}\n\n`);
      for (let i = 0; i < 3; i++) { res.write(`data: ${JSON.stringify({ choices: [{ text: CHUNK }] })}\n\n`); await sleep(30); }
      res.write("data: [DONE]\n\n"); res.end(); return;
    }
    const m = /^\/v1\/models\/([^/]+)\/complete/.exec(req.url);
    if (req.method === "POST" && m) {
      // 模型完整路径（仿真 AI21 风格 NDJSON：{"text":...}）
      res.writeHead(200, { "content-type": "application/x-ndjson" });
      for (let i = 0; i < 3; i++) { res.write(JSON.stringify({ text: CHUNK }) + "\n"); await sleep(30); }
      res.end(); return;
    }
    res.writeHead(404).end();
  });
});
server.listen(18099, "127.0.0.1");
console.log("distributor EMULATOR (self-test only) on 18099");
