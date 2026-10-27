<script lang="ts">
  import { t } from "../lib/i18n/index.svelte";
  import { outDissolve } from "../lib/motion";
  import { activeTtsService, config, saveConfig, toast } from "../lib/stores.svelte";
  import {
    refreshOnlineVoices,
    refreshVoices,
    ttsArmSleep,
    ttsClearSleep,
    ttsNext,
    ttsPauseResume,
    ttsPrev,
    ttsSetBook,
    ttsState,
    ttsStop,
  } from "../lib/tts.svelte";
  import { shouldArmSleep, sleepRemainingSec } from "../lib/ttsSleep";
  import { getBookOverride, setBookOverride, type BookOverride } from "../lib/bookcfg.svelte";
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

  let {
    current = null as FlatSegment | null,
    bookId = null as number | null,
  }: { current?: FlatSegment | null; bookId?: number | null } = $props();

  // D8：按书记住音色/语速（localStorage，经 bookcfg）；未设置时回退全局。
  $effect(() => {
    ttsSetBook(bookId);
  });
  const bookOv = $derived<BookOverride>(bookId ? getBookOverride(bookId) : {});
  function setBookVoice(v: string) {
    if (!bookId) return;
    setBookOverride(bookId, { ttsVoice: v ? v : undefined });
  }
  function setBookRate(v: number) {
    if (!bookId) return;
    setBookOverride(bookId, { ttsRate: v });
  }
  function resetBookTts() {
    if (!bookId) return;
    setBookOverride(bookId, { ttsVoice: undefined, ttsRate: undefined });
  }

  // 不内置 OpenAI 别名预设（alloy/echo/…）：无固定说话人，不适用于克隆引擎。
  // 音色一律以服务端返回的列表为准。
  const FALLBACK_ONLINE_VOICES: OnlineVoice[] = [];

  let expanded = $state(false);
  /** F5-H1：只做「每秒触发响应」的 tick；剩余时间来自 ttsState.sleepDeadline。 */
  let nowTick = $state(Date.now());
  const sleepLeft = $derived(
    config.tts.sleepMinutes === -1
      ? -1
      : sleepRemainingSec(ttsState.sleepDeadline, nowTick),
  );

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

  // F5-H1：睡眠截止时间放在 ttsState 里跨 TtsBar 挂载保持（章末自动续读 playing=false
  // 会让本组件卸载重挂；若在这里重新 Date.now()+min，倒计时会被重置）。
  $effect(() => {
    const min = config.tts.sleepMinutes;
    void ttsState.playing;
    void ttsState.sleepDeadline;
    // D8：-1 = 章末停止（不跑倒计时，只在条上提示）；Reader 的 onTtsChapterEnd 负责停。
    if (min <= 0) {
      ttsClearSleep();
      return;
    }
    if (
      ttsState.playing &&
      shouldArmSleep(min, ttsState.sleepMinutesArmed, ttsState.sleepDeadline, Date.now())
    ) {
      ttsArmSleep(min);
    }
    // 只有真正启用倒计时后才每秒 tick；组件卸载时定时器随之清理。
    if (ttsState.sleepDeadline > 0) {
      const timer = setInterval(() => {
        nowTick = Date.now();
      }, 1000);
      return () => clearInterval(timer);
    }
  });

  // 到点停止：单独 effect 观察 tick/截止时间，避免在起臂 effect 内部写 ttsState 造成自触发。
  $effect(() => {
    void nowTick;
    if (config.tts.sleepMinutes <= 0) return;
    if (!ttsState.playing) return;
    if (ttsState.sleepDeadline > 0 && sleepRemainingSec(ttsState.sleepDeadline, nowTick) <= 0) {
      ttsStop();
      toast(t("tts.stopTimerDone"));
    }
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

<div class="tts-bar rdr-pop" out:outDissolve class:expanded>
  <button class="tb-btn" data-testid="tts-prev" title={t("tts.prevSentence")} onclick={ttsPrev}><IconPlayerSkipBack /></button>
  <button class="tb-btn primary" data-testid="tts-pause" title={ttsState.paused ? t("tts.resume") : t("tts.pause")} onclick={ttsPauseResume}>
    {#if ttsState.paused}<IconPlayerPlay />{:else}<IconPlayerPause />{/if}
  </button>
  <button class="tb-btn" data-testid="tts-next" title={t("tts.nextSentence")} onclick={ttsNext}><IconPlayerSkipForward /></button>
  <button class="tb-btn stop" data-testid="tts-stop" title={t("tts.stop")} onclick={ttsStop}><IconPlayerStop /></button>

  <div class="tb-info" title={current?.text || ""}>
    <span class="tb-count">
      {ttsState.index >= 0 ? ttsState.index + 1 : 0}/{ttsState.total}
    </span>
    <span class="tb-text">{current?.text || "…"}</span>
    {#if ttsState.buffering}
      <span class="tb-buffering" title={t("tts.generating")}>{t("tts.buffering")}</span>
    {/if}
  </div>

  {#if sleepLeft === -1}
    <span class="tb-sleep" title={t("tts.sleepChapterEnd")}><IconClock /> {t("tts.sleepChapterEnd")}</span>
  {:else if sleepLeft > 0}
    <span class="tb-sleep" title={t("tts.timerRemain")}><IconClock /> {fmtLeft(sleepLeft)}</span>
  {/if}

  <button class="tb-btn" data-testid="tts-settings" title={t("tts.settings")} onclick={() => (expanded = !expanded)}><IconSettings /></button>

  {#if expanded}
    <div class="tb-panel" out:outDissolve>
      <div class="tb-row">
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
            class="tb-mini"
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
      <div class="tb-grid">
        <label>{t("tts.rate")} {config.tts.rate.toFixed(2)}</label>
        <input type="range" min="0.5" max="2" step="0.05" bind:value={config.tts.rate} oninput={saveConfig} />
        <label>{t("tts.pitch")} {config.tts.pitch.toFixed(2)}</label>
        <input type="range" min="0" max="2" step="0.05" bind:value={config.tts.pitch} oninput={saveConfig} />
        <label>{t("tts.volume")} {Math.round(config.tts.volume * 100)}%</label>
        <input type="range" min="0" max="1" step="0.05" bind:value={config.tts.volume} oninput={saveConfig} />
        <label>{t("tts.timerStop")}</label>
        <select data-testid="tts-sleep" bind:value={config.tts.sleepMinutes} onchange={saveConfig}>
          <option value={0}>{t("common.action.close")}</option>
          <option value={-1}>{t("tts.sleepChapterEnd")}</option>
          <option value={5}>{t("tts.minutes", { n: 5 })}</option>
          <option value={10}>{t("tts.minutes", { n: 10 })}</option>
          <option value={15}>{t("tts.minutes", { n: 15 })}</option>
          <option value={30}>{t("tts.minutes", { n: 30 })}</option>
          <option value={60}>{t("tts.minutes", { n: 60 })}</option>
        </select>
      </div>
      <div class="tb-grid tb-book">
        <label>{t("tts.bookVoice")}</label>
        <div style="display:flex; gap:6px; align-items:center;">
          {#if config.tts.engine === "online"}
            <select value={bookOv.ttsVoice ?? ""} onchange={(e) => setBookVoice(e.currentTarget.value)}>
              <option value="">{t("tts.followGlobal")}</option>
              {#each [...onlineVoiceProfiles, ...onlineVoiceOthers] as v (v.id)}
                <option value={v.id}>{v.name || v.id}</option>
              {/each}
            </select>
          {:else}
            <select value={bookOv.ttsVoice ?? ""} onchange={(e) => setBookVoice(e.currentTarget.value)}>
              <option value="">{t("tts.followGlobal")}</option>
              {#each [...zhVoices, ...otherVoices] as v (v.voiceURI)}
                <option value={v.name}>{v.name} · {v.lang}</option>
              {/each}
            </select>
          {/if}
          <button class="tb-mini" title={t("tts.restoreDefault")} onclick={resetBookTts}>↺</button>
        </div>
        <label>{t("tts.bookRate")} {(bookOv.ttsRate ?? config.tts.rate).toFixed(2)}</label>
        <input
          type="range"
          min="0.5"
          max="2"
          step="0.05"
          value={bookOv.ttsRate ?? config.tts.rate}
          oninput={(e) => setBookRate(Number(e.currentTarget.value))}
        />
      </div>
      <div class="tb-checks">
        <label><input type="checkbox" bind:checked={config.tts.highlight} onchange={saveConfig} /> {t("tts.highlightFollow")}</label>
        <label><input type="checkbox" bind:checked={config.tts.autoNextChapter} onchange={saveConfig} /> {t("tts.autoNext")}</label>
      </div>
    </div>
  {/if}
</div>

<style>
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
    animation: tp-rise-in var(--dur) var(--ease-premium) both;
  }

  .tb-btn {
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

  .tb-btn:hover { background: color-mix(in srgb, var(--rdr-fg) 12%, transparent); }

  .tb-btn.primary {
    background: var(--accent);
    color: var(--accent-fg);
    font-size: var(--fs-md);
  }

  .tb-btn.primary:hover { background: var(--accent-hover); }

  .tb-btn.stop { color: var(--danger); }

  .tb-info {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-width: 120px;
    max-width: 340px;
    overflow: hidden;
  }

  .tb-count {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    opacity: 0.65;
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }

  .tb-text {
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    opacity: 0.85;
  }

  .tb-sleep {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    opacity: 0.75;
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
  }
  .tb-buffering {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    opacity: 0.7;
    flex-shrink: 0;
    animation: tb-pulse 1s ease-in-out infinite;
  }

  @keyframes tb-pulse {
    0%, 100% { opacity: 0.35; }
    50% { opacity: 0.85; }
  }

  .tb-mini {
    border: none;
    background: transparent;
    color: inherit;
    font-size: var(--fs-sm);
    padding: 3px var(--sp-1);
    border-radius: var(--radius-pill);
    cursor: pointer;
    flex-shrink: 0;
  }

  .tb-mini:hover { background: color-mix(in srgb, var(--rdr-fg) 12%, transparent); }

  .tb-panel {
    flex-basis: 100%;
    border-top: 1px solid var(--rdr-line);
    margin-top: var(--sp-1);
    padding-top: var(--sp-2);
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    animation: tp-fade-in var(--dur-fast) var(--ease-in) both;
  }

  .tb-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }

  .tb-row label { font-size: var(--fs-xs); letter-spacing: var(--ls-xs); opacity: 0.7; width: 36px; }
  .tb-row select { flex: 1; min-width: 0; font-size: var(--fs-xs); }

  .tb-grid {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 10px;
    align-items: center;
  }

  .tb-grid label { font-size: var(--fs-xs); letter-spacing: var(--ls-xs); opacity: 0.7; white-space: nowrap; }
  .tb-grid input[type="range"] { width: 100%; }
  .tb-grid select { font-size: var(--fs-xs); }

  .tb-checks {
    display: flex;
    gap: var(--sp-4);
    flex-wrap: wrap;
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
  }
</style>
