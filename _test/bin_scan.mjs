// 二进制字符串扫描：needles × ascii/utf16le + SHA-256。用法: node bin_scan.mjs <file...>
import fs from "node:fs";
import crypto from "node:crypto";
const NEEDLES = ["FR_TEST_CDP_PORT", "--remote-debugging-port", "--remote-debugging-address", "remote-debugging-pipe"];
function findAll(buf, needle) {
  let count = 0, i = 0;
  while ((i = buf.indexOf(needle, i)) !== -1) { count++; i += needle.length; }
  return count;
}
for (const f of process.argv.slice(2)) {
  const buf = fs.readFileSync(f);
  const rows = NEEDLES.map((n) => ({
    needle: n,
    ascii: findAll(buf, Buffer.from(n, "ascii")),
    utf16le: findAll(buf, Buffer.from(n, "utf16le")),
  }));
  console.log(JSON.stringify({
    file: f,
    bytes: buf.length,
    sha256: crypto.createHash("sha256").update(buf).digest("hex"),
    hits: rows,
  }, null, 1));
}

// 退出码兜底：任何未捕获异常/未处理 rejection 都必须让 CI 判定失败（否则失败也报“成功”）
process.on("uncaughtException", (e) => { console.error(e); process.exitCode = 1; });
process.on("unhandledRejection", (e) => { console.error(e); process.exitCode = 1; });
