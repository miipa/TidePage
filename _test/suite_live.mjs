// 沙箱 A–G 验证套件（连接受守卫启动的 9444；所有发送走 sendTestPrompt 唯一入口）
import fs from "node:fs";
import path from "node:path";
import { connectTarget } from "./cdp.mjs";
import { sendTestPrompt, expectSendBlocked } from "./send.mjs";
import { SANDBOX_ROOT } from "./isolation_guard.mjs";

const PORT = 9444;
const MOCKLOG = path.join(SANDBOX_ROOT, "mock_reqs.jsonl");
const mockReqs = () => (fs.existsSync(MOCKLOG) ? fs.readFileSync(MOCKLOG, "utf8").trim().split("\n").filter(Boolean).map((l) => JSON.parse(l)) : []);
const brief = (r) => ({ path: r.path, model: r.body.model, think: "think" in r.body ? r.body.think : "(absent)", unknownKeys: Object.keys(r.body).filter((k) => /think|reason|effort/i.test(k) && k !== "think") });
const log = (k) => console.error("STEP " + k);

const HELPER = `
const sleep=(ms)=>new Promise(r=>setTimeout(r,ms));
const wait=async(fn,name,rounds=150)=>{for(let i=0;i<rounds;i++){const v=fn();if(v)return v;await sleep(100);}throw new Error('timeout '+name)};
const field=(t)=>[...document.querySelectorAll('.field')].find(f=>f.textContent.includes(t));
const cfgRead=()=>JSON.parse(localStorage.getItem('flowread.config.v1')||'{}');
const openBook=async()=>{ if(!document.querySelector('#chapter-content')){ const c=await wait(()=>document.querySelector('.card,.row'),'card'); c.dispatchEvent(new MouseEvent('dblclick',{bubbles:true})); await wait(()=>document.querySelector('#chapter-content'),'chapter'); } await sleep(500); };
const openDrawer=async()=>{ localStorage.setItem('fr.ai.mode','drawer'); if(!document.querySelector('.ai-panel')) [...document.querySelectorAll('button')].find(x=>x.textContent.trim()==='AI')?.click(); const d=await wait(()=>document.querySelector('.ai-panel'),'panel'); if(!d.classList.contains('reader')){[...d.querySelectorAll('button')].find(x=>x.textContent.includes('阅读样式'))?.click();await sleep(400);} return d; };
const openSettings=async(tab)=>{ document.querySelector('button[title="设置"]')?.click(); await wait(()=>document.querySelector('dialog[open]'),'dialog'); [...document.querySelectorAll('.tabs button')].find(x=>x.textContent.trim()===tab)?.click(); await sleep(250); };
const closeSettings=async()=>{ document.querySelector('dialog button.primary')?.click(); await sleep(200); };
const uiSet=async(labelText,val)=>{ const s=await wait(()=>field(labelText)?.querySelector('select'),'uiSet '+labelText); s.value=val; s.dispatchEvent(new Event('change',{bubbles:true})); await sleep(150); };
const uiInput=async(labelText,val)=>{ const s=await wait(()=>field(labelText)?.querySelector('input'),'uiInput '+labelText); s.value=val; s.dispatchEvent(new Event('input',{bubbles:true})); s.dispatchEvent(new Event('change',{bubbles:true})); await sleep(150); };
const setReasoningUI=async(v)=>{ await openSettings('AI'); const s=await wait(()=>field('思考模式')?.querySelector('select'),'rs'); s.value=v; s.dispatchEvent(new Event('change',{bubbles:true})); closeSettings(); await sleep(400); };
const paltBox=async(want)=>{ await openSettings('阅读'); const cb=await wait(()=>[...document.querySelectorAll('label')].find(x=>x.textContent.includes('压缩全角标点'))?.querySelector('input'),'palt box'); if(cb.checked!==want) cb.click(); await sleep(250); };
const toastsNow=()=>[...document.querySelectorAll('.toast-card')].map(t=>t.textContent).join(' | ');
const TXT='，。！？（）《》，。！？（）《》，。！？（）《》，。！？（）《》，。！？（）《》';
const probeEl=(el)=>{ const cs=getComputedStyle(el); const mk=(ffs)=>{const s=document.createElement('span'); s.style.cssText='position:absolute;left:-99999px;top:0;white-space:nowrap;'; for(const k of ['fontFamily','fontSize','fontWeight','fontStyle','letterSpacing','fontFeatureSettings']) s.style[k]=cs[k]; s.style.fontFeatureSettings=ffs; s.textContent=TXT; document.body.appendChild(s); const w=s.getBoundingClientRect().width; s.remove(); return Math.round(w*100)/100;}; return {ffs:cs.fontFeatureSettings, family:cs.fontFamily, off:mk('normal'), on:mk('"palt" 1')}; };
`;
const H = (body) => `(async()=>{ ${HELPER}\n${body} })()`;

