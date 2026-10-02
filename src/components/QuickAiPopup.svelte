<script lang="ts">
  import { t } from "../lib/i18n/index.svelte";
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { config, copyText, saveConfigNow, toast } from "../lib/stores.svelte";
  import { api } from "../lib/api";
  import { esc, mdToHtml } from "../lib/md";
  import { placePopup, type PopupAnchor } from "../lib/placePopup";

  let {
    text,
    anchor,
    onclose,
    /** H3：若来自某条划线，则提供其 id —— 可把 AI 输出追加进该划线的 note。 */
    targetHighlightId = null,
    onadded,
    /** 任务 7：由 Reader 统一处理「添加到感想」。返回 true=已处理(禁用按钮)，false=失败(保持可点)。
     *  支持两种来源：已有划线追加 / 新划选先建划线再写 note；判重与提示由回调内部负责。 */
    onaddnote,
  }: {
    text: string;
    anchor: PopupAnchor;
    onclose: () => void;
    targetHighlightId?: number | null;
    onadded?: () => void;
    onaddnote?: (output: string) => Promise<boolean>;
  } = $props();

  let added = $state(false);
  let adding = $state(false);
  async function addToNote() {
    const out = output.trim();
    if (!out || busy || failed || added || adding) return;
    adding = true;
    try {
      if (onaddnote) {
        const ok = await onaddnote(out);
        if (ok) {
          added = true;
          onadded?.();
        }
      } else if (targetHighlightId) {
        // 兜底：无回调时沿用旧行为（仅已有划线可追加）
        await api.appendHighlightNote(targetHighlightId, out);
        added = true;
        toast(t("quickAi.addedToNote"));
        onadded?.();
      } else {
        toast(t("quickAi.noSelection"), "error");
      }
    } catch (e) {
      toast(String(e), "error");
    } finally {
      adding = false;
    }
  }

  let output = $state("");
  let busy = $state(false);
  let failed = $state(false);
  let popEl: HTMLDivElement | undefined = $state();
  let bodyEl: HTMLDivElement | undefined = $state();
  let seq = 0;
  let cancelled = false;
  let activeUn: (() => void) | null = null;

  function errLabel(kind: string): string {
    switch (kind) {
      case "network": return t("ai.errKind.network");
      case "timeout": return t("ai.errKind.timeout");
      case "http_4xx": return t("ai.errKind.http4xx");
      case "http_5xx": return t("ai.errKind.http5xx");
      case "protocol": return t("ai.errKind.protocol");
      case "model_id": return t("ai.errKind.modelId");
      case "config": return t("ai.errKind.config");
      default: return t("ai.errKind.unknown");
    }
  }

  function normErr(e: unknown): { kind: string; message: string } {
    if (e && typeof e === "object") {
      const o = e as Record<string, unknown>;
      if ("message" in o || "kind" in o) {
        return {
          kind: typeof o.kind === "string" ? o.kind : "unknown",
          message: typeof o.message === "string" ? o.message : JSON.stringify(e),
        };
      }
    }
    const s = typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
    return { kind: "unknown", message: s };
  }

  function renderPrompt(): string {
    const sel = text.length > 4000 ? text.slice(0, 4000) + "…" : text;
    return (config.ai.quickPrompt || "{selection}")
      .split("{selection}")
      .join(sel)
      .split("{text}")
      .join(sel);
  }

  async function run() {
    const profiles = config.ai.profiles;
    if (!profiles.length) {
      toast(t("ai.noEndpoint"), "error");
      return;
    }
    busy = true;
    failed = false;
    output = "";
    const userText = renderPrompt();
    const start = Math.min(Math.max(config.ai.active, 0), profiles.length - 1);
    const order: number[] = [];
    for (let k = 0; k < profiles.length; k++) order.push((start + k) % profiles.length);
    const maxTries = config.ai.failover ? order.length : 1;
    const failures: string[] = [];

    for (let a = 0; a < maxTries; a++) {
      if (cancelled) return;
      const idx = order[a];
      const p = profiles[idx];
      if (!p || !p.baseUrl || !p.model) {
        failures.push(t("ai.failureNoAddr", { name: p?.name || t("ai.endpointPlain"), msg: t("ai.noAddrOrModel") }));
        continue;
      }
      if (a > 0) output = "";
      const id = `q${++seq}-${Date.now()}`;
      let active = true;
      let un: (() => void) | null = null;
      const stop = listen<{ delta: string; done: boolean }>(`ai-token:${id}`, (e) => {
        if (!active || cancelled) return;
        if (e.payload.delta) {
          output += e.payload.delta;
          if (bodyEl) bodyEl.scrollTop = bodyEl.scrollHeight;
        }
      });
      un = () => stop.then((f) => f());
      activeUn = un;
      try {
        await api.aiChat({
          id,
          provider: p.provider,
          baseUrl: p.baseUrl,
          apiKey: p.apiKey,
          model: p.model,
          deployment: p.deployment,
          apiVersion: p.apiVersion,
          system: config.ai.system,
          messages: [["user", userText]],
          proxyMode: p.proxyMode,
          proxyUrl: p.proxyUrl,
        });
        active = false;
        un?.();
        activeUn = null;
        if (config.ai.active !== idx) {
          config.ai.active = idx;
          saveConfigNow();
        }
        busy = false;
        return;
      } catch (e) {
        active = false;
        un?.();
        activeUn = null;
        const err = normErr(e);
        failures.push(t("ai.errLine", { name: p.name || t("ai.endpointPlain"), kind: errLabel(err.kind), msg: err.message }));
        if (!config.ai.failover) break;
      }
    }
    failed = true;
    output = `${t("ai.allFailedTitle")}\n${failures.map((f) => `• ${f}`).join("\n")}`;
    busy = false;
  }

  function retry() {
    run();
  }

  $effect(() => {
    const h = (e: PointerEvent) => {
      if (popEl && !popEl.contains(e.target as Node)) {
        cancelled = true;
        onclose();
      }
    };
    window.addEventListener("pointerdown", h, true);
    return () => {
      cancelled = true;
      activeUn?.();
      activeUn = null;
      window.removeEventListener("pointerdown", h, true);
    };
  });

  onMount(() => {
    run();
  });

  let posStyle = $state("visibility:hidden;");
  $effect(() => {
    void anchor.top;
    void anchor.bottom;
    void anchor.cx;
    void output;
    if (!popEl) return;
    requestAnimationFrame(() => {
      if (!popEl) return;
      const p = placePopup(anchor, popEl.offsetWidth, popEl.offsetHeight);
      posStyle = `left:${p.left}px; top:${p.top}px;`;
    });
  });
