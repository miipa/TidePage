// 必修C：4 张极端底图 → 真机封面 PNG 截图（肉眼可读证据）。
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import { connectTarget } from "./cdp.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";
import { launchSandbox, killTree, sweepSandboxWebviews } from "./launch_sandbox.mjs";

const PORT = Number(process.env.FR_PORT || 9711);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sbx = path.join(SANDBOX_ROOT, "ex_sbx"); const data = path.join(SANDBOX_ROOT, "ex_data");
fs.mkdirSync(sbx, { recursive: true }); fs.mkdirSync(data, { recursive: true });
for (const d of [sbx, data]) for (const f of fs.readdirSync(d)) fs.rmSync(path.join(d, f), { recursive: true, force: true });
const outDir = path.join(SANDBOX_ROOT, "extreme_shots"); fs.rmSync(outDir, { recursive: true, force: true }); fs.mkdirSync(outDir, { recursive: true });
const isApp = (t) => t.type === "page" && !t.url.includes("ai=1");
const H = (b) => `(async()=>{ const sleep=(ms)=>new Promise(r=>setTimeout(r,ms)); const q=(s)=>document.querySelector(s); const qa=(s)=>[...document.querySelectorAll(s)]; const inv=(c,a={})=>window.__TAURI_INTERNALS__.invoke(c,a);\n${b} })()`;

function crc32(buf){const T=[];for(let n=0;n<256;n++){let c=n;for(let k=0;k<8;k++)c=c&1?0xedb88320^(c>>>1):c>>>1;T[n]=c>>>0}let c=0xffffffff;for(let i=0;i<buf.length;i++)c=T[(c^buf[i])&0xff]^(c>>>8);return (c^0xffffffff)>>>0}
function chunk(t,b){const l=Buffer.alloc(4);l.writeUInt32BE(b.length,0);const tt=Buffer.from(t,"ascii");const cr=Buffer.alloc(4);cr.writeUInt32BE(crc32(Buffer.concat([tt,b])),0);return Buffer.concat([l,tt,b,cr])}
function makePng(w,h,px){const raw=Buffer.alloc((w*3+1)*h);let o=0;for(let y=0;y<h;y++){raw[o++]=0;for(let x=0;x<w;x++){const[r,g,b]=px(x,y);raw[o++]=r;raw[o++]=g;raw[o++]=b}}const ih=Buffer.alloc(13);ih.writeUInt32BE(w,0);ih.writeUInt32BE(h,4);ih[8]=8;ih[9]=2;return Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),chunk("IHDR",ih),chunk("IDAT",zlib.deflateSync(raw)),chunk("IEND",Buffer.alloc(0))])}

const bases = {
  "1-white": makePng(400,600,()=>[255,255,255]),
  "2-black": makePng(400,600,()=>[8,8,8]),
  "3-mid": makePng(400,600,(x)=>x<200?[112,128,120]:[104,116,132]),
  "4-split": makePng(400,600,(x,y)=>(x+y)%2===0?[18,20,26]:[236,238,243]),
};
const out = {};
let app=null;
try {
  app = launchSandbox({ env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: sbx, TIDEPAGE_DATA_DIR: data }, cdpPort: PORT, logFile: path.join(SANDBOX_ROOT, "ex_diag.log") });
  let M=null; for(let i=0;i<240;i++){try{M=await connectTarget(PORT,isApp);break}catch{await sleep(2000)}}
  for(let i=0;i<300;i++){try{if(await M.evalJs(`!!window.__TAURI_INTERNALS__`))break}catch{}await sleep(400)}
  const tmp=path.join(SANDBOX_ROOT,"ex_books"); fs.rmSync(tmp,{recursive:true,force:true}); fs.mkdirSync(tmp,{recursive:true});
  fs.writeFileSync(path.join(tmp,"测试书名 (测试作者).txt"),"正文。".repeat(40),"utf8");
  const p=path.join(tmp,"测试书名 (测试作者).txt").replace(/\\/g,"\\\\");
  await M.evalJs(H(`try{await inv('add_dropped_book',{path:"${p}"});}catch(e){}`));
  await sleep(1500); await M.evalJs(`setTimeout(()=>location.reload(),100);1`); await sleep(3500);
  M.close(); M=await connectTarget(PORT,isApp);
  for(let i=0;i<300;i++){try{if(await M.evalJs(`!!window.__TAURI_INTERNALS__`))break}catch{}await sleep(400)}
  await M.evalJs(H(`for(let i=0;i<60;i++){const s=await inv('meta_debug_stats');if(!s.running)break;await sleep(400)}return 1`));
  for (const [tag,buf] of Object.entries(bases)) {
    const bp=path.join(outDir,tag+".png"); // 底图原图
    fs.writeFileSync(bp,buf);
    const r = await M.evalJs(H(`
      await inv('set_default_cover',{imagePath:${JSON.stringify(bp.replace(/\\/g,"\\\\"))}});
      await sleep(500);
      const pg=await inv('list_books_page',{query:'',sort:'title',offset:0,limit:50});
      const b=pg.rows.filter(x=>/ex_books/i.test(x.path))[0];
      const svg=await (await fetch('http://tidepage.localhost/cover-default/'+b.id)).text();
      const dataUrl=await new Promise((res)=>{
        const blob=new Blob([svg],{type:'image/svg+xml'});const url=URL.createObjectURL(blob);
        const img=new Image();img.onload=()=>{const c=document.createElement('canvas');c.width=320;c.height=480;const ctx=c.getContext('2d');ctx.drawImage(img,0,0,320,480);URL.revokeObjectURL(url);res(c.toDataURL('image/png'))};
        img.onerror=()=>res(null);img.src=url;
      });
      return { title:b.title, author:b.author, dataUrl };
    `));
    if (r.dataUrl) {
      fs.writeFileSync(path.join(outDir,"cover-"+tag+".png"), Buffer.from(r.dataUrl.split(",")[1],"base64"));
      out[tag]={ title:r.title, author:r.author, ok:true };
    } else out[tag]={ ok:false };
    console.log(tag, JSON.stringify(out[tag]));
  }
  M.close();
} catch(e){ process.exitCode = 1; /* 失败必须体现在退出码上 */ out.error=String(e); console.error(e); }
finally { if(app){killTree(app.child.pid); await sleep(1500); sweepSandboxWebviews("fr_test_profiles");} fs.writeFileSync(path.join(SANDBOX_ROOT,"ex_out.json"),JSON.stringify(out,null,1)); console.log("PNGs in",outDir); }

// 退出码兜底：任何未捕获异常/未处理 rejection 都必须让 CI 判定失败（否则失败也报“成功”）
process.on("uncaughtException", (e) => { console.error(e); process.exitCode = 1; });
process.on("unhandledRejection", (e) => { console.error(e); process.exitCode = 1; });