const out = {};
const main = await connectTarget(PORT, (t) => t.type === "page" && !t.url.includes("ai=1"));

log("seed");
out.seed = await main.evalJs(H(`
  const base=cfgRead();
  const cfg={ reader: { fontFamily: "方正兰亭圆_gbk", fontSize: 27, letterSpacing: 0.05, mode: "page", punctuationCompression: false },
    ai: { enabled:true, autoOnChapter:false, system: base.ai?.system||"", active:0, reasoningMode:"auto",
      profiles:[{name:"TEST-EP",provider:"ollama",baseUrl:"http://127.0.0.1:18081",apiKey:"sk-GUARDTEST-PLACEHOLDER-9f3k2l",model:"mock",proxyMode:"direct",proxyUrl:""}],
      prompts:[{id:1,name:"测试总结",key:"F1",text:"总结：{chapter}"}], display:"reader", style: base.ai?.style||{} },
    webdav:{enabled:false}, regex:"" };
  localStorage.setItem("flowread.config.v1", JSON.stringify(cfg));
  setTimeout(()=>location.reload(),200);
  return "seeded+reloading";
`));
await new Promise((r) => setTimeout(r, 5000));
main.close();

const M = await connectTarget(PORT, (t) => t.type === "page" && !t.url.includes("ai=1"));
await M.evalJs(H(`await openBook(); await openDrawer(); return 1`));
out.seedReadback = await M.evalJs(H(`
  const c=cfgRead(); const p=c.ai.profiles[0];
  return {name:p.name,provider:p.provider,baseUrl:p.baseUrl,model:p.model,keyLen:(p.apiKey||'').length,enabled:c.ai.enabled,reasoning:c.ai.reasoningMode};
`));

log("B");
{
  const n0 = mockReqs().length;
  await M.evalJs(H(`
    const c=cfgRead(); c.ai.profiles.push({name:"EVIL-DEEPSEEK",provider:"openai",baseUrl:"https://api.deepseek.com",apiKey:"x",model:"deepseek-chat",proxyMode:"auto",proxyUrl:""});
    localStorage.setItem("flowread.config.v1", JSON.stringify(c)); return 1`));
  out.B_blockedWith = await expectSendBlocked(M, "非本机端点");
  out.B_mockDelta = mockReqs().length - n0;
  await M.evalJs(H(`const c=cfgRead(); c.ai.profiles=c.ai.profiles.filter(p=>p.name!=="EVIL-DEEPSEEK"); localStorage.setItem("flowread.config.v1", JSON.stringify(c)); return 1`));
}

