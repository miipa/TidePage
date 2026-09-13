// 测试发送唯一入口。除本文件外，任何测试脚本不得点击 .prompt-chip、
// 不得调用 firePrompt()/api.aiChat —— 全部发送必须经 sendTestPrompt()，
// 其内部先 withSendGuard（从页面实时读 config，校验所有 ai.profiles 为本机回环）。
// “关闭思考”功能已移除，发送行为恒为 auto；本入口不再有第二层策略守卫。
import { withSendGuard } from "./isolation_guard.mjs";

const CHIP = ".prompt-chip:not(.add)";

export async function sendTestPrompt(cdp, { timeoutMs = 90_000, onStarted = null } = {}) {
  return withSendGuard(cdp.evalJs, async () => {
    return cdp.evalJs(`(async()=>{
      const sleep=ms=>new Promise(r=>setTimeout(r,ms));
      const d=document.querySelector('.ai-panel');
      if(!d) throw new Error('sendTestPrompt: 未找到 .ai-panel');
      const chip=d.querySelector(${JSON.stringify(CHIP)});
      if(!chip) throw new Error('sendTestPrompt: 未找到提示词芯片');
      for(let w=0;w<100 && d.querySelector(${JSON.stringify(CHIP)}).disabled;w++) await sleep(200);
      const before=d.querySelectorAll('.ai-mark').length;
      chip.click();
      await sleep(150);
      ${onStarted ? "return {__started:true, before};" : ""}
      const max=${Math.ceil(timeoutMs / 200)};
      for(let i=0;i<max;i++){
        const ms=d.querySelectorAll('.ai-mark');
        const last=ms[ms.length-1];
        if(ms.length>before && !d.querySelector(${JSON.stringify(CHIP)}).disabled){
          return {before, marks: ms.length, len: last.textContent.length,
                  err: last.textContent.includes('[错误]'),
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
                streamLoading: d.querySelector(${JSON.stringify(CHIP)}).disabled,
                thinkUI: document.querySelectorAll('[class*=think],[class*=reason]').length};
      }
      await sleep(200);
    }
    throw new Error('waitReplyDone: 超时未完成');
  })()`);
}

// 断言发送被守卫拦截（本机端点校验），不点击。
export async function expectSendBlocked(cdp, reasonIncludes = "非本机端点") {
  let err = null;
  try { await withSendGuard(cdp.evalJs, async () => { throw new Error("__SHOULD_NOT_REACH__"); }); }
  catch (e) { err = e; }
  if (!err) throw new Error("守卫未拦截，发送路径可达 —— fail-open，不可接受");
  if (err.message.includes("__SHOULD_NOT_REACH__")) throw new Error("守卫放行后进入了 action，失败");
  if (!reasonIncludes || !err.message.includes(reasonIncludes)) throw new Error(`拦截原因不符：${err.message}`);
  return err.message;
}