</script>

<div class="qa-pop" bind:this={popEl} style={posStyle}>
  <div class="qa-head">
    <span data-testid="qa-title">{t("quickAi.title")}</span>
    <div class="qa-acts">
      {#if !busy && output}
        <button class="mini" data-testid="qa-retry" title={t("quickAi.regenerate")} onclick={retry}>↻</button>
        <button class="mini" data-testid="qa-copy" title={t("quickAi.copyResult")} onclick={() => copyText(output)}>{t("common.action.copy")}</button>
      {/if}
      <button
        class="mini primary"
        data-testid="qa-add-note"
    title={t("quickAi.addNoteTip")}
        onclick={addToNote}
        disabled={busy || !output.trim() || failed || added || adding}
      >
        {added ? t("common.toast.added") : t("quickAi.addToNote")}
      </button>
      <button class="mini" data-testid="qa-close" title={t("common.action.close")} onclick={onclose}>×</button>
    </div>
  </div>
  <div class="qa-body" bind:this={bodyEl}>
    {#if output}
      <div class="qa-mark" class:err={failed}>{@html failed ? `<pre>${esc(output)}</pre>` : mdToHtml(output)}</div>
    {:else}
      <div class="qa-loading">{busy ? t("ai.thinking") : "…"}</div>
    {/if}
  </div>
</div>

<style>
  .qa-pop {
    position: fixed;
    z-index: 980;
    width: min(420px, 88vw);
    max-height: min(58vh, 480px);
    display: flex;
    flex-direction: column;
    background: color-mix(in srgb, var(--rdr-bg, #f5f1e8) 94%, #202020 6%);
    color: var(--rdr-fg, #2c2c2c);
    border: 1px solid rgba(128, 128, 128, 0.3);
    border-radius: 12px;
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.16), 0 24px 52px rgba(0, 0, 0, 0.26);
    overflow: hidden;
    animation: qa-in 0.2s cubic-bezier(0.32, 0.72, 0, 1) both;
  }

  @keyframes qa-in {
    from { opacity: 0; transform: translateY(6px) scale(0.97); }
    to { opacity: 1; transform: none; }
  }

  .qa-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 7px 12px;
    font-size: 13px;
    font-weight: 600;
    border-bottom: 1px solid rgba(128, 128, 128, 0.16);
  }

  .qa-acts { display: flex; gap: 4px; }

  .mini {
    font-size: 12px;
    padding: 2px 7px;
    border-radius: 6px;
  }

  .qa-body {
    overflow-y: auto;
    padding: 10px 13px;
    font-size: 13px;
    line-height: 1.7;
  }

  .qa-mark :global(p) { margin: 0.35em 0; }
  .qa-mark :global(h1), .qa-mark :global(h2), .qa-mark :global(h3), .qa-mark :global(h4) {
    margin: 0.5em 0 0.3em;
    font-weight: 600;
  }
  .qa-mark :global(ul), .qa-mark :global(ol) { padding-left: 1.5em; }
  .qa-mark :global(pre) {
    background: rgba(128, 128, 128, 0.14);
    padding: 8px 10px;
    border-radius: 6px;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .qa-mark.err { opacity: 0.85; font-size: 13px; }

  .qa-loading {
    opacity: 0.55;
    font-size: 13px;
    padding: 6px 0;
  }
</style>
