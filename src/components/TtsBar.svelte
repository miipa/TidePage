<script lang="ts">
  import { t } from "../lib/i18n/index.svelte";
  import { activeTtsService, config, saveConfig, toast } from "../lib/stores.svelte";
  import {
    refreshOnlineVoices,
    refreshVoices,
    ttsNext,
    ttsPauseResume,
    ttsPrev,
    ttsState,
    ttsStop,
  } from "../lib/tts.svelte";
  import type { FlatSegment, OnlineVoice } from "../lib/tts.svelte";
  import {
    IconClock,
    IconPlayerPause,
    IconPlayerPlay,
    IconPlayerSkipBack,
    IconPlayerSkipForward,
    IconPlayerStop,
    IconRefresh,
    IconSettings,
  } from "@tabler/icons-svelte";

  let { current = null as FlatSegment | null }: { current?: FlatSegment | null } = $props();

  // 不内置 OpenAI 别名预设（alloy/echo/…）：无固定说话人，不适用于克隆引擎。
  // 音色一律以服务端返回的列表为准。
  const FALLBACK_ONLINE_VOICES: OnlineVoice[] = [];

  let expanded = $state(false);
  let sleepLeft = $state(0);
  let sleepEnd = 0;

  refreshVoices();

  const onlineVoices = $derived(
    ttsState.onlineVoices.length ? ttsState.onlineVoices : FALLBACK_ONLINE_VOICES
  );
  const onlineVoiceProfiles = $derived(onlineVoices.filter((v) => v.type === "profile"));
  // 隐藏 OpenAI 别名，仅保留服务端的固定音色与预设。
  const onlineVoiceOthers = $derived(
    onlineVoices.filter((v) => v.type !== "profile" && v.type !== "openai_alias")
  );
  // 当前语音服务档案（无档案时退回第一条）。
  const onlineSvc = $derived(activeTtsService() ?? config.tts.services[0]);

  $effect(() => {
    const svc = onlineSvc;
    const ready = ((svc?.baseUrl || "").trim()) || svc?.provider === "edge";
    if (expanded && config.tts.engine === "online" && ready) {
      void refreshOnlineVoices();
    }
  });

  $effect(() => {
    const min = config.tts.sleepMinutes;
    void ttsState.playing;
    if (!min || !ttsState.playing) {
      sleepEnd = 0;
      sleepLeft = 0;
      return;
    }
    sleepEnd = Date.now() + min * 60000;
    sleepLeft = Math.round(min * 60);
    // 局部变量改名 timer：避免与 i18n 的 t() 同名遮蔽
    const timer = setInterval(() => {
      sleepLeft = Math.max(0, Math.round((sleepEnd - Date.now()) / 1000));
      if (sleepLeft <= 0) {
        clearInterval(timer);
        ttsStop();
        toast(t("tts.stopTimerDone"));
      }
    }, 1000);
    return () => clearInterval(timer);
  });

  function fmtLeft(s: number): string {
    const m = Math.floor(s / 60);
    const ss = s % 60;
    return `${m}:${String(ss).padStart(2, "0")}`;
  }

  const zhVoices = $derived(
    ttsState.voices.filter((v) => (v.lang || "").toLowerCase().startsWith("zh"))
  );
  const otherVoices = $derived(
    ttsState.voices.filter((v) => !(v.lang || "").toLowerCase().startsWith("zh"))
  );
</script>