log("C");
{
  out.C_setOffViaUI = await M.evalJs(H(`await setReasoningUI("off"); return cfgRead().ai.reasoningMode`));
  await M.evalJs(H(`await openSettings('AI'); await uiSet('服务商','openai'); closeSettings(); await sleep(300); return 1;`));
  out.C_revert = await M.evalJs(H(`
    const t=toastsNow();
    await openSettings('AI');
    const sel=await wait(()=>field('思考模式')?.querySelector('select'),'rs');
    const v={ toastRevert:t.includes('已切回「自动」'), selectValue: sel.value, offDisabled: sel.querySelector('option[value="off"]').disabled, ls: cfgRead().ai.reasoningMode };
    closeSettings();
    return v;
  `));
  await M.evalJs(H(`setTimeout(()=>location.reload(),100); return 1`));
  await new Promise((r) => setTimeout(r, 4500));
  out.C_afterReload = await M.evalJs(H(`await openBook(); return {ls: cfgRead().ai.reasoningMode}`));
  await M.evalJs(H(`const c=cfgRead(); c.ai.reasoningMode="off"; localStorage.setItem("flowread.config.v1", JSON.stringify(c)); setTimeout(()=>location.reload(),200); return 1`));
  await new Promise((r) => setTimeout(r, 4500));
  out.C_importKeepsOff = await M.evalJs(H(`await openBook(); await openDrawer(); return { ls: cfgRead().ai.reasoningMode, provider: cfgRead().ai.profiles[0].provider }`));
  try { await sendTestPrompt(M); out.C_testSideGuard = "NOT-BLOCKED!!"; }
  catch (e) { out.C_testSideGuard = e.message.slice(0, 80); }
  const n0 = mockReqs().length;
  await sendTestPrompt(M, { allowUnsupported: true });
  await new Promise((r) => setTimeout(r, 700));
  out.C_appSideGuard = await M.evalJs(H(`return { toastBlocked: toastsNow().includes('已拦截'), marks: document.querySelectorAll('.ai-panel .ai-mark').length }`));
  out.C_appGuard_mockDelta = mockReqs().length - n0;
  await M.evalJs(H(`await openSettings('AI'); await uiSet('服务商','ollama'); closeSettings(); await sleep(300); return cfgRead().ai.profiles[0].provider`));
  out.C_backOllama_modeStillOff = await M.evalJs(H(`return cfgRead().ai.reasoningMode`));
}

log("D");
{
  await M.evalJs(H(`await setReasoningUI("auto")`));
  let n0 = mockReqs().length;
  out.D_auto_mock = { send: await sendTestPrompt(M), req: brief(mockReqs()[n0]) };
  await M.evalJs(H(`await openSettings('AI'); await uiInput('模型','mock-thinking'); closeSettings(); await sleep(300); return 1`));
  n0 = mockReqs().length;
  out.D_auto_thinkingServer = { send: await sendTestPrompt(M), req: brief(mockReqs()[n0]) };
  await M.evalJs(H(`await setReasoningUI("off")`));
  await M.evalJs(H(`await openSettings('AI'); await uiInput('模型','mock'); closeSettings(); await sleep(300); return 1`));
  n0 = mockReqs().length;
  out.D_off_compliant = { send: await sendTestPrompt(M), req: brief(mockReqs()[n0]) };
  await M.evalJs(H(`await openSettings('AI'); await uiInput('模型','mock-thinking'); closeSettings(); await sleep(300); return 1`));
  n0 = mockReqs().length;
  out.D_off_violating_guard = { send: await sendTestPrompt(M), req: brief(mockReqs()[n0]) };
  await M.evalJs(H(`await setReasoningUI("auto")`));
}

log("E");
{
  await M.evalJs(H(`await openSettings('AI'); await uiInput('模型','mock'); closeSettings(); await sleep(300); await setReasoningUI("auto"); return 1`));
  const n0 = mockReqs().length;
  let firstBody = null;
  const e = await sendTestPrompt(M, {
    onStarted: async () => {
      await new Promise((r) => setTimeout(r, 500));
      firstBody = mockReqs()[n0];
      await M.evalJs(H(`await setReasoningUI("off")`));
    },
  });
  out.E_firstStream = { completed: e, firstReqThink: firstBody ? ("think" in firstBody.body ? firstBody.body.think : "(absent)") : "n/a" };
  const n1 = mockReqs().length;
  const e2 = await sendTestPrompt(M);
  out.E_secondReq = { req: brief(mockReqs()[n1]), completed: e2 };
  await M.evalJs(H(`await setReasoningUI("auto")`));
}

