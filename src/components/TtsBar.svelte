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

<div class="tts-bar" class:expanded>
  <button class="tb-btn" data-testid="tts-prev" title={t("tts.prevSentence")} onclick={ttsPrev}>⏮</button>
  <button class="tb-btn primary" data-testid="tts-pause" title={ttsState.paused ? t("tts.resume") : t("tts.pause")} onclick={ttsPauseResume}>
    {ttsState.paused ? "▶" : "⏸"}
  </button>
  <button class="tb-btn" data-testid="tts-next" title={t("tts.nextSentence")} onclick={ttsNext}>⏭</button>
  <button class="tb-btn stop" data-testid="tts-stop" title={t("tts.stop")} onclick={ttsStop}>⏹</button>

  <div class="tb-info" title={current?.text || ""}>
    <span class="tb-count">
      {ttsState.index >= 0 ? ttsState.index + 1 : 0}/{ttsState.total}
    </span>
    <span class="tb-text">{current?.text || "…"}</span>
    {#if ttsState.buffering}
      <span class="tb-buffering" title={t("tts.generating")}>{t("tts.buffering")}</span>
    {/if}
  </div>

  {#if sleepLeft > 0}
    <span class="tb-sleep" title={t("tts.timerRemain")}>⏰ {fmtLeft(sleepLeft)}</span>
  {/if}

  <button class="tb-btn" data-testid="tts-settings" title={t("tts.settings")} onclick={() => (expanded = !expanded)}>⚙</button>

  {#if expanded}
    <div class="tb-panel">
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
          >{ttsState.onlineVoiceLoading ? "…" : "↻"}</button>
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
          <option value={5}>{t("tts.minutes", { n: 5 })}</option>
          <option value={10}>{t("tts.minutes", { n: 10 })}</option>
          <option value={15}>{t("tts.minutes", { n: 15 })}</option>
          <option value={30}>{t("tts.minutes", { n: 30 })}</option>
          <option value={60}>{t("tts.minutes", { n: 60 })}</option>
        </select>
      </div>
      <div class="tb-checks">
        <label><input type="checkbox" bind:checked={config.tts.highlight} onchange={saveConfig} /> {t("tts.highlightFollow")}</label>
        <label><input type="checkbox" bind:checked={config.tts.autoNextChapter} onchange={saveConfig} /> {t("tts.autoNext")}</label>
      </div>
    </div>
  {/if}
</div>

<style>
  .tts-bar {
    position: fixed;
    left: 50%;
    bottom: 46px;
    transform: translateX(-50%);
    z-index: 70;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    max-width: min(760px, 92vw);
    background: color-mix(in srgb, var(--rdr-bg, #f5f1e8) 90%, #202020 10%);
    color: var(--rdr-fg, #2c2c2c);
    border: 1px solid rgba(128, 128, 128, 0.3);
    border-radius: 22px;
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.16), 0 20px 48px rgba(0, 0, 0, 0.26);
    flex-wrap: wrap;
  }

  .tb-btn {
    border: none;
    background: transparent;
    color: inherit;
    font-size: 16px;
    line-height: 1;
    padding: 6px 8px;
    border-radius: 50%;
    cursor: pointer;
  }

  .tb-btn:hover { background: rgba(128, 128, 128, 0.18); }

  .tb-btn.primary {
    background: var(--accent, #4a6b8a);
    color: #fff;
    font-size: 14px;
  }

  .tb-btn.stop { color: var(--danger, #c0392b); }

  .tb-info {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 120px;
    max-width: 340px;
    overflow: hidden;
  }

  .tb-count {
    font-size: 12px;
    opacity: 0.65;
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }

  .tb-text {
    font-size: 13px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    opacity: 0.85;
  }

  .tb-sleep {
    font-size: 12px;
    opacity: 0.75;
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }

  .tb-buffering {
    font-size: 12px;
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
    font-size: 13px;
    padding: 3px 6px;
    border-radius: 6px;
    cursor: pointer;
    flex-shrink: 0;
  }

  .tb-mini:hover { background: rgba(128, 128, 128, 0.18); }

  .tb-panel {
    flex-basis: 100%;
    border-top: 1px solid rgba(128, 128, 128, 0.2);
    margin-top: 6px;
    padding-top: 8px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .tb-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .tb-row label { font-size: 12px; opacity: 0.7; width: 36px; }
  .tb-row select { flex: 1; min-width: 0; font-size: 12px; }

  .tb-grid {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 10px;
    align-items: center;
  }

  .tb-grid label { font-size: 12px; opacity: 0.7; white-space: nowrap; }
  .tb-grid input[type="range"] { width: 100%; }
  .tb-grid select { font-size: 12px; }

  .tb-checks {
    display: flex;
    gap: 16px;
    flex-wrap: wrap;
    font-size: 12px;
  }
</style>