<div class="tts-bar rdr-pop" class:expanded>
  <button class="tts-btn" data-testid="tts-prev" title={t("tts.prevSentence")} onclick={ttsPrev}><IconPlayerSkipBack /></button>
  <button class="tts-btn primary" data-testid="tts-pause" title={ttsState.paused ? t("tts.resume") : t("tts.pause")} onclick={ttsPauseResume}>
    {#if ttsState.paused}<IconPlayerPlay />{:else}<IconPlayerPause />{/if}
  </button>
  <button class="tts-btn" data-testid="tts-next" title={t("tts.nextSentence")} onclick={ttsNext}><IconPlayerSkipForward /></button>
  <button class="tts-btn stop" data-testid="tts-stop" title={t("tts.stop")} onclick={ttsStop}><IconPlayerStop /></button>

  <div class="tts-info" title={current?.text || ""}>
    <span class="tts-count">
      {ttsState.index >= 0 ? ttsState.index + 1 : 0}/{ttsState.total}
    </span>
    <span class="tts-text">{current?.text || "…"}</span>
    {#if ttsState.buffering}
      <span class="tts-buffering" title={t("tts.generating")}>{t("tts.buffering")}</span>
    {/if}
  </div>

  {#if sleepLeft > 0}
    <span class="tts-sleep" title={t("tts.timerRemain")}><IconClock /> {fmtLeft(sleepLeft)}</span>
  {/if}

  <button class="tts-btn" data-testid="tts-settings" title={t("tts.settings")} onclick={() => (expanded = !expanded)}><IconSettings /></button>

  {#if expanded}
    <div class="tts-panel">
      <div class="tts-row">
        <label>{config.tts.engine === "online" ? t("tts.voiceLabel") : t("tts.voice")}</label>
        {#if config.tts.engine === "online"}
          <select bind:value={onlineSvc.voice} onchange={saveConfig}>
            <option value="">{t("tts.defaultVoice")}</option>
            {#if onlineVoiceProfiles.length}
              <optgroup label={t("tts.clonedVoice")}>
                {#each onlineVoiceProfiles as v (v.id)}
                  <option value={v.id}>{v.name || v.id}{v.language ? ` · ${v.language}` : ""}</option>
                {/each}
              </optgroup>
            {/if}
            {#if onlineVoiceOthers.length}
              <optgroup label={t("tts.otherVoices")}>
                {#each onlineVoiceOthers as v (v.id)}
                  <option value={v.id}>{v.name && v.name !== v.id ? `${v.name} (${v.id})` : v.id}</option>
                {/each}
              </optgroup>
            {/if}
          </select>
          <button
            class="tts-mini"
            data-testid="tts-load-voices"
            title={t("tts.loadVoices")}
            onclick={() => refreshOnlineVoices()}
            disabled={ttsState.onlineVoiceLoading}
          >{#if ttsState.onlineVoiceLoading}…{:else}<IconRefresh />{/if}</button>
        {:else}
          <select bind:value={config.tts.voice} onchange={saveConfig}>
            <option value="">{t("tts.auto")}</option>
            {#if zhVoices.length}
              <optgroup label={t("tts.lang.zh")}>
                {#each zhVoices as v (v.voiceURI)}
                  <option value={v.name}>{v.name} · {v.lang}</option>
                {/each}
              </optgroup>
            {/if}
            {#if otherVoices.length}
              <optgroup label={t("tts.langOther")}>
                {#each otherVoices as v (v.voiceURI)}
                  <option value={v.name}>{v.name} · {v.lang}</option>
                {/each}
              </optgroup>
            {/if}
          </select>
        {/if}
      </div>
      <div class="tts-grid">
        <label>{t("tts.rate")} {config.tts.rate.toFixed(2)}</label>
        <input type="range" min="0.5" max="2" step="0.05" bind:value={config.tts.rate} oninput={saveConfig} />
        <label>{t("tts.pitch")} {config.tts.pitch.toFixed(2)}</label>
        <input type="range" min="0" max="2" step="0.05" bind:value={config.tts.pitch} oninput={saveConfig} />
        <label>{t("tts.volume")} {Math.round(config.tts.volume * 100)}%</label>
        <input type="range" min="0" max="1" step="0.05" bind:value={config.tts.volume} oninput={saveConfig} />
        <label>{t("tts.timerStop")}</label>
        <select data-testid="tts-sleep" bind:value={config.tts.sleepMinutes} onchange={saveConfig}>
          <option value={0}>{t("common.action.close")}</option>
          <option value={5}>{t("tts.minutes", { n: 5 })}</option>
          <option value={10}>{t("tts.minutes", { n: 10 })}</option>
          <option value={15}>{t("tts.minutes", { n: 15 })}</option>
          <option value={30}>{t("tts.minutes", { n: 30 })}</option>
          <option value={60}>{t("tts.minutes", { n: 60 })}</option>
        </select>
      </div>
      <div class="tts-checks">
        <label><input type="checkbox" bind:checked={config.tts.highlight} onchange={saveConfig} /> {t("tts.highlightFollow")}</label>
        <label><input type="checkbox" bind:checked={config.tts.autoNextChapter} onchange={saveConfig} /> {t("tts.autoNext")}</label>
      </div>
    </div>
  {/if}
</div>

<style>
  /* ---- 前缀归属（改名前必读）----
     .tb-* 专属 Reader.svelte 的顶栏（topbar = 工具条）：.tb-zone/.tb-l/.tb-c/.tb-r/.tb-btn/
     .tb-panel/.tb-menu-item/... 那边一个字都不许动，_test 里有十几处 querySelector 依赖它。

     TTS 条曾经误用过同一套 .tb-* 前缀（.tb-btn 是播放键、.tb-panel 是 TTS 设置展开区），
     而 Reader 会把本组件渲染进 .reader 子树 —— 两个组件的类名于是落在同一棵 DOM 里。
     后果不是样式泄漏（两边都是 scoped，编译后各带自己的 scope class，互不命中），
     而是**全局选择器与 querySelector 会命中错的那一个**：document.querySelector('.tb-panel')
     返回的是 DOM 顺序里的第一个，而不是「你指的那个」，今天恰好命中正确的那一个纯属
     DOM 顺序的巧合，不是契约。任何人调整这两个组件在模板里的先后顺序，
     或者顺手写一条裸 .tb-btn / .tb-panel 的全局规则，就会静默命中错的一套。

     现在 TTS 侧统一 .tts-*，.tb-* 归顶栏专用。这段注释是防止下一个把它
     「统一」回去的唯一依据 —— 要改前缀请先读完上面那段。 */
  /* 底色 / 描边 / 投影走全局 .rdr-pop（跟随阅读主题）；
     圆角保留药丸形——这是一条「芯片条」而非卡片，故用 --radius-pill。 */
  .tts-bar {
    position: fixed;
    left: 50%;
    bottom: 46px;
    transform: translateX(-50%);
    z-index: var(--z-sticky);
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    padding: var(--sp-1) 10px;
    max-width: min(760px, 92vw);
    border-radius: var(--radius-pill);
    flex-wrap: wrap;
    /* 固定药丸：用 opacity + translateY 的轻量入场，不带 blur（会与条内正在更新的文本抢合成层） */
    animation: fr-rise-in var(--dur) var(--ease-premium) both;
  }

  .tts-btn {
    border: none;
    background: transparent;
    color: inherit;
    font-size: var(--fs-lg);
    line-height: 1;
    padding: var(--sp-1) var(--sp-2);
    border-radius: 50%;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .tts-btn:hover { background: color-mix(in srgb, var(--rdr-fg) 12%, transparent); }

  /* 播放键是阅读区浮层里唯一的「实底强调」，按纪律取阅读区自己的强调色：
     换 UI 皮肤不该让这本书的播放键变色（此前引外壳 --accent/--accent-fg）。 */
  .tts-btn.primary {
    background: var(--rdr-accent);
    color: var(--rdr-accent-fg);
    font-size: var(--fs-md);
  }

  .tts-btn.primary:hover { background: var(--rdr-accent-hover); }

  .tts-btn.stop { color: var(--danger); }

  .tts-info {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-width: 120px;
    max-width: 340px;
    overflow: hidden;
  }

  .tts-count {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    opacity: 0.65;
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }

  .tts-text {
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    opacity: 0.85;
  }

  .tts-sleep {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    opacity: 0.75;
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
  }
  .tts-buffering {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    opacity: 0.7;
    flex-shrink: 0;
    animation: tts-pulse 1s ease-in-out infinite;
  }

  @keyframes tts-pulse {
    0%, 100% { opacity: 0.35; }
    50% { opacity: 0.85; }
  }

  .tts-mini {
    border: none;
    background: transparent;
    color: inherit;
    font-size: var(--fs-sm);
    padding: 3px var(--sp-1);
    border-radius: var(--radius-pill);
    cursor: pointer;
    flex-shrink: 0;
  }

  .tts-mini:hover { background: color-mix(in srgb, var(--rdr-fg) 12%, transparent); }

  .tts-panel {
    flex-basis: 100%;
    border-top: 1px solid var(--rdr-line);
    margin-top: var(--sp-1);
    padding-top: var(--sp-2);
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    animation: fr-fade-in var(--dur-fast) var(--ease-in) both;
  }

  .tts-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }

  .tts-row label { font-size: var(--fs-xs); letter-spacing: var(--ls-xs); opacity: 0.7; width: 36px; }
  .tts-row select { flex: 1; min-width: 0; font-size: var(--fs-xs); }

  .tts-grid {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 10px;
    align-items: center;
  }

  .tts-grid label { font-size: var(--fs-xs); letter-spacing: var(--ls-xs); opacity: 0.7; white-space: nowrap; }
  .tts-grid input[type="range"] { width: 100%; }
  .tts-grid select { font-size: var(--fs-xs); }

  .tts-checks {
    display: flex;
    gap: var(--sp-4);
    flex-wrap: wrap;
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
  }
</style>
