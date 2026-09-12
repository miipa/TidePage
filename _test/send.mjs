// 测试发送唯一入口。除本文件外，任何测试脚本不得点击 .prompt-chip、
// 不得调用 firePrompt()/api.aiChat —— 全部发送必须经 sendTestPrompt()，
// 其内部先 withSendGuard（从页面实时读 config，校验所有 ai.profiles 为本机回环）。
import { withSendGuard } from "./isolation_guard.mjs";

const CHIP = ".prompt-chip:not(.add)";

export async function sendTestPrompt(cdp, { timeoutMs = 90_000, onStarted = null, allowUnsupported = false } = {}) {
  return withSendGuard(cdp.evalJs, async () => {
    // 守卫第二层：off 时按与前端 reasoningOffUsable 相同逻辑判定可用性（含策略），不可用才拦截；
    // allowUnsupported 仅用于验证应用侧 run() 拦截本身
    const state = await cdp.evalJs(`(()=>{const c=JSON.parse(localStorage.getItem('flowread.config.v1'));const p=c.ai.profiles[Math.min(c.ai.active|0,c.ai.profiles.length-1)];return {mode:c.ai.reasoningMode, provider:p.provider, strategy:p.reasoningOffStrategy||'switch-model', offModel:(p.reasoningOffModel||'').trim(), model:(p.model||'').trim()}})()`);
    const plainId = (s) => !!s && s.length <= 160 && !s.startsWith('/') && !s.includes('://') && /^[A-Za-z0-9][A-Za-z0-9._:/-]*$/.test(s);
    const usable = state.mode !== "off" || state.provider === "ollama"
      || (state.strategy === "switch-model" ? plainId(state.offModel) : plainId(state.model));
    if (!allowUnsupported && state.mode === "off" && !usable) {
      throw new Error(`发送拦截：reasoningMode=off 而激活端点 ${state.provider}（策略 ${state.strategy}）不可用，测试不点击、不假装关闭。`);
    }
    return cdp.evalJs(`(async()=>{
      const sleep=ms=>new Promise(r=>setTimeout(r,ms));
      const d=document.querySelector('.ai-panel');
      if(!d) throw new Error('sendTestPrompt: 未找到 .ai-panel');
      const chip=d.querySelector(${JSON.stringify(CHIP)});
      if(!chip) throw new Error('sendTestPrompt: 未找到提示词芯片');
      for(let w=0;w<100 && d.querySelector(${JSON.stringify(CHIP)}).disabled;w++) await sleep(200); // 等待上一条流式结束，避免点击被忽略
      const before=d.querySelectorAll('.ai-mark').length;
      chip.click();
      await sleep(150);
      ${onStarted || allowUnsupported ? "return {__started:true, before};" : ""}
      const max=${Math.ceil(timeoutMs / 200)};
      for(let i=0;i<max;i++){
        const ms=d.querySelectorAll('.ai-mark');
        const last=ms[ms.length-1];
        if(ms.length>before && !d.querySelector(${JSON.stringify(CHIP)}).disabled){
          return {before, marks: ms.length, len: last.textContent.length,
                  err: last.textContent.includes('[错误]'),
                  guardMsg: last.textContent.includes('未执行关闭思考'),
                  streamLoading: d.querySelector(${JSON.stringify(CHIP)}).disabled,
                  thinkUI: document.querySelectorAll('[class*=think],[class*=reason]').length};
        }
        await sleep(200);
      }
      throw new Error('sendTestPrompt: 流式回复超时未完成');
    })()`).then(async (r) => {
      if (r && r.__started) {
        if (onStarted) {
          await onStarted();
          return waitReplyDone(cdp, { timeoutMs, before: r.before });
        }
        return { startedOnly: true, before: r.before };
      }
      return r;
    });
  });
}

export async function waitReplyDone(cdp, { timeoutMs = 90_000, before = 0 } = {}) {
  return cdp.evalJs(`(async()=>{
    const sleep=ms=>new Promise(r=>setTimeout(r,ms));
    const d=document.querySelector('.ai-panel');
    const max=${Math.ceil(timeoutMs / 200)};
    for(let i=0;i<max;i++){
      const ms=d.querySelectorAll('.ai-mark');
      const last=ms[ms.length-1];
      if(last && ms.length>${before || 0} && !d.querySelector(${JSON.stringify(CHIP)}).disabled){
        return {marks: ms.length, len: last.textContent.length,
                err: last.textContent.includes('[错误]'),
                guardMsg: last.textContent.includes('未执行关闭思考'),
                streamLoading: d.querySelector(${JSON.stringify(CHIP)}).disabled,
                thinkUI: document.querySelectorAll('[class*=think],[class*=reason]').length};
      }
      await sleep(200);
    }
    throw new Error('waitReplyDone: 超时未完成');
  })()`);
}

// 发送但预期被守卫拦截（用于反例/C 导入场景断言）：断言未点击即 throw。
export async function expectSendBlocked(cdp, reasonIncludes = "非本机端点") {
  let err = null;
  try { await withSendGuard(cdp.evalJs, async () => { throw new Error("__SHOULD_NOT_REACH__"); }); }
  catch (e) { err = e; }
  if (!err) throw new Error("守卫未拦截，发送路径可达 —— fail-open，不可接受");
  if (err.message.includes("__SHOULD_NOT_REACH__")) throw new Error("守卫放行后进入了 action，失败");
  if (!err.message.includes(reasonIncludes)) throw new Error(`拦截原因不符：${err.message}`);
  return err.message;
}
