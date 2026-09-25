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