log("F");
{
  await M.evalJs(H(`await paltBox(true); return 1`));
  out.F_fangzheng_hint = await M.evalJs(H(`
    const hint=field('压缩全角标点').textContent.includes('当前正文字体不支持比例标点');
    closeSettings();
    return { palt: cfgRead().reader.punctuationCompression, hint };
  `));
  await M.evalJs(H(`await paltBox(false); closeSettings(); return 1`));
  await M.evalJs(H(`
    await openSettings('阅读');
    const fv=await wait(()=>field('字体（支持搜索')?.querySelector('.font-value'),'font input');
    fv.value='Noto Sans SC'; fv.dispatchEvent(new Event('input',{bubbles:true})); fv.dispatchEvent(new Event('change',{bubbles:true}));
    await sleep(200);
    closeSettings();
    return cfgRead().reader.fontFamily;
  `));
  await new Promise((r) => setTimeout(r, 900));
  out.F_noto_hint = await M.evalJs(H(`
    await paltBox(true);
    const hintOn = field('压缩全角标点').textContent.includes('当前正文字体不支持比例标点');
    closeSettings();
    return { palt: cfgRead().reader.punctuationCompression, hintOnWithNoto: hintOn };
  `));
  await new Promise((r) => setTimeout(r, 800));
  out.F_noto_on = await M.evalJs(H(`
    const d=document.querySelector('.ai-panel');
    return { chapter: probeEl(document.querySelector('#chapter-content')), drawer: d.querySelector('.ai-mark') ? probeEl(d.querySelector('.ai-mark')) : null };
  `));
  await M.evalJs(H(`await paltBox(false); closeSettings(); return 1`));
  await new Promise((r) => setTimeout(r, 800));
  out.F_noto_off = await M.evalJs(H(`
    const d=document.querySelector('.ai-panel');
    return { chapter: probeEl(document.querySelector('#chapter-content')), drawer: d.querySelector('.ai-mark') ? probeEl(d.querySelector('.ai-mark')) : null };
  `));
}

log("G");
{
  await M.evalJs(H(`await paltBox(true); closeSettings(); await sleep(400); return 1`));
  out.G_start = await M.evalJs(H(`
    const pg=()=>Number(document.querySelector('.pctinfo')?.textContent.match(/^(\\d+)\\//)?.[1]||1);
    let g=0; while(pg()<5 && g++<10){ window.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true})); await sleep(220);}
    const d=await openDrawer();
    const cur=()=>Number((d.querySelector('.ai-pg-text')?.textContent||'').match(/第 (\\d+)/)?.[1]||1);
    g=0; while(cur()>1 && g++<80){ d.querySelector('button[title^="上一页"]')?.click(); await sleep(120);}
    return { chapter: document.querySelector('.pctinfo')?.textContent.trim(), ai: d.querySelector('.ai-pg-text')?.textContent.trim() };
  `));
  const flip = async () => {
    const want = !(await M.evalJs(H(`return cfgRead().reader.punctuationCompression`)));
    await M.evalJs(H(`await paltBox(${want}); closeSettings(); await sleep(1400); return 1`));
    return M.evalJs(H(`
      const d=document.querySelector('.ai-panel');
      return { palt: cfgRead().reader.punctuationCompression, chapter: document.querySelector('.pctinfo')?.textContent.trim(), ai: d.querySelector('.ai-pg-text')?.textContent.trim() };
    `));
  };
  out.G_toggle1 = await flip();
  out.G_toggle2 = await flip();
  out.G_toggle3 = await flip();
  const before = await M.evalJs(H(`return document.querySelectorAll('.ai-panel .ai-mark').length`));
  const done = await sendTestPrompt(M);
  await new Promise((r) => setTimeout(r, 700));
  const st = await M.evalJs(H(`
    const d=document.querySelector('.ai-panel'); const t=d.querySelector('.ai-pg-text')?.textContent.trim()||'';
    const m=t.match(/第 (\\d+)\\/(\\d+) 页/);
    return { aiText:t, atLast: m?Number(m[1])===Number(m[2]):null, chapter: document.querySelector('.pctinfo')?.textContent.trim() };
  `));
  out.G_newReply = { grew: done.marks > before, ...st };
}

log("final");
out.finalReadback = await M.evalJs(H(`
  const c=cfgRead(); const p=c.ai.profiles[0];
  return {name:p.name,provider:p.provider,baseUrl:p.baseUrl,model:p.model,keyLen:(p.apiKey||'').length,enabled:c.ai.enabled};
`));
out.mockKeyLeak = mockReqs().some((r) => JSON.stringify(r).includes("sk-GUARDTEST"));
M.close();
fs.writeFileSync(path.join(SANDBOX_ROOT, "suite_out.json"), JSON.stringify(out, null, 1));
console.log("SUITE DONE");
