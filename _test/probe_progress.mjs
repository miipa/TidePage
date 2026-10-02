import { connectTarget } from "./cdp.mjs";
const port = process.env.PORT || 9333;
const wait = Number(process.env.WAIT || 0);
const t = await connectTarget(port, (x) => x.type === "page" && !String(x.url).includes("devtools"));
if (wait) await new Promise((r) => setTimeout(r, wait));
const out = await t.evalJs(`(() => {
  const sb = document.querySelector('.statusbar');
  return JSON.stringify({
    hasStatusbar: !!sb,
    statusbarText: sb ? sb.innerText.replace(/\\s+/g,' ').trim() : null,
    metaProgressEl: !!document.querySelector('.meta-progress'),
  });
})()`);
console.log("PROBE:", out);
t.close();
process.exit(0);

// 退出码兜底：任何未捕获异常/未处理 rejection 都必须让 CI 判定失败（否则失败也报“成功”）
process.on("uncaughtException", (e) => { console.error(e); process.exitCode = 1; });
process.on("unhandledRejection", (e) => { console.error(e); process.exitCode = 1; });
