import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";
const PORT=9788; const sleep=(ms)=>new Promise(r=>setTimeout(r,ms));
const sbx=path.join(SANDBOX_ROOT,"smoke_sbx"); const data=path.join(SANDBOX_ROOT,"smoke_data");
fs.mkdirSync(sbx,{recursive:true}); fs.mkdirSync(data,{recursive:true});
for(const d of [sbx,data]) for(const f of fs.readdirSync(d)) fs.rmSync(path.join(d,f),{recursive:true,force:true});
const isApp=(t)=>t.type==="page"&&!t.url.includes("ai=1");
let app=null;
try{
  app=launchSandbox({env:{...process.env,WEBVIEW2_USER_DATA_FOLDER:sbx,TIDEPAGE_DATA_DIR:data},cdpPort:PORT,logFile:path.join(SANDBOX_ROOT,"smoke.log")});
  console.log("launched",app.child.pid);
  let M=null; for(let i=0;i<60;i++){ try{M=await connectTarget(PORT,isApp);break}catch{await sleep(2000)} }
  console.log("cdp:",!!M);
  if(M){ const r=await M.evalJs("!!window.__TAURI_INTERNALS__"); console.log("tauri:",r); M.close(); }
}catch(e){console.error("ERR",e.message)}
finally{ if(app){killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles");} }
