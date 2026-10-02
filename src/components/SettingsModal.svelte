<script lang="ts">
  import { api } from "../lib/api";
  import {
    config,
    saveConfig,
    saveConfigNow,
    newTtsService,
    THEMES,
    toast,
    ui,
    APP_VERSION,
    copyText,
    AI_PROVIDERS,
    aiProviderMeta,
    newAiProfile,
    BIND_DEFS,
    comboOf,
    DEFAULT_BINDINGS,
    ZONE_DEFS,
    DEFAULT_ZONES,
    appConfirm,
    defaultReaderConfig,
    resetReaderConfig,
    SCROLL_BOUNCE_PRESETS,
    HIGHLIGHT_STYLES,
    applyUiTheme,
    UI_SKINS,
    coverTheme,
    bumpDefaultCoverBust,
    createReaderScheme,
    overwriteReaderScheme,
    renameReaderScheme,
    deleteReaderScheme,
    applyReaderSchemeGlobal,
    type UiTheme,
    type ReaderScheme,
    type AiProvider,
    type TtsService,
    type BindAction,
    type ZoneAction,
    type StatusItemId,
    STATUS_ITEM_DEFS,
    AI_GROUPS,
    type AiGroup,
    stripSecrets,
    applySecrets,
    applyConfigJson,
    runSettingsBackup,
    builtinSchemeNames,
    SEED_SCHEME_NAME_KEYS,
  } from "../lib/stores.svelte";
  import type { Icon as TablerIcon } from "@tabler/icons-svelte";
  import {
    IconCloud,
    IconColorSwatch,
    IconAdjustments,
    IconKeyboard,
    IconPhoto,
    IconScissors,
    IconSettings,
    IconSparkles,
    IconTypography,
    IconVolume,
  } from "@tabler/icons-svelte";
  import ColorPicker from "./ColorPicker.svelte";
  import { dragSort } from "../lib/dragSort";
  import { slide } from "svelte/transition";
  // i18n：第一阶段仅「通用 → 界面语言」这一小块接线（其余分区文案留给后续阶段）。
  import { dateLocale, errText, hasLabelOf, labelOfId, localeSetting, setLocale, t } from "../lib/i18n/index.svelte";
  import type { MessageKey } from "../lib/i18n/types";
  import { onMount, tick, untrack } from "svelte";
  import { getBookOverride, setBookOverride } from "../lib/bookcfg.svelte";
  import { refreshOnlineVoices, refreshVoices, ttsState } from "../lib/tts.svelte";
  import type { OnlineVoice } from "../lib/tts.svelte";
  import type { TocEntry } from "../lib/types";

  type SettingsTab = "appearance" | "reader" | "manga" | "split" | "keys" | "tts" | "webdav" | "ai" | "prefs" | "other";
  // 默认不硬编码分区：initialTab 不传时用「排序后的第一项」，以区分调用方显式指定与未指定。
  let { initialTab = undefined as SettingsTab | undefined }: { initialTab?: SettingsTab } = $props();

  /** 设置各分区的元信息（扁平列表，无分组）：标签 + 线性图标。 */
  // 分区 label 走 labelKey（t() 渲染时才求值，故切语言即时生效）；持久化只存 id。
  const TAB_META: Record<SettingsTab, { labelKey: MessageKey; icon: TablerIcon }> = {
    reader: { labelKey: "settings.tab.reader", icon: IconTypography },
    appearance: { labelKey: "settings.tab.appearance", icon: IconColorSwatch },
    manga: { labelKey: "settings.tab.manga", icon: IconPhoto },
    tts: { labelKey: "settings.tab.tts", icon: IconVolume },
    split: { labelKey: "settings.tab.split", icon: IconScissors },
    ai: { labelKey: "settings.tab.ai", icon: IconSparkles },
    keys: { labelKey: "settings.tab.keys", icon: IconKeyboard },
    webdav: { labelKey: "settings.tab.webdav", icon: IconCloud },
    prefs: { labelKey: "settings.tab.prefs", icon: IconAdjustments },
    other: { labelKey: "settings.tab.other", icon: IconSettings },
  };

  /** 按持久化顺序排列的扁平分区列表（settingsOrder 已归一化，含全部合法 id）。 */
  const navItems = $derived(
    config.settingsOrder
      .filter((id): id is SettingsTab => id in TAB_META)
      .map((id) => ({ id, labelKey: TAB_META[id].labelKey, icon: TAB_META[id].icon })),
  );

  // 打开机制：+page.svelte 两处入口都在 {#if ui.settingsOpen} 内，每次打开重新挂载，
  // 故此初始化即按当前 settingsOrder 生效——显式传入（如 AI 入口 initialTab="ai"）优先，
  // 否则取排序后的第一项；orderedTabs 为空时兜底 "reader"。
  let tab = $state<SettingsTab>(initialTab ?? navItems[0]?.id ?? "reader");

  /** 当前标签的显示名（用于头部面包屑）。 */
  const tabLabel = $derived(TAB_META[tab] ? t(TAB_META[tab].labelKey) : t("settings.title"));

  /** 导航搜索过滤（空则全部显示）。 */
  let navQuery = $state("");
  const navFiltered = $derived(
    navQuery.trim()
      ? navItems.filter((i) => t(i.labelKey).toLowerCase().includes(navQuery.trim().toLowerCase()))
      : navItems,
  );

  /* ---------- 具体设置项搜索：索引 / 结果 / 定位 ---------- */
  type SearchHit = { tab: SettingsTab; text: string; el: HTMLElement };
  let searchIndex = $state<SearchHit[]>([]);
  const sq = (s: string) => (s || "").replace(/\s+/g, " ").trim().toLowerCase();

  /** 全文设置索引：各 pane 常驻 DOM（含隐藏的），以每个 <label> 文本为条目。 */
  function buildSearchIndex() {
    const root = document.querySelector(".set-content");
    if (!root) return;
    const hits: SearchHit[] = [];
    root.querySelectorAll<HTMLElement>(".set-pane[data-pane]").forEach((pane) => {
      const t = pane.dataset.pane as SettingsTab;
      pane.querySelectorAll<HTMLElement>("label").forEach((lb) => {
        const text = (lb.textContent || "").replace(/\s+/g, " ").trim();
        if (!text) return;
        const el = (lb.closest(".field") as HTMLElement | null) ?? lb.parentElement ?? lb;
        hits.push({ tab: t, text, el });
      });
    });
    searchIndex = hits;
  }

  // 组件每次打开都重挂载，onMount 重建索引，正好覆盖 AI 分类默认展开的状态。
  onMount(buildSearchIndex);

  /** 搜索结果：仅取匹配的具体设置项（上限 80 条）。 */
  const searchResults = $derived.by(() => {
    const q = sq(navQuery);
    if (!q) return [];
    return searchIndex.filter((h) => sq(h.text).includes(q)).slice(0, 80);
  });

  /** 点结果：切大项 → 展开 AI 分类 → 滚动到该项并短暂高亮。 */
  async function locateSetting(h: SearchHit) {
    tab = h.tab;
    // AI 分类若被折叠过，先展开，保证字段在 DOM 中
    if (h.tab === "ai") aiOpen = { node: true, window: true, prompt: true };
    await tick();
    let el: HTMLElement | null = h.el;
    if (!el.isConnected) {
      // 索引建立后节点被重建（如折叠/条件变化）→ 按文本回退查找
      const pane = document.querySelector<HTMLElement>(`.set-pane[data-pane="${h.tab}"]`);
      const target = sq(h.text);
      el =
        (Array.from(pane?.querySelectorAll<HTMLElement>("label") ?? [])
          .find((lb) => sq(lb.textContent || "") === target)
          ?.closest(".field") as HTMLElement | null) ?? null;
    }
    if (!el) return;
    el.scrollIntoView({ block: "center", behavior: "smooth" });
    el.classList.add("fr-locate");
    setTimeout(() => el?.classList.remove("fr-locate"), 1000);
  }

  /** 分区拖拽排序：把 from 插入到 target 之前/之后，持久化到 config.settingsOrder（与顶栏图标同款 dragSort）。 */
  function navDrop(from: string, target: string, place: "before" | "after") {
    if (!from || from === target) return;
    const arr = [...config.settingsOrder];
    const fi = arr.indexOf(from);
    if (fi < 0) return;
    arr.splice(fi, 1);
    const ti = arr.indexOf(target);
    if (ti < 0) return;
    arr.splice(place === "after" ? ti + 1 : ti, 0, from);
    config.settingsOrder = arr;
    saveConfig();
  }
  /** 漫画默认倍率滑块：先保存全局设置；若正从「正在阅读的书」打开设置，则清除本书 imgScale 覆盖，
   *  使当前书跟随新全局值。书库（currentBookId=0）或其它书不受影响。 */
  function onMangaScaleInput() {
    saveConfig();
    if (ui.currentBookId) setBookOverride(ui.currentBookId, { imgScale: undefined });
  }
  /** 漫画「适宽 / 适高」互斥：开一个自动关另一个。 */
  function setMangaFit(which: "fitWidth" | "fitHeight", v: boolean) {
    if (which === "fitWidth") {
      config.reader.manga.fitWidth = v;
      if (v) config.reader.manga.fitHeight = false;
    } else {
      config.reader.manga.fitHeight = v;
      if (v) config.reader.manga.fitWidth = false;
    }
    saveConfig();
  }
  let webdavTesting = $state(false);
  let webdavStatus = $state("");
  let regexText = $state(config.regex || "");
  let rebuildingFts = $state(false);
  let dataDir = $state("");
  /** 存储统计（数据目录/封面/提取资源/字体/数据库/可清理缓存）。 */
  type StorageStats = {
    data_dir: string;
    total_bytes: number;
    covers_bytes: number;
    res_bytes: number;
    fonts_bytes: number;
    db_bytes: number;
    cache_bytes: number;
  };
  let storage = $state<StorageStats | null>(null);
  let storageBusy = $state(false);

  /* ---------- 默认封面底图设置 ---------- */
  let hasDefaultCover = $state(false);
  let coverPreviewBust = $state(0);
  let coverBusy = $state(false);

  async function loadDefaultCoverState() {
    try {
      hasDefaultCover = await api.hasDefaultCover();
      coverPreviewBust++;
    } catch { /* ignore */ }
  }

  /** 预览：用共享端点渲染一张样例封面（底图 + 实时叠字）；未设底图时即主题渐变。
   *  带上主题配色参数，预览与书库默认封面完全一致，且切主题即时更新。 */
  // i18n-allow:纯 URL 拼接、无可翻译文案（+ 连接的模板字面量会被守门扫描器合并成一段，属已知误报）
  const coverPreviewUrl = $derived(
    `http://tidepage.localhost/cover-default/0?v=${coverPreviewBust}` +
      `&g1=${encodeURIComponent(coverTheme.g1)}&g2=${encodeURIComponent(coverTheme.g2)}` +
      `&g3=${encodeURIComponent(coverTheme.g3)}&tc=${encodeURIComponent(coverTheme.tc)}`, // i18n-allow:纯 URL 拼接、无可翻译文案（+ 连接的模板字面量会被守门扫描器合并成一段，属已知误报）
  ); // i18n-allow:纯 URL 拼接、无可翻译文案（+ 连接的模板字面量会被守门扫描器合并成一段，属已知误报）

  async function pickDefaultCover() {
    const p = await api.pickFile(t("settings.cover.pickDlg"), [
      { name: t("settings.fileDlg.image"), exts: ["png", "jpg", "jpeg", "webp"] },
    ]);
    if (!p) return;
    coverBusy = true;
    try {
      await api.setDefaultCover(p);
      hasDefaultCover = true;
      bumpDefaultCoverBust();
      // 触发书库既有封面刷新链路：重拉可见窗口行数据（新对象身份），
      // 与 {#key coverUrl} 一起确保无内嵌封面的书立即重挂 <img> 换新底图，无需滚动。
      ui.coverTick++;
      coverPreviewBust++;
      toast(t("settings.cover.updated"));
    } catch (e) {
      toast(String(e), "error");
    } finally {
      coverBusy = false;
    }
  }

  async function resetDefaultCover() {
    coverBusy = true;
    try {
      await api.clearDefaultCover();
      hasDefaultCover = false;
      bumpDefaultCoverBust();
      // 同 pickDefaultCover：触发书库可见窗口重拉行数据，配合 {#key coverUrl} 即时重挂封面。
      ui.coverTick++;
      coverPreviewBust++;
      toast(t("settings.cover.restored"));
    } catch (e) {
      toast(String(e), "error");
    } finally {
      coverBusy = false;
    }
  }

  function fmtBytes(n: number): string {
    if (!n) return "0 B";
    const mb = n / 1048576;
    if (mb >= 1024) return (mb / 1024).toFixed(2) + " GB";
    if (mb >= 1) return mb.toFixed(1) + " MB";
    return Math.max(1, Math.round(n / 1024)) + " KB";
  }

  async function loadStorage() {
    try {
      storage = await api.storageStats();
      if (storage?.data_dir) dataDir = storage.data_dir;
    } catch { /* ignore */ }
  }

  async function clearCache(what: "covers" | "res" | "all") {
    const label = what === "covers" ? t("settings.cache.covers") : what === "res" ? t("settings.cache.res") : t("settings.cache.all");
    const ok = await appConfirm(t("settings.cache.confirm", { label }));
    if (!ok) return;
    storageBusy = true;
    try {
      const freed = await api.clearCache(what);
      toast(t("settings.cache.cleared", { label, size: fmtBytes(freed) }));
      await loadStorage();
    } catch (e) {
      toast(String(e), "error");
    } finally {
      storageBusy = false;
    }
  }
  let mobiEnc = $state("auto");

  /** 设置界面主题并立即生效（config 为响应式，$effect 也会同步）。 */
  function setUiTheme(t: UiTheme) {
    config.uiTheme = t;
    saveConfig();
    applyUiTheme();
  }

  /** 选择指定明暗基调所用的外壳皮肤。 */
  function setUiSkin(skinId: string) {
    const skin = UI_SKINS.find((s) => s.id === skinId);
    if (!skin) return;
    if (skin.mode === "dark") config.uiSkinDark = skinId;
    else config.uiSkinLight = skinId;
    saveConfig();
    applyUiTheme();
  }

  /* ---------- 阅读配色方案 ---------- */
  let schemeSaveOpen = $state(false);
  let schemeNameInput = $state("");

  /** 判断某方案是否为"当前书正在使用"的（近似：与书覆盖一致则高亮）。 */
  function schemeActive(s: ReaderScheme): boolean {
    const r = config.reader;
    return (
      r.theme === s.cfg.theme &&
      r.bgColor === s.cfg.bgColor &&
      r.fgColor === s.cfg.fgColor &&
      r.fontSize === s.cfg.fontSize &&
      r.lineHeight === s.cfg.lineHeight
    );
  }

  /** 本书是否已被单独套用方案（存在按书覆盖的阅读参数）。 */
  function bookSchemeNote(): boolean {
    const id = ui.currentBookId;
    if (!id) return false;
    const o = getBookOverride(id);
    return o.schemeTheme !== undefined;
  }

  /** 清除本书单独套用的方案，回到全局设置（全局字体/配色此后对本书生效）。 */
  function clearBookScheme() {
    const id = ui.currentBookId;
    if (!id) return;
    setBookOverride(id, {
      fontSize: undefined,
      overrideBookCss: undefined,
      schemeTheme: undefined,
      schemeBg: undefined,
      schemeFg: undefined,
      schemeFontFamily: undefined,
      schemeLatinFont: undefined,
      schemeLineHeight: undefined,
      schemeParaSpacing: undefined,
      schemeIndent: undefined,
      schemeTextAlign: undefined,
      schemeBgImage: undefined,
      schemeBgOpacity: undefined,
      schemeCssExtra: undefined,
      schemeFontWeight: undefined,
    });
    toast(t("settings.scheme.cleared"));
  }

  /**
   * 配色方案（config.readerSchemes）的显示名。**用户数据优先**：只有「名字还是我们给的那个」
   * 才用译文，用户改过就以用户的名字为准。三条分流：
   *  - 早期种子名「默认」（stores 的 SEED_SCHEME_NAME_KEYS）：是我们的文案不是用户命名 → 查表；
   *  - 内置 5 套且 name 仍等于 builtinSchemeNames()[id]：查表。内置 id 与文案表键后缀**逐字相同**
   *    （s-paper / s-modern / s-green / s-night / s-ink），默认名由 builtinSchemes() 单一来源给出；
   *  - 其余（用户自建、已改名）：**原样显示用户自己的名字**，绝不查表、绝不翻译。
   *  全程只读不写：config.readerSchemes[].name 一个字节都不改。
   */
  function schemeLabel(s: ReaderScheme): string {
    const seed = SEED_SCHEME_NAME_KEYS[s.name];
    if (seed !== undefined && hasLabelOf("scheme", seed)) return labelOfId("scheme", seed);
    const def = builtinSchemeNames()[s.id];
    if (def !== undefined && s.name === def && hasLabelOf("scheme", s.id)) return labelOfId("scheme", s.id);
    return s.name;
  }

  function openSchemeSave() {
    schemeSaveOpen = true;
    schemeNameInput = "";
  }

  function openSchemeNew() {
    const id = createReaderScheme(schemeNameInput || t("settings.scheme.defaultName"));
    schemeNameInput = "";
    schemeSaveOpen = false;
    toast(t("settings.scheme.created", { name: config.readerSchemes.find((s) => s.id === id)?.name ?? "" }));
  }

  function saveOverScheme(s: ReaderScheme) {
    overwriteReaderScheme(s.id);
    schemeSaveOpen = false;
    toast(t("settings.scheme.overwritten", { name: s.name }));
  }

  function renameScheme(s: ReaderScheme) {
    const n = window.prompt(t("settings.scheme.renamePrompt"), s.name);
    if (n == null) return;
    renameReaderScheme(s.id, n);
  }

  async function delScheme(s: ReaderScheme) {
    const ok = await appConfirm(t("settings.scheme.delConfirm", { name: s.name }));
    if (!ok) return;
    deleteReaderScheme(s.id);
    toast(t("settings.scheme.deleted"));
  }

  /** 应用方案：global=写全局；book=只给当前书（无打开书时退化为全局）。 */
  function applyScheme(id: string, scope: "global" | "book") {
    const s = config.readerSchemes.find((x) => x.id === id);
    if (!s) return;
    if (scope === "global" || !ui.currentBookId) {
      applyReaderSchemeGlobal(id);
      toast(t("settings.scheme.appliedGlobal", { name: s.name }));
      return;
    }
    // 仅当前书：把方案快照写入这本书的覆盖。
    applySchemeToBook(s);
    toast(t("settings.scheme.appliedBook", { name: s.name }));
  }

  function applySchemeToBook(s: ReaderScheme) {
    const id = ui.currentBookId;
    if (!id) return;
    const c = s.cfg;
    setBookOverride(id, {
      fontSize: c.fontSize,
      overrideBookCss: c.overrideBookCss,
      schemeTheme: c.theme,
      schemeBg: c.bgColor,
      schemeFg: c.fgColor,
      schemeFontFamily: c.fontFamily,
      schemeLatinFont: c.latinFont,
      schemeLineHeight: c.lineHeight,
      schemeParaSpacing: c.paraSpacing,
      schemeIndent: c.indent,
      schemeTextAlign: c.textAlign,
      schemeBgImage: c.bgImage,
      schemeBgOpacity: c.bgOpacity,
      schemeCssExtra: c.cssExtra,
      schemeFontWeight: c.fontWeight,
    });
  }

  let ttsVoiceList = $state<OnlineVoice[]>([]);
  let ttsVoiceLoading = $state(false);
  let ttsVoiceMsg = $state("");

  const ttsVoiceProfiles = $derived(ttsVoiceList.filter((v) => v.type === "profile"));
  // 隐藏 OpenAI 别名：无固定说话人，不适用于克隆引擎。
  const ttsVoiceOthers = $derived(
    ttsVoiceList.filter((v) => v.type !== "profile" && v.type !== "openai_alias")
  );

  /** 各接口协议的文案提示与默认值（切换协议时用于补空字段）。 */
  interface TtsProviderMeta {
    id: string;
    /** 界面文案走 t()：labelKey 恒有；model/voiceHint 仅在含中文说明时给 *HintKey，其余是纯取值（模型名、音色 id 等）。
     *  **字段归属按后端实际取值定，别凭字段名下结论**（src-tauri/src/ai.rs）：
     *  gptsovits 的参考音频路径、baidu 的音色编号、breeze 的音色 id 都取自 `voice`
     *  （`ref_audio_path = p.voice` / `per = p.voice` / `fetch_breeze_voice_at(root, p.voice)`），
     *  三者的 `model` 只有 breeze 用来指定模型名、另两个根本不读 → 所以
     *  voiceHintKey 给「参考音频路径/音色 id/音色编号」，modelHintKey 给各自的实话（无模型名 / 模型名）。 */
    labelKey: MessageKey;
    baseHint: string;
    modelHintKey?: MessageKey;
    voiceHintKey?: MessageKey;
    modelHint: string;
    voiceHint: string;
    defaults: { baseUrl: string; model: string; voice: string; format: string };
    /** 是否支持从服务读取音色列表（否则音色手填）。 */
    voiceList: boolean;
    /** 是否隐藏密钥/模型（Edge 固定接口为在线公共端点，无需二者）。 */
    hideKeyModel: boolean;
  }

  const TTS_PROVIDER_METAS: TtsProviderMeta[] = [
    { id: "openai", labelKey: "dict.ttsProtocol.openai", baseHint: "http://127.0.0.1:3900", modelHint: "omnivoice", modelHintKey: undefined, voiceHint: "", voiceHintKey: "dict.ttsModel.openai", defaults: { baseUrl: "http://127.0.0.1:3900", model: "omnivoice", voice: "", format: "wav" }, voiceList: true, hideKeyModel: false },
    { id: "voicestudio", labelKey: "dict.ttsProtocol.voicestudio", baseHint: "http://127.0.0.1:3900", modelHint: "tts-1", modelHintKey: "dict.ttsModel.voicestudio", voiceHint: "", voiceHintKey: "dict.ttsModel.voicestudio", defaults: { baseUrl: "http://127.0.0.1:3900", model: "tts-1", voice: "", format: "wav" }, voiceList: true, hideKeyModel: false },
    { id: "azure", labelKey: "dict.ttsProtocol.azure", baseHint: "https://<resource name>.openai.azure.com", modelHint: "", modelHintKey: "dict.ttsModel.azure", voiceHint: "", voiceHintKey: "dict.ttsModel.common", defaults: { baseUrl: "", model: "", voice: "", format: "mp3" }, voiceList: false, hideKeyModel: false },
    { id: "dashscope", labelKey: "dict.ttsProtocol.dashscope", baseHint: "https://dashscope.aliyuncs.com", modelHint: "qwen3-tts-flash", voiceHint: "Cherry", defaults: { baseUrl: "https://dashscope.aliyuncs.com", model: "qwen3-tts-flash", voice: "Cherry", format: "mp3" }, voiceList: false, hideKeyModel: false },
    { id: "minimax", labelKey: "dict.ttsProtocol.minimax", baseHint: "https://api.minimax.chat", modelHint: "speech-2.6-hd", voiceHint: "male-qn-qingse", defaults: { baseUrl: "https://api.minimax.chat", model: "speech-2.6-hd", voice: "male-qn-qingse", format: "mp3" }, voiceList: false, hideKeyModel: false },
    { id: "fish", labelKey: "dict.ttsProtocol.fish", baseHint: "https://api.fish.audio", modelHint: "s1", voiceHint: "reference_id", defaults: { baseUrl: "https://api.fish.audio", model: "s1", voice: "", format: "mp3" }, voiceList: false, hideKeyModel: false },
    { id: "gptsovits", labelKey: "dict.ttsProtocol.gptsovits", baseHint: "http://127.0.0.1:9880", modelHint: "", modelHintKey: "dict.ttsModel.gptsovitsNone", voiceHint: "", voiceHintKey: "dict.ttsModel.gptsovits", defaults: { baseUrl: "http://127.0.0.1:9880", model: "", voice: "", format: "wav" }, voiceList: false, hideKeyModel: false },
    { id: "breeze", labelKey: "dict.ttsProtocol.breeze", baseHint: "http://127.0.0.1:7863", modelHint: "", modelHintKey: "dict.ttsModel.breezeModel", voiceHint: "", voiceHintKey: "dict.ttsModel.breeze", defaults: { baseUrl: "http://127.0.0.1:7863", model: "", voice: "", format: "wav" }, voiceList: true, hideKeyModel: false },
    { id: "volcano", labelKey: "dict.ttsProtocol.volcano", baseHint: "https://openspeech.bytedance.com/api/v1/tts", modelHint: "", modelHintKey: "dict.ttsModel.volcano", voiceHint: "zh_female_cancan_mars_bigtts", voiceHintKey: undefined, defaults: { baseUrl: "https://openspeech.bytedance.com/api/v1/tts", model: "", voice: "", format: "mp3" }, voiceList: false, hideKeyModel: false },
    { id: "baidu", labelKey: "dict.ttsProtocol.baidu", baseHint: "https://tsn.baidu.com", modelHint: "", modelHintKey: "dict.ttsModel.baiduNone", voiceHint: "", voiceHintKey: "dict.ttsModel.baidu", defaults: { baseUrl: "https://tsn.baidu.com", model: "", voice: "", format: "mp3" }, voiceList: false, hideKeyModel: false },
    { id: "edge", labelKey: "dict.ttsProtocol.edge", baseHint: "", modelHint: "", modelHintKey: undefined, voiceHint: "", voiceHintKey: "dict.ttsModel.common", defaults: { baseUrl: "", model: "", voice: "", format: "mp3" }, voiceList: true, hideKeyModel: true },
  ];

  function ttsProviderMeta(id: string): TtsProviderMeta {
    return TTS_PROVIDER_METAS.find((m) => m.id === id) ?? TTS_PROVIDER_METAS[0];
  }

  type TtsField = "baseUrl" | "model" | "voice" | "format";

  /** 该字段值是否等于「任一协议的默认值」：若是则视为未自定义（等价于空），
   *  切换协议时可安全替换为新协议默认值，避免残留旧协议地址/模型/音色。 */
  function isTtsDefaultValue(field: TtsField, value: string): boolean {
    const v = (value ?? "").trim();
    return TTS_PROVIDER_METAS.some((m) => (m.defaults[field] || "").trim() === v);
  }

  /** 当前语音服务档案。 */
  const tsvc = $derived(
    config.tts.services[Math.min(config.tts.activeService, config.tts.services.length - 1)] ??
      config.tts.services[0]
  );

  const tsvcMeta = $derived(ttsProviderMeta(tsvc?.provider || "openai"));

  function addTtsService() {
    const list = config.tts.services;
    list.push(newTtsService(list));
    config.tts.activeService = list.length - 1;
    saveConfigNow();
    loadTtsVoices();
  }

  function delTtsService() {
    const list = config.tts.services;
    if (list.length <= 1) {
      toast(t("settings.tts.keepOne"), "error");
      return;
    }
    const i = Math.min(Math.max(config.tts.activeService, 0), list.length - 1);
    list.splice(i, 1);
    config.tts.activeService = Math.max(0, Math.min(i, list.length - 1));
    saveConfigNow();
    loadTtsVoices();
  }

  /** 切换接口协议：空值或仍为「某协议默认值」的字段一律替换为新协议默认值；
   *  固定接口协议（火山/百度/Edge）的地址每次切换都直接写死（Edge 为空串），
   *  不残留其它协议地址。secretKey/appId/cluster 保持不变。 */
  function onTtsProtocolChange(s: TtsService) {
    const m = ttsProviderMeta(s.provider);
    const fixedBase =
      s.provider === "volcano" || s.provider === "baidu" || s.provider === "edge";
    const fill = (field: TtsField): string => {
      const cur = s[field] ?? "";
      if (field === "baseUrl" && fixedBase) return m.defaults.baseUrl;
      return !cur.trim() || isTtsDefaultValue(field, cur) ? m.defaults[field] : cur;
    };
    s.baseUrl = fill("baseUrl");
    s.model = fill("model");
    s.voice = fill("voice");
    s.format = fill("format");
    // 切换协议后旧音色列表不再适用，清空重读。
    ttsVoiceList = [];
    ttsVoiceMsg = "";
    saveConfigNow();
    loadTtsVoices();
  }

  async function loadTtsVoices() {
    const s = tsvc;
    if (!s) return;
    const m = ttsProviderMeta(s.provider);
    if (!m.voiceList) {
      ttsVoiceList = [];
      ttsState.onlineVoices = [];
      ttsVoiceMsg = t("settings.tts.noVoiceList");
      return;
    }
    if (!(s.baseUrl || "").trim() && s.provider !== "edge") return;
    ttsVoiceLoading = true;
    ttsVoiceMsg = "";
    try {
      // 列表同时写入 ttsState.onlineVoices，供阅读界面播放条复用。
      ttsVoiceList = await refreshOnlineVoices();
      ttsVoiceMsg = ttsVoiceList.length
        ? t("settings.tts.voiceCount", { n: ttsVoiceList.length })
        : t("settings.tts.noVoiceList");
    } finally {
      ttsVoiceLoading = false;
    }
  }

  $effect(() => {
    if (tab !== "tts") return;
    // 依赖当前档案、协议与地址：切换服务/协议或改地址后自动重新读取音色。
    void config.tts.activeService;
    void tsvc?.provider;
    void tsvc?.baseUrl;
    loadTtsVoices();
  });

  $effect(() => {
    if (tab !== "prefs") return;
    api
      .getSetting("mobi.encoding")
      .then((v) => (mobiEnc = v && v.trim() ? v.trim().toLowerCase() : "auto"))
      .catch(() => {});
  });

  $effect(() => {
    if (tab !== "appearance") return;
    loadDefaultCoverState();
  });

  async function saveMobiEnc() {
    try {
      await api.setMobiEncoding(mobiEnc);
      toast(t("settings.tts.encodingDone"));
    } catch (e) {
      toast(String(e), "error");
    }
  }

  /* ---------- 顶栏 / 底栏 状态项 ---------- */
  function topEnabled(id: StatusItemId): boolean {
    return config.reader.statusItems.some((x) => x.id === id && x.enabled);
  }
  function toggleTopItem(id: StatusItemId) {
    const list = config.reader.statusItems.map((x) => ({ ...x }));
    const it = list.find((x) => x.id === id);
    if (it) it.enabled = !it.enabled;
    else list.push({ id, enabled: true });
    config.reader.statusItems = list;
    saveConfig();
  }
  function footerList(side: "left" | "right"): StatusItemId[] {
    return side === "left" ? config.reader.footerLeft : config.reader.footerRight;
  }
  function footerHas(side: "left" | "right", id: StatusItemId): boolean {
    return footerList(side).includes(id);
  }
  function toggleFooter(side: "left" | "right", id: StatusItemId) {
    const cur = [...footerList(side)];
    const i = cur.indexOf(id);
    if (i >= 0) cur.splice(i, 1);
    else cur.push(id);
    if (side === "left") config.reader.footerLeft = cur;
    else config.reader.footerRight = cur;
    saveConfig();
  }
  function moveFooter(side: "left" | "right", id: StatusItemId, dir: -1 | 1) {
    const cur = [...footerList(side)];
    const i = cur.indexOf(id);
    const j = i + dir;
    if (i < 0 || j < 0 || j >= cur.length) return;
    [cur[i], cur[j]] = [cur[j], cur[i]];
    if (side === "left") config.reader.footerLeft = cur;
    else config.reader.footerRight = cur;
    saveConfig();
  }

  $effect(() => {
    if (tab !== "prefs") return;
    loadStorage();
  });

  async function exportConfig() {
    try {
      // 导出内容不含明文密钥：config 已脱敏，密钥以 DPAPI 密文单列。
      const secret = await api.getSetting("secret.config").catch(() => null);
      const payload = { v: 1, config: stripSecrets(config), secret: secret ?? null };
      await copyText(JSON.stringify(payload, null, 2));
      toast(t("settings.backup.copied"));
    } catch (e) {
      toast(String(e), "error");
    }
  }

  async function importConfig() {
    const ok = await appConfirm(t("settings.backup.importConfirm"));
    if (!ok) return;
    try {
      const txt = await navigator.clipboard.readText();
      const parsed = JSON.parse(txt);
      // 兼容新格式 { v, config, secret } 与旧版「直接是完整配置」两种。
      const obj =
        parsed && typeof parsed === "object" && parsed.config && typeof parsed.config === "object"
          ? parsed.config
          : parsed;
      if (!obj || typeof obj !== "object" || !obj.reader) throw new Error(t("settings.backup.invalid"));
      Object.assign(config, {
        ...obj,
        reader: { ...config.reader, ...obj.reader },
        ai: {
          ...config.ai,
          ...(obj.ai || {}),
          style: { ...config.ai.style, ...(obj.ai?.style || {}) },
        },
      });
      // 密文原样写回（DPAPI 才能解），再解密回填。
      const secret = parsed && typeof parsed === "object" ? (parsed as any).secret : undefined;
      if (typeof secret === "string" && secret) {
        try {
          await api.setSetting("secret.config", secret);
          const raw = await api.secretRead();
          if (raw) applySecrets(config, JSON.parse(raw) as Record<string, unknown>);
        } catch {
          /* 密钥与当前机器不匹配时静默跳过 */
        }
      }
      saveConfigNow();
      toast(t("settings.backup.imported"));
    } catch (e) {
      toast(t("settings.backup.importFailed", { err: String(e) }), "error");
    }
  }

  async function rebuildIndex() {
    rebuildingFts = true;
    try {
      const n = await api.rebuildFts();
      toast(t("settings.backup.indexRebuilt", { n }));
    } catch (e) {
      toast(String(e), "error");
    } finally {
      rebuildingFts = false;
    }
  }

  function copyDataDir() {
    copyText(dataDir || "%APPDATA%\\com.a.tidepage");
    toast(t("settings.backup.dataDirCopied"));
  }

  const SPLIT_PRESETS: { id: string; labelKey: MessageKey; pattern: string }[] = [
    {
      id: "chapterSpecial",
      labelKey: "dict.splitPreset.chapterSpecial",
      pattern:
        // i18n-allow:正则字面量：要匹配的正是中文标题词，翻译它会直接改变匹配行为。
        "(?<=[　\\s])(?:(?:内容|文章)?简介|文案|前言|序章|楔子|正文(?!完|结)|终章|后记|番外|第?\\s*[\\d〇零一二两三四五六七八九十百千万壹贰叁肆伍陆柒捌玖拾佰仟]+\\s*(?:卷|章))(?![\\w\\d]).{0,30}$",
    },
    // i18n-allow:正则字面量：要匹配的正是中文标题词，翻译它会直接改变匹配行为。
    { id: "chapterN", labelKey: "dict.splitPreset.chapterN", pattern: "第\\s*[0-9〇零一二两三四五六七八九十百千]{1,12}\\s*[章节卷回部篇]" },
    { id: "chapterEn", labelKey: "dict.splitPreset.chapterEn", pattern: "^\\s*Chapter\\s+\\d+.*$" },
    // i18n-allow:正则字面量：含中文顿号 / 句点，翻译它会直接改变匹配行为。
    { id: "numbered", labelKey: "dict.splitPreset.numbered", pattern: "^\\s*\\d+[、.．]\\s*\\S.*$" },
    { id: "separator", labelKey: "dict.splitPreset.separator", pattern: "^\\s*[-=*_]{3,}\\s*$" },
  ];

  const FALLBACK_FONTS = [
    "Microsoft YaHei",
    "SimSun",
    "KaiTi",
    "SimHei",
    "DengXian",
    "Segoe UI",
    "Georgia",
    "Consolas",
    "仿宋", // i18n-allow:字体族名：必须与系统字体名逐字一致（否则 CSS font-family 匹配不上），禁止翻译。
    "幼圆", // i18n-allow:字体族名：必须与系统字体名逐字一致（否则 CSS font-family 匹配不上），禁止翻译。
    "隶书", // i18n-allow:字体族名：必须与系统字体名逐字一致（否则 CSS font-family 匹配不上），禁止翻译。
    "思源黑体", // i18n-allow:字体族名：必须与系统字体名逐字一致（否则 CSS font-family 匹配不上），禁止翻译。
    "思源宋体", // i18n-allow:字体族名：必须与系统字体名逐字一致（否则 CSS font-family 匹配不上），禁止翻译。
  ];

  let fontList: string[] = $state([]);
  let fontFilter = $state("");

  const fontOptions = $derived.by(() => {
    const q = fontFilter.trim().toLowerCase();
    const customNames = new Set(config.reader.customFonts.map((f) => f.family.toLowerCase()));
    const all = [...new Set([...FALLBACK_FONTS, ...fontList])].filter(
      (f) => !customNames.has(f.toLowerCase())
    );
    return q ? all.filter((f) => f.toLowerCase().includes(q)).slice(0, 60) : all.slice(0, 60);
  });

  const customFontOptions = $derived.by(() => {
    const q = fontFilter.trim().toLowerCase();
    const list = config.reader.customFonts;
    return q ? list.filter((f) => f.family.toLowerCase().includes(q)) : list;
  });

  const latinFontNames = $derived(
    [...new Set([...config.reader.customFonts.map((f) => f.family), ...FALLBACK_FONTS, ...fontList])]
  );

  async function deleteCustomFont(family: string, file: string) {
    const stem = file.replace(/\.(ttf|otf|ttc)$/i, "");
    try {
      await api.deleteCustomFont(stem);
      const i = config.reader.customFonts.findIndex((f) => f.family === family && f.file === file);
      if (i >= 0) config.reader.customFonts.splice(i, 1);
      let fellBack = false;
      if (config.reader.fontFamily === family) {
        config.reader.fontFamily = defaultReaderConfig().fontFamily;
        fellBack = true;
      }
      if (config.ai.style.fontFamily === family) config.ai.style.fontFamily = "";
      saveConfig();
      toast(fellBack ? t("settings.font.deletedFallbackToast", { name: family }) : t("settings.font.deletedToast", { name: family }));
    } catch (e) {
      toast(String(e), "error");
    }
  }

  async function resetReader() {
    const ok = await appConfirm(
      t("settings.reader.resetConfirm")
    );
    if (!ok) return;
    resetReaderConfig();
    toast(t("settings.reader.resetDone"));
  }

  let aiTesting = $state(false);
  let aiTestResult = $state("");
  let capPromptIdx = $state<number | null>(null);

  // 字体平滑：单个复选框（勾选=灰度抗锯齿；取消=系统 ClearType 子像素）。
  let grayAA = $state(true);

  $effect(() => {
    if (tab !== "reader") return;
    api
      .getSetting("render.webviewArgs")
      .then((v) => {
        // 兼容旧值：含 enable-lcd-text 视为关闭灰度；空/灰度/GDI/自定义旧值一律视为开启
        grayAA = !(v || "").includes("enable-lcd-text");
      })
      .catch(() => {});
  });

  async function saveWebviewArgs() {
    try {
      await api.setSetting("render.webviewArgs", grayAA ? "--disable-lcd-text" : "--enable-lcd-text");
      toast(t("settings.font.graySaved"));
    } catch (e) {
      toast(String(e), "error");
    }
  }

  const ap = $derived(
    config.ai.profiles[Math.min(config.ai.active, config.ai.profiles.length - 1)] ?? config.ai.profiles[0]
  );
  const apMeta = $derived(aiProviderMeta(ap?.provider ?? "openai"));

  /** AI 设置三个分类的标题（显示顺序由 config.ai.groupOrder 决定）。 */
  const AI_GROUP_TITLES: Record<string, MessageKey> = {
    node: "dict.aiGroup.node",
    window: "dict.aiGroup.window",
    prompt: "dict.aiGroup.prompt",
  };
  /** 分类开合状态；本组件每次打开重挂载，故每次默认全展开（折叠态不持久化）。 */
  let aiOpen = $state<Record<string, boolean>>({ node: true, window: true, prompt: true });
  /** 按 config.ai.groupOrder 排列后的分类列表（过滤非法 id）。 */
  const aiGroups = $derived(
    (config.ai.groupOrder ?? [...AI_GROUPS])
      .filter((id): id is AiGroup => (AI_GROUPS as readonly string[]).includes(id))
      .map((id) => ({ id, title: AI_GROUP_TITLES[id] })),
  );

  /** 分类拖拽排序：把 from 插入到 target 之前/之后，持久化到 config.ai.groupOrder（同 navDrop）。 */
  function aiGroupDrop(from: string, target: string, place: "before" | "after") {
    if (!from || from === target) return;
    const arr = [...config.ai.groupOrder];
    const fi = arr.indexOf(from);
    if (fi < 0) return;
    arr.splice(fi, 1);
    const ti = arr.indexOf(target);
    if (ti < 0) return;
    arr.splice(place === "after" ? ti + 1 : ti, 0, from);
    config.ai.groupOrder = arr;
    saveConfig();
  }

  function addProfile() {
    const provider: AiProvider = ap?.provider ?? "openai";
    const created = newAiProfile(config.ai.profiles, provider);
    config.ai.profiles.push(created);
    config.ai.active = config.ai.profiles.length - 1;
    saveConfigNow();
    toast(t("settings.ai.endpointCreated", { name: created.name }));
  }

  function delProfile() {
    if (config.ai.profiles.length <= 1) {
      toast(t("settings.ai.keepOneEndpoint"), "error");
      return;
    }
    config.ai.profiles.splice(config.ai.active, 1);
    config.ai.active = Math.max(0, Math.min(config.ai.active, config.ai.profiles.length - 1));
    saveConfigNow();
  }

  function copyReaderStyleToAi() {
    const r = config.reader;
    const s = config.ai.style;
    s.fontFamily = r.fontFamily;
    s.fontSize = r.fontSize;
    s.lineHeight = r.lineHeight;
    s.paraSpacing = r.paraSpacing;
    s.marginH = r.marginH;
    s.marginV = r.marginV;
    s.bgColor = r.bgColor;
    s.fgColor = r.fgColor;
    saveConfig();
    toast(t("settings.ai.copiedReaderStyle"));
  }

  function addPrompt() {
    config.ai.prompts.push({ id: Date.now(), name: `提示${config.ai.prompts.length + 1}`, text: "{chapter}", key: "" }); // i18n-allow:持久化值：新建提示词档的种子名，会原样落进 config.ai.prompts。
    saveConfig();
  }

  function delPrompt(i: number) {
    if (config.ai.prompts.length <= 1) {
      toast(t("settings.ai.keepOnePrompt"), "error");
      return;
    }
    config.ai.prompts.splice(i, 1);
    saveConfig();
  }

  $effect(() => {
    if (capPromptIdx === null) return;
    const idx = capPromptIdx;
    const h = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") { capPromptIdx = null; return; }
      if (["Control", "Alt", "Shift", "Meta", "CapsLock"].includes(e.key)) return;
      let k = e.key;
      if (k === " ") k = "Space";
      const p = config.ai.prompts[idx];
      if (p) p.key = k;
      capPromptIdx = null;
      saveConfig();
    };
    document.addEventListener("keydown", h, true);
    return () => document.removeEventListener("keydown", h, true);
  });

  async function testAi() {
    aiTesting = true;
    aiTestResult = t("settings.ai.testingHint");
    try {
      const p = ap;
      const r = await api.aiTest({
        provider: p.provider,
        baseUrl: p.baseUrl,
        apiKey: p.apiKey,
        model: p.model,
        deployment: p.deployment,
        apiVersion: p.apiVersion,
        proxyMode: p.proxyMode,
        proxyUrl: p.proxyUrl,
      });
      aiTestResult = r;
    } catch (e) {
      aiTestResult = `✗ ${e}`;
    } finally {
      aiTesting = false;
    }
  }

  $effect(() => {
    api.listSystemFonts().then((l) => {
      fontList = l.map((f) => f.family);
    }).catch(() => {});
  });

  // 打开设置时自动把已装字体的旧名修正为字体内部真实名（只需一次）
  $effect(() => {
    let key = "";
    try { key = localStorage.getItem("tidepage.font.mig") ?? ""; } catch { /* ignore */ }
    if (key !== "1") {
      try { localStorage.setItem("tidepage.font.mig", "1"); } catch { /* ignore */ }
      refreshFontNames(true);
    }
  });

  async function installFont() {
    const p = await api.pickFile(t("settings.font.pickDlg"), [
      { name: t("settings.fileDlg.font"), exts: ["ttf", "otf", "ttc"] },
    ]);
    if (!p) return;
    try {
      const family = await api.installFontFile(p);
      const file = p.split(/[\\/]/).pop()!.toLowerCase();
      if (!config.reader.customFonts.some((f) => f.family === family)) {
        config.reader.customFonts.push({ family, file });
      }
      config.reader.fontFamily = family;
      saveConfig();
      toast(t("settings.font.installedToast", { name: family }));
    } catch (e) {
      toast(String(e), "error");
    }
  }

  async function refreshFontNames(silent = false) {
    try {
      const list = await api.listCustomFonts();
      const old = config.reader.customFonts;
      const renamed: Record<string, string> = {};
      const updated = old.map((f) => {
        const hit = list.find((x) => x.file.toLowerCase() === f.file.toLowerCase());
        if (hit && hit.family !== f.family) renamed[f.family] = hit.family;
        return hit ? { family: hit.family, file: f.file } : f;
      });
      config.reader.customFonts = updated;
      if (renamed[config.reader.fontFamily]) config.reader.fontFamily = renamed[config.reader.fontFamily];
      saveConfig();
      if (!silent) toast(t("settings.font.namesFixed"));
    } catch (e) {
      if (!silent) toast(String(e), "error");
    }
  }

  // 「添加快捷键」芯片在未捕获态显示的图形符号。放脚本常量而非 markup 字面量有两个原因：
  //   1) markup 里的 `//` 不是注释，会被 Svelte 原样渲染成界面文字（历史缺陷）；
  //   2) 豁免说明必须和字面量同处脚本块，守门脚本的行内 `// i18n-allow:` 规则才认得到。
  const ADD_BIND_GLYPH = "＋"; // i18n-allow:图形符号：全角加号是「添加」芯片的图形，两种语言同形，不是可翻译文案。

  let capturing = $state<BindAction | null>(null);

  $effect(() => {
    const act = capturing;
    if (!act) return;
    const h = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        capturing = null;
        return;
      }
      if (["Control", "Alt", "Shift", "Meta", "CapsLock"].includes(e.key)) return;
      const combo = comboOf(e);
      for (const def of BIND_DEFS) {
        const arr = config.reader.bindings[def.id];
        if (arr && def.id !== act && arr.includes(combo)) {
          arr.splice(arr.indexOf(combo), 1);
          toast(t("settings.font.comboRemoved", { combo, name: def.name }));
        }
      }
      const target = config.reader.bindings[act];
      if (!target.includes(combo)) target.push(combo);
      saveConfig();
      capturing = null;
    };
    document.addEventListener("keydown", h, true);
    return () => document.removeEventListener("keydown", h, true);
  });

  function removeBind(a: BindAction, k: string) {
    const arr = config.reader.bindings[a];
    const i = arr.indexOf(k);
    if (i >= 0) arr.splice(i, 1);
    saveConfig();
  }

  function resetBindings() {
    config.reader.bindings = structuredClone(DEFAULT_BINDINGS);
    saveConfig();
    toast(t("settings.keys.resetDone"));
  }

  async function testWebdav() {
    webdavTesting = true;
    webdavStatus = t("settings.dav.testing");
    try {
      const ok = await api.webdavTest(config.webdav.base, config.webdav.user, config.webdav.pass);
      webdavStatus = ok ? t("settings.dav.testOk") : t("settings.dav.testFail");
      if (!ok) toast(t("settings.dav.testFailed"), "error");
    } catch (e) {
      webdavStatus = `✗ ${e}`;
    } finally {
      webdavTesting = false;
    }
  }

  async function pickWebdavDir() {
    const d = await api.pickFolder(t("settings.dav.pickDlg"));
    if (!d) return;
    config.webdav.downloadDir = d;
    saveConfig();
  }

  /* ---------- WebDAV 设置备份（手动 + 定期） ---------- */
  let webdavBackupBusy = $state(false);
  let webdavRestoreBusy = $state(false);
  let backupListBusy = $state(false);
  let backupList = $state<{ name: string; mtime: number }[]>([]);
  let selectedBackup = $state("");

  /** 「上次备份」本地时间文本。 */
  const lastBackupText = $derived.by(() => {
    const lastAt = config.webdav.backup?.lastAt ?? 0;
    if (!lastAt) return t("settings.dav.never");
    try {
      return new Date(lastAt).toLocaleString(dateLocale());
    } catch {
      return String(lastAt);
    }
  });

  /** 备份下拉项文本：设备名 · 本地时间（时间优先用文件 mtime，缺失时回退文件名内时间）。 */
  function backupLabel(it: { name: string; mtime: number }): string {
    const base = it.name.replace(/^config-/, "").replace(/\.json$/, "");
    const m = base.match(/^(.*)-(\d{8}-\d{6})$/);
    const device = m ? m[1] : base;
    const at = it.mtime ? new Date(it.mtime * 1000).toLocaleString() : m ? m[2] : "";
    return `${device} · ${at}`;
  }

  /** 读取云端备份列表（仅 TidePage/settings 下 config-*.json）。 */
  async function loadBackupList() {
    const wd = config.webdav;
    if (!wd.base.trim() || !wd.user.trim() || !wd.pass) {
      backupList = [];
      return;
    }
    backupListBusy = true;
    try {
      const items = await api.webdavList(wd.base, wd.user, wd.pass, "TidePage/settings");
      backupList = items
        .filter((it) => !it.is_dir && /^config-.*\.json$/.test(it.name))
        .map((it) => ({ name: it.name, mtime: it.mtime || 0 }))
        .sort((a, b) => b.mtime - a.mtime || b.name.localeCompare(a.name));
      if (!backupList.some((b) => b.name === selectedBackup)) {
        selectedBackup = backupList[0]?.name ?? "";
      }
    } catch (e) {
      backupList = [];
      toast(t("settings.dav.listFailed", { err: String(e) }), "error");
    } finally {
      backupListBusy = false;
    }
  }

  $effect(() => {
    if (tab !== "webdav") return;
    untrack(() => {
      void loadBackupList();
    });
  });

  async function backupWebdavSettingsNow() {
    webdavBackupBusy = true;
    try {
      await runSettingsBackup();
      await loadBackupList();
      toast(t("settings.dav.backupDone"));
    } catch (e) {
      toast(t("settings.dav.backupFailed", { err: String(e) }), "error");
    } finally {
      webdavBackupBusy = false;
    }
  }

  async function restoreWebdavSettings() {
    const wd = config.webdav;
    if (!wd.base.trim() || !wd.user.trim() || !wd.pass) {
      toast(t("settings.dav.needCreds"), "error");
      return;
    }
    if (!selectedBackup) {
      toast(t("settings.dav.pickFirst"), "error");
      return;
    }
    const ok = await appConfirm(t("settings.dav.restoreConfirm"));
    if (!ok) return;
    webdavRestoreBusy = true;
    try {
      const data = await api.webdavRestoreSettings(wd.base, wd.user, wd.pass, selectedBackup);
      if (!data) {
        toast(t("settings.dav.notExist"), "error");
        return;
      }
      const parsed = JSON.parse(data);
      if (parsed && parsed.v === 2 && parsed.enc) {
        // 跨设备可解：用当前 WebDAV 账号派生口令解密凭据。
        const passphrase = `${wd.user}\u0000${wd.pass}`;
        let credJson: string;
        try {
          credJson = await api.backupOpen(parsed.enc, passphrase);
        } catch {
          toast(t("settings.dav.decryptFailed"), "error");
          return;
        }
        applyConfigJson(JSON.stringify(parsed.config));
        await api.secretWrite(credJson);
        applySecrets(config, JSON.parse(credJson) as Record<string, unknown>);
      } else {
        // v1 旧备份：非密文部分 + 本机 DPAPI 密文，尽力兼容。
        const obj =
          parsed && typeof parsed === "object" && (parsed as any).config
            ? (parsed as any).config
            : parsed;
        if (!obj || typeof obj !== "object" || !obj.reader) throw new Error(t("settings.dav.invalidFormat"));
        applyConfigJson(JSON.stringify(obj));
        const secret = parsed && typeof parsed === "object" ? (parsed as any).secret : undefined;
        if (typeof secret === "string" && secret) {
          try {
            await api.setSetting("secret.config", secret);
            const raw = await api.secretRead();
            if (raw) applySecrets(config, JSON.parse(raw) as Record<string, unknown>);
          } catch {
            /* 旧备份密钥与当前机器不匹配时跳过 */
          }
        }
      }
      saveConfigNow();
      toast(t("settings.dav.restored"));
    } catch (e) {
      toast(t("settings.dav.restoreFailed", { err: String(e) }), "error");
    } finally {
      webdavRestoreBusy = false;
    }
  }

  async function saveRegex() {
    try {
      await api.saveSplitRules(regexText.trim());
      config.regex = regexText.trim();
      saveConfig();
      toast(t("settings.split.saved"));
    } catch (e) {
      toast(String(e), "error");
    }
  }

  let regexBookPath = $state("");
  let previewToc: TocEntry[] | null = $state(null);
  let previewing = $state(false);

  async function pickBgImage() {
    const p = await api.pickFile(t("settings.bgImage.pickDlg"), [
      { name: t("settings.fileDlg.image"), exts: ["png", "jpg", "jpeg", "webp", "bmp", "gif"] },
    ]);
    if (!p) return;
    try {
      const name = await api.setBgImage(p);
      config.reader.bgImage = name;
      saveConfig();
    } catch (e) {
      toast(String(e), "error");
    }
  }

  async function removeBgImage() {
    try {
      await api.clearBgImage();
      config.reader.bgImage = "";
      saveConfig();
    } catch { /* ignore */ }
  }

  async function pickRegexBook() {
    const p = await api.pickFile(t("settings.split.pickDlg"), [{ name: t("settings.fileDlg.txt"), exts: ["txt"] }]);
    if (p) {
      regexBookPath = p;
      await previewRegex();
    }
  }

  async function previewRegex() {
    if (!regexBookPath) {
      await pickRegexBook();
      if (!regexBookPath) return;
    }
    previewing = true;
    try {
      previewToc = await api.previewSplit(regexBookPath, regexText.trim());
    } catch (e) {
      previewToc = null;
      toast(String(e), "error");
    } finally {
      previewing = false;
    }
  }
</script>

<div class="settings-root">
  <!-- 顶栏：标题 + 面包屑 + 关闭 -->
  <header class="set-head">
    <div class="set-title">
      <span class="set-title-main">{t("settings.title")}</span>
      <span class="set-crumb">›</span>
      <span class="set-crumb-now">{tabLabel}</span>
    </div>
    <button class="icon set-close" title={t("settings.close")} onclick={() => (ui.settingsOpen = false)}>×</button>
  </header>

  <div class="set-main">
    <!-- 左侧导航：扁平分区列表（无分组）+ 搜索；分区可拖拽排序（复用 dragSort，特效同顶栏图标）。 -->
    <aside class="set-nav">
      <div class="set-search">
        <input placeholder={t("settings.nav.searchPlaceholder")} bind:value={navQuery} aria-label={t("settings.nav.searchAria")} onkeydown={(e) => { if (e.key === "Escape") navQuery = ""; }} />
        {#if navQuery}
          <button class="set-search-clear" title={t("settings.nav.clear")} aria-label={t("settings.nav.clearSearchAria")} onclick={() => (navQuery = "")}>×</button>
        {/if}
      </div>
      <div class="set-nav-list" use:dragSort={{ onDrop: navDrop, axis: "y" }}>
        {#each navFiltered as it (it.id)}
          {@const Icon = it.icon}
          <button
            class="set-nav-item"
            data-drag-id={it.id}
            data-testid={"set-tab-" + it.id}
            class:active={tab === it.id}
            onclick={() => { tab = it.id; if (it.id === "tts") refreshVoices(); }}
          >
            <span class="set-nav-glyph"><Icon size={16} stroke={1.75} /></span>
            <span class="set-nav-label">{t(it.labelKey)}</span>
          </button>
        {/each}
      </div>
      {#if navQuery.trim()}
        <div class="set-nav-group">{t("settings.nav.group")}</div>
        {#if searchResults.length}
          <div class="set-nav-results">
            {#each searchResults as h, i (h.tab + "|" + h.text + "|" + i)}
              <button class="set-result" onclick={() => locateSetting(h)} title={h.text}>
                <span class="set-result-label">{h.text}</span>
                <span class="set-result-tab">{t(TAB_META[h.tab].labelKey)}</span>
              </button>
            {/each}
          </div>
        {:else if !navFiltered.length}
          <div class="set-nav-empty">{t("settings.nav.noMatch")}</div>
        {/if}
      {/if}
    </aside>

    <!-- 右侧内容：全部 pane 常驻渲染、按需 hidden，使搜索可索引并定位到任意设置项。 -->
    <section class="set-content">
  <div class="set-pane" data-pane="appearance" hidden={tab !== "appearance"}>

    <div class="field">
      <label>{t("settings.uiTheme")}</label>
      <div class="fr-seg">
        <button data-testid="ui-theme-light" class:active={config.uiTheme === "light"} onclick={() => setUiTheme("light")}>{t("settings.uiTheme.light")}</button>
        <button data-testid="ui-theme-dark" class:active={config.uiTheme === "dark"} onclick={() => setUiTheme("dark")}>{t("settings.uiTheme.dark")}</button>
        <button data-testid="ui-theme-system" class:active={config.uiTheme === "system"} onclick={() => setUiTheme("system")}>{t("settings.uiTheme.system")}</button>
      </div>
    </div>

    <div class="field">
      <label>{t("settings.skin.light")}</label>
      <div class="skin-grid">
        {#each UI_SKINS.filter((s) => s.mode === "light") as skin (skin.id)}
          <button
            class="skin-card" data-skin-id={skin.id}
            class:active={config.uiSkinLight === skin.id}
            onclick={() => setUiSkin(skin.id)}
            title={labelOfId("skin", skin.id)}
          >
            <span class="sk-preview" style="background:{skin.bg}; border-color:{skin.line};">
              <span class="sk-panel" style="background:{skin.panel};"></span>
              <span class="sk-accent" style="background:{skin.accent};"></span>
              <span class="sk-bar" style="background:{skin.fg};"></span>
              <span class="sk-bar short" style="background:{skin.fg2};"></span>
            </span>
            <span class="sk-name">{labelOfId("skin", skin.id)}</span>
            {#if config.uiSkinLight === skin.id}<span class="sk-check">✓</span>{/if}
          </button>
        {/each}
      </div>
    </div>

    <div class="field">
      <label>{t("settings.skin.dark")}</label>
      <div class="skin-grid">
        {#each UI_SKINS.filter((s) => s.mode === "dark") as skin (skin.id)}
          <button
            class="skin-card" data-skin-id={skin.id}
            class:active={config.uiSkinDark === skin.id}
            onclick={() => setUiSkin(skin.id)}
            title={labelOfId("skin", skin.id)}
          >
            <span class="sk-preview" style="background:{skin.bg}; border-color:{skin.line};">
              <span class="sk-panel" style="background:{skin.panel};"></span>
              <span class="sk-accent" style="background:{skin.accent};"></span>
              <span class="sk-bar" style="background:{skin.fg};"></span>
              <span class="sk-bar short" style="background:{skin.fg2};"></span>
            </span>
            <span class="sk-name">{labelOfId("skin", skin.id)}</span>
            {#if config.uiSkinDark === skin.id}<span class="sk-check">✓</span>{/if}
          </button>
        {/each}
      </div>
    </div>

    <div class="field">
      <label>{t("settings.cover.base")}</label>
      <div class="cover-ctl">
        <img class="cover-preview" src={coverPreviewUrl} alt={t("settings.cover.previewAlt")} />
        <div class="cover-ctl-main">
          <p class="tip">
            {t("settings.cover.hint")}
            {hasDefaultCover ? t("settings.cover.custom") : t("settings.cover.builtin")}
          </p>
          <div class="cover-ctl-acts">
            <button onclick={pickDefaultCover} disabled={coverBusy}>{t("settings.cover.pick")}</button>
            <button onclick={resetDefaultCover} disabled={coverBusy || !hasDefaultCover}>{t("settings.cover.restore")}</button>
          </div>
        </div>
      </div>
      <p class="tip">{t("settings.cover.tip")}</p>
    </div>

  </div>
  <div class="set-pane" data-pane="reader" hidden={tab !== "reader"}>
    <div class="field">
      <label>{t("settings.reader.pageMode")}</label>
      <select bind:value={config.reader.mode} onchange={saveConfig}>
        <option value="page">{t("settings.reader.pageModePage")}</option>
        <option value="scroll">{t("settings.reader.pageModeScroll")}</option>
      </select>
      <p class="tip">{t("settings.reader.pageModeTip")}</p>
    </div>

    {#if config.reader.mode === "scroll"}
      <div class="field">
        <label>{t("settings.reader.chapterTransition")}</label>
        <select bind:value={config.reader.scrollBounce} onchange={saveConfig}>
          <option value={false}>{t("settings.reader.transSeamless")}</option>
          <option value={true}>{t("settings.reader.transBuffered")}</option>
        </select>
        <p class="tip">{t("settings.reader.transTip")}</p>
      </div>
      {#if config.reader.scrollBounce}
        <div class="field">
          <label>{t("settings.reader.bufferStrength")}</label>
          <select bind:value={config.reader.scrollBounceLevel} onchange={saveConfig}>
            {#each Object.entries(SCROLL_BOUNCE_PRESETS) as [k] (k)}
              <option value={k}>{labelOfId("bounce", k)}</option>
            {/each}
          </select>
          <p class="tip">{t("settings.reader.bufferTip")}</p>
        </div>
      {/if}
    {:else}
      <div class="grid2">
        <div class="field">
          <label>{t("settings.reader.columns")}</label>
          <select bind:value={config.reader.duo} onchange={saveConfig} title={t("settings.reader.columnsTitle")}>
            <option value={false}>{t("settings.reader.columnSingle")}</option>
            <option value={true}>{t("settings.reader.columnDual")}</option>
          </select>
        </div>
        <div class="field">
          <label>{t("settings.reader.colGap", { n: config.reader.colGap })}</label>
          <input type="range" min="0" max="160" step="4" bind:value={config.reader.colGap} oninput={saveConfig} disabled={!config.reader.duo} />
        </div>
      </div>
      <p class="tip">{t("settings.reader.columnsTip")}</p>
    {/if}

    <div class="grid3">
      <div class="field">
        <label>{t("settings.reader.statusPos")}</label>
        <select bind:value={config.reader.statusPos} onchange={saveConfig}>
          <option value="left">{t("settings.reader.posLeft")}</option>
          <option value="center">{t("settings.reader.posCenter")}</option>
          <option value="right">{t("settings.reader.posRight")}</option>
          <option value="none">{t("settings.reader.posNone")}</option>
        </select>
      </div>
      <div class="field">
        <label>{t("settings.reader.statusColor")}</label>
        <div class="color-row">
          <input placeholder={t("settings.reader.colorAuto")} title={t("settings.reader.statusColorTip")} bind:value={config.reader.statusColor} oninput={saveConfig} />
          <ColorPicker bind:value={config.reader.statusColor} allowEmpty emptyLabel={t("settings.reader.followTheme")} title={t("settings.reader.statusColor")} onchange={saveConfig} />
        </div>
      </div>
      <div class="field">
        <label>{t("settings.reader.topbarColor")}</label>
        <div class="color-row">
          <input placeholder={t("settings.reader.colorAuto")} title={t("settings.reader.topbarColorTip")} bind:value={config.reader.topbarColor} oninput={saveConfig} />
          <ColorPicker bind:value={config.reader.topbarColor} allowEmpty emptyLabel={t("settings.reader.followTheme")} title={t("settings.reader.topbarColor")} onchange={saveConfig} />
        </div>
      </div>
    </div>

    <details class="adv">
      <summary>{t("settings.statusItems.title")}</summary>
      <div class="field">
      <div class="stat-grid">
        <div class="stat-head">{t("settings.statusItems.item")}</div>
        <div class="stat-head">{t("settings.statusItems.top")}</div>
        <div class="stat-head">{t("settings.statusItems.footerLeft")}</div>
        <div class="stat-head">{t("settings.statusItems.footerRight")}</div>
        {#each STATUS_ITEM_DEFS as d (d.id)}
          <div class="stat-name">{labelOfId("statusItem", d.id)}</div>
          <div class="stat-cell">
            <input type="checkbox" checked={topEnabled(d.id)} onchange={() => toggleTopItem(d.id)} />
          </div>
          <div class="stat-cell">
            <input type="checkbox" checked={footerHas("left", d.id)} onchange={() => toggleFooter("left", d.id)} />
            {#if footerHas("left", d.id)}
              <button type="button" class="stat-mv" title={t("settings.moveLeft")} onclick={() => moveFooter("left", d.id, -1)}>‹</button>
              <button type="button" class="stat-mv" title={t("settings.moveRight")} onclick={() => moveFooter("left", d.id, 1)}>›</button>
            {/if}
          </div>
          <div class="stat-cell">
            <input type="checkbox" checked={footerHas("right", d.id)} onchange={() => toggleFooter("right", d.id)} />
            {#if footerHas("right", d.id)}
              <button type="button" class="stat-mv" title={t("settings.moveLeft")} onclick={() => moveFooter("right", d.id, -1)}>‹</button>
              <button type="button" class="stat-mv" title={t("settings.moveRight")} onclick={() => moveFooter("right", d.id, 1)}>›</button>
            {/if}
          </div>
        {/each}
      </div>
      <p class="tip">{t("settings.statusItems.tip")}</p>
      </div>
    </details>

    <div class="field">
      <div class="scheme-head">
        <label>{t("settings.scheme.title")}</label>
        <div class="scheme-actions">
          <button onclick={() => openSchemeSave()}>{t("settings.scheme.saveCurrent")}</button>
        </div>
      </div>
      <div class="scheme-grid">
        {#each config.readerSchemes as s (s.id)}
          <div class="scheme-card" class:active={schemeActive(s)} data-scheme-id={s.id}>
            <button class="scheme-preview" data-testid={"scheme-apply-" + s.id} title={t("settings.scheme.applyTip")} style="background:{s.cfg.bgColor || THEMES[s.cfg.theme].bg}; color:{s.cfg.fgColor || THEMES[s.cfg.theme].fg};" onclick={() => applyScheme(s.id, "book")}>
              <span class="sp-line" style="background:currentColor; opacity:.85;"></span>
              <span class="sp-line short" style="background:currentColor; opacity:.55;"></span>
              <span class="sp-line" style="background:currentColor; opacity:.75;"></span>
              <span class="sp-chip" style="background:var(--accent);"></span>
            </button>
            <div class="scheme-name" title={schemeLabel(s)}>{schemeLabel(s)}</div>
            <div class="scheme-menu">
              <button class="icon" data-testid="scheme-global" title={t("settings.scheme.globalTitle")} onclick={() => applyScheme(s.id, "global")}>{t("settings.scheme.global")}</button>
              <button class="icon" data-testid="scheme-book" title={t("settings.scheme.bookTitle")} onclick={() => applyScheme(s.id, "book")}>{t("settings.scheme.book")}</button>
              <button class="icon" data-testid="scheme-rename" title={t("settings.scheme.rename")} onclick={() => renameScheme(s)}>{t("settings.scheme.renameShort")}</button>
              <button class="icon" data-testid="scheme-delete" title={t("settings.scheme.delete")} onclick={() => delScheme(s)}>×</button>
            </div>
          </div>
        {/each}
      </div>
      {#if bookSchemeNote()}
        <div class="book-scheme-note">
          <span>{t("settings.scheme.bookApplied")}</span>
          <button class="link-btn" data-testid="scheme-clear-book" onclick={clearBookScheme}>{t("settings.scheme.clearBook")}</button>
        </div>
      {/if}
    </div>

    {#if schemeSaveOpen}
      <div class="scheme-save-backdrop" role="presentation" onclick={() => (schemeSaveOpen = false)}>
        <div class="scheme-save" role="dialog" aria-label={t("settings.scheme.saveAria")} onclick={(e) => e.stopPropagation()}>
          <div class="ss-head">{t("settings.scheme.saveHead")}</div>
          <p class="tip">{t("settings.scheme.saveTip")}</p>
          <div class="scheme-grid compact">
            {#each config.readerSchemes as s (s.id)}
              <button class="scheme-card save-target" onclick={() => saveOverScheme(s)} title={t("settings.scheme.overwriteTitle", { name: schemeLabel(s) })}>
                <span class="scheme-preview" style="background:{s.cfg.bgColor || THEMES[s.cfg.theme].bg}; color:{s.cfg.fgColor || THEMES[s.cfg.theme].fg};">
                  <span class="sp-line" style="background:currentColor; opacity:.85;"></span>
                  <span class="sp-line short" style="background:currentColor; opacity:.55;"></span>
                  <span class="sp-chip" style="background:var(--accent);"></span>
                </span>
                <span class="scheme-name">{schemeLabel(s)}</span>
              </button>
            {/each}
          </div>
          <div class="ss-new">
            <input placeholder={t("settings.scheme.newPlaceholder")} bind:value={schemeNameInput} />
            <button class="primary" onclick={openSchemeNew}>{t("settings.scheme.newBtn")}</button>
          </div>
        </div>
      </div>
    {/if}

    <div class="grid2">
      <div class="field">
        <label>{t("settings.color.bg")}</label>
        <div class="color-row">
          <input
            placeholder={t("settings.color.bgPlaceholder")}
            bind:value={config.reader.bgColor}
            oninput={saveConfig}
          />
          <ColorPicker bind:value={config.reader.bgColor} allowEmpty emptyLabel={t("settings.color.followPreset")} title={t("settings.color.bg")} onchange={saveConfig} />
        </div>
      </div>
      <div class="field">
        <label>{t("settings.color.fg")}</label>
        <div class="color-row">
          <input
            placeholder={t("settings.color.fgPlaceholder")}
            bind:value={config.reader.fgColor}
            oninput={saveConfig}
          />
          <ColorPicker bind:value={config.reader.fgColor} allowEmpty emptyLabel={t("settings.color.followPreset")} title={t("settings.color.fg")} onchange={saveConfig} />
        </div>
      </div>
    </div>
    <p class="tip">{t("settings.color.customTip")}</p>

    <div class="field">
      <label>{t("settings.bgImage.title")}</label>
      <div class="row-inline">
        {#if config.reader.bgImage}
          <img class="bg-thumb" src={"http://tidepage.localhost/bg/" + config.reader.bgImage} alt="" />
          <button onclick={pickBgImage}>{t("settings.bgImage.change")}</button>
          <button onclick={removeBgImage}>{t("settings.bgImage.remove")}</button>
        {:else}
          <button onclick={pickBgImage}>{t("settings.bgImage.pick")}</button>
        {/if}
      </div>
      <p class="tip">{t("settings.bgImage.tip")}</p>
    </div>

    {#if config.reader.bgImage}
      <div class="field">
        <label>{t("settings.bgOpacity", { n: Math.round(config.reader.bgOpacity * 100) })}</label>
        <input type="range" min="0.05" max="1" step="0.05" bind:value={config.reader.bgOpacity} oninput={saveConfig} />
        <p class="tip">{t("settings.bgOpacityTip")}</p>
      </div>
    {/if}

    <div class="field">
      <label>{t("settings.font.title")}</label>
      <div class="row-inline">
        <input
          class="font-input"
          placeholder={t("settings.font.searchPlaceholder")}
          bind:value={fontFilter}
        />
        <input
          class="font-value"
          placeholder={t("settings.font.currentPlaceholder")}
          bind:value={config.reader.fontFamily}
          onchange={saveConfig}
        />
        <button onclick={installFont}>{t("settings.font.install")}</button>
        <button onclick={() => refreshFontNames()} title={t("settings.font.reloadNamesTip")}>{t("settings.font.reloadNames")}</button>
      </div>
      <div class="font-list">
        {#if customFontOptions.length}
          <div class="font-group-title">{t("settings.font.installedGroup")}</div>
          {#each customFontOptions as f, fi (f.family + fi)}
            <span class="font-opt-wrap">
              <button
                class="font-opt"
                class:active={config.reader.fontFamily === f.family}
                style="font-family:'{f.family}'"
                onclick={() => { config.reader.fontFamily = f.family; saveConfig(); }}
              >★ {f.family}</button>
              <button
                class="font-del"
                title={t("settings.font.deleteTitle")}
                onclick={(e) => { e.stopPropagation(); deleteCustomFont(f.family, f.file); }}
              >×</button>
            </span>
          {/each}
        {/if}
        <div class="font-group-title">{t("settings.font.systemGroup")}</div>
        {#each fontOptions as f (f)}
          <button
            class="font-opt"
            class:active={config.reader.fontFamily === f}
            style="font-family:'{f}'"
            onclick={() => { config.reader.fontFamily = f; saveConfig(); }}
          >{f}</button>
        {/each}
      </div>
    </div>

    <div class="field">
      <label>{t("settings.font.latinTitle")}</label>
      <div class="row-inline">
        <input
          class="font-value"
          list="latin-font-options"
          placeholder={t("settings.font.latinPlaceholder")}
          bind:value={config.reader.latinFont}
          onchange={saveConfig}
        />
        {#if config.reader.latinFont}
          <button class="icon" title={t("settings.font.clear")} onclick={() => { config.reader.latinFont = ""; saveConfig(); }}>×</button>
        {/if}
      </div>
      <datalist id="latin-font-options">
        {#each latinFontNames as f (f)}
          <option value={f}></option>
        {/each}
      </datalist>
      <p class="tip">{t("settings.font.latinTip")}</p>
    </div>

    <div class="grid3">
      <div class="field">
        <label>{t("settings.font.size", { n: config.reader.fontSize })}</label>
        <input type="range" min="10" max="64" bind:value={config.reader.fontSize} oninput={saveConfig} />
      </div>
      <div class="field">
        <label>{t("settings.font.lineHeight", { n: config.reader.lineHeight })}</label>
        <input type="range" min="1.2" max="3" step="0.05" bind:value={config.reader.lineHeight} oninput={saveConfig} />
      </div>
      <div class="field">
        <label>{t("settings.font.paraSpacing", { n: config.reader.paraSpacing })}</label>
        <input type="range" min="0" max="2" step="0.1" bind:value={config.reader.paraSpacing} oninput={saveConfig} />
      </div>
    </div>

    <hr />
    <div class="field">
      <label class="check">
        <input type="checkbox" bind:checked={grayAA} onchange={saveWebviewArgs} />
        {t("settings.font.grayAA")}
      </label>
      <p class="tip">{t("settings.font.grayAATip")}</p>
    </div>

    <div class="grid3">
      <div class="field">
        <label>{t("settings.font.weight")}</label>
        <select bind:value={config.reader.fontWeight} onchange={saveConfig}>
          <option value={300}>{t("settings.font.weight300")}</option>
          <option value={400}>{t("settings.font.weight400")}</option>
          <option value={500}>{t("settings.font.weight500")}</option>
        </select>
      </div>
      <div class="field">
        <label>{t("settings.font.align")}</label>
        <select bind:value={config.reader.textAlign} onchange={saveConfig}>
          <option value="justify">{t("settings.font.alignJustify")}</option>
          <option value="left">{t("settings.font.alignLeft")}</option>
        </select>
      </div>
      <div class="field">
        <label>{config.reader.maxLineChars > 0 ? t("settings.font.maxLine", { n: config.reader.maxLineChars }) : t("settings.font.maxLineOff")}</label>
        <input type="range" min="0" max="60" step="2" bind:value={config.reader.maxLineChars} oninput={saveConfig} />
      </div>
    </div>
    <p class="tip">{t("settings.font.maxLineTip")}</p>
    <hr />

    <div class="grid3">
      <div class="field">
        <label>{config.reader.maxLineChars > 0 ? t("settings.font.marginHTaken") : t("settings.font.marginH", { n: config.reader.marginH + "px" })}</label>
        <div class="row-inline">
          <input type="range" min="0" max="560" bind:value={config.reader.marginH} oninput={saveConfig} disabled={config.reader.maxLineChars > 0} />
          <input class="numbox" type="number" min="0" max="560" bind:value={config.reader.marginH} onchange={saveConfig} disabled={config.reader.maxLineChars > 0} />
        </div>
      </div>
      <div class="field">
        <label>{t("settings.font.marginV", { n: config.reader.marginV })}</label>
        <div class="row-inline">
          <input type="range" min="0" max="400" bind:value={config.reader.marginV} oninput={saveConfig} />
          <input class="numbox" type="number" min="0" max="400" bind:value={config.reader.marginV} onchange={saveConfig} />
        </div>
      </div>
      <div class="field">
        <label>{t("settings.font.indent")}</label>
        <select bind:value={config.reader.indent} onchange={saveConfig}>
          <option value="0em">{t("settings.font.indent0")}</option>
          <option value="1em">{t("settings.font.indent1")}</option>
          <option value="2em">{t("settings.font.indent2")}</option>
          <option value="3em">{t("settings.font.indent3")}</option>
        </select>
      </div>
    </div>

    <div class="checks">
      <label><input type="checkbox" bind:checked={config.reader.overrideBookCss} onchange={saveConfig}  /> {t("settings.reader.overrideCss")}</label>
      <label><input type="checkbox" bind:checked={config.reader.pageTurnKeys} onchange={saveConfig}  /> {t("settings.reader.pageTurnKeys")}</label>
    </div>
    <p class="tip">{t("settings.reader.overrideCssTip")}</p>

    <hr />
    <div class="grid3">
      <div class="field">
        <label>{t("settings.hl.defaultStyle")}</label>
        <select bind:value={config.reader.highlightStyle} onchange={saveConfig} title={t("settings.hl.defaultStyleTip")}>
          {#each HIGHLIGHT_STYLES as s (s.id)}
            <option value={s.id}>{labelOfId("hlStyle", s.id)}</option>
          {/each}
        </select>
      </div>
      <div class="field">
        <label>{t("settings.hl.color")}</label>
        <div class="color-row">
          <input placeholder={t("settings.hl.colorPlaceholder")} bind:value={config.reader.highlightColor} oninput={saveConfig} />
          <ColorPicker bind:value={config.reader.highlightColor} allowEmpty emptyLabel={t("settings.hl.colorEmpty")} title={t("settings.hl.color")} onchange={saveConfig} />
        </div>
      </div>
      <div class="field">
        <label>{t("settings.reader.browserSearch")}</label>
        <input placeholder="https://www.baidu.com/s?wd=%s" title={t("settings.reader.browserSearchTip")} bind:value={config.reader.browserSearchUrl} oninput={saveConfig} />
      </div>
    </div>
    <div class="field">
      <label>{t("settings.reader.customCss")}</label>
      <textarea class="cssbox" rows="6" bind:value={config.reader.cssExtra} oninput={saveConfig} placeholder={"p { letter-spacing: 0.05em; }\nh1 { color: #a00; }"}></textarea>
      <p class="tip">{t("settings.reader.customCssTip")}</p>
    </div>

    <div class="close-row" style="margin-top:12px">
      <button onclick={resetReader}>{t("settings.reader.reset")}</button>
    </div>
  </div>
  <div class="set-pane" data-pane="manga" hidden={tab !== "manga"}>
    <p class="tip">{t("settings.manga.scopeTip")}</p>

    <div class="grid3 mg-grid">
      <div class="field">
        <label>{t("settings.manga.display")}</label>
        <select bind:value={config.reader.manga.layout} onchange={saveConfig}>
          <option value="flow">{t("settings.manga.displayFlow")}</option>
          <option value="horizontal">{t("settings.manga.displayHorizontal")}</option>
        </select>
        <p class="tip">{t("settings.manga.displayTip")}</p>
      </div>
      <div class="field">
        <label>{t("settings.manga.direction", { note: config.reader.manga.layout === "flow" ? t("settings.manga.horizontalOnly") : "" })}</label>
        <select
          bind:value={config.reader.manga.direction}
          onchange={saveConfig}
          disabled={config.reader.manga.layout === "flow"}
        >
          <option value="ltr">{t("settings.manga.dirLtr")}</option>
          <option value="rtl">{t("settings.manga.dirRtl")}</option>
        </select>
      </div>
      <div class="field">
        <label>{t("settings.manga.textMode")}</label>
        <select bind:value={config.reader.manga.hideText} onchange={saveConfig}>
          <option value={false}>{t("settings.manga.textShow")}</option>
          <option value={true}>{t("settings.manga.textImageOnly")}</option>
        </select>
      </div>
    </div>

    <hr />
    <div class="field">
      <label>{t("settings.manga.imgScale", { n: config.reader.manga.imgScale.toFixed(1) })}</label>
      <input type="range" min="0.3" max="3" step="0.1" bind:value={config.reader.manga.imgScale} oninput={onMangaScaleInput} />
      <p class="tip">{t("settings.manga.imgScaleTip")}</p>
    </div>

    <div class="checks">
      <label><input type="checkbox" checked={config.reader.manga.fitWidth} onchange={(e) => setMangaFit("fitWidth", e.currentTarget.checked)}  /> {t("settings.manga.fitWidth")}</label>
      <label><input type="checkbox" checked={config.reader.manga.fitHeight} onchange={(e) => setMangaFit("fitHeight", e.currentTarget.checked)}  /> {t("settings.manga.fitHeight")}</label>
    </div>

    <hr />
    <div class="grid3 mg-grid">
      <div class="field" class:mg-off={config.reader.manga.layout === "horizontal"}>
        <label>{config.reader.manga.layout === "horizontal" ? t("settings.manga.marginH", { n: t("settings.manga.horizontalUnsupported") }) : t("settings.manga.marginH", { n: config.reader.manga.marginH + "px" })}</label>
        <div class="row-inline">
          <input type="range" min="0" max="400" bind:value={config.reader.manga.marginH} oninput={saveConfig} disabled={config.reader.manga.layout === "horizontal"} />
          <input class="numbox" type="number" min="0" max="400" bind:value={config.reader.manga.marginH} onchange={saveConfig} disabled={config.reader.manga.layout === "horizontal"} />
        </div>
      </div>
      <div class="field" class:mg-off={config.reader.manga.layout === "flow"}>
        <label>{config.reader.manga.layout === "flow" ? t("settings.manga.marginV", { n: t("settings.manga.flowUnsupported") }) : t("settings.manga.marginV", { n: config.reader.manga.marginV + "px" })}</label>
        <div class="row-inline">
          <input type="range" min="0" max="400" bind:value={config.reader.manga.marginV} oninput={saveConfig} disabled={config.reader.manga.layout === "flow"} />
          <input class="numbox" type="number" min="0" max="400" bind:value={config.reader.manga.marginV} onchange={saveConfig} disabled={config.reader.manga.layout === "flow"} />
        </div>
      </div>
      <div class="field">
        <label>{t("settings.manga.imgGap", { n: config.reader.manga.imgGap })}</label>
        <div class="row-inline">
          <input type="range" min="0" max="80" bind:value={config.reader.manga.imgGap} oninput={saveConfig} />
          <input class="numbox" type="number" min="0" max="80" bind:value={config.reader.manga.imgGap} onchange={saveConfig} />
        </div>
        <p class="tip">{t("settings.manga.imgGapTip")}</p>
      </div>
    </div>

    <hr />
    <div class="field">
      <label>{t("settings.manga.bgColor")}</label>
      <div class="color-row">
        <input placeholder={t("settings.manga.bgColorPlaceholder")} bind:value={config.reader.manga.bgColor} oninput={saveConfig} />
        <ColorPicker bind:value={config.reader.manga.bgColor} allowEmpty emptyLabel={t("settings.reader.followTheme")} title={t("settings.manga.bgColor")} onchange={saveConfig} />
      </div>
      <p class="tip">{t("settings.manga.bgColorTip")}</p>
    </div>
  </div>
  <div class="set-pane" data-pane="split" hidden={tab !== "split"}>
    <p class="tip">
      {t("settings.split.desc")}
    </p>

    <div class="field">
      <label>{t("settings.split.presets")}</label>
      <div class="presets">
        {#each SPLIT_PRESETS as p (p.id)}
          <button onclick={() => (regexText = p.pattern)}>{t(p.labelKey)}</button>
        {/each}
      </div>
    </div>

    <div class="field">
      <label>{t("settings.split.regexLabel")}</label>
      <textarea rows="4" bind:value={regexText} spellcheck="false"></textarea>
    </div>

    <div class="row-inline" style="margin-bottom:12px">
      <button onclick={pickRegexBook}>{regexBookPath ? t("settings.split.pickBookOther") : t("settings.split.pickBook")}</button>
      <button onclick={previewRegex} disabled={previewing}>{previewing ? t("settings.split.previewing") : t("settings.split.preview")}</button>
      {#if previewToc !== null}
        <span class="status" class:ok={previewToc.length > 1}>{t("settings.split.previewCount", { n: previewToc.length })}</span>
      {/if}
    </div>

    {#if previewToc !== null && previewToc.length}
      <div class="toc-preview">
        {#each previewToc.slice(0, 100) as t (t.index)}
          <div class="tp-row">{t.index + 1}. {t.title}</div>
        {/each}
        {#if previewToc.length > 100}
          <div class="tp-row muted">{t("settings.split.totalMore", { n: previewToc.length })}</div>
        {/if}
      </div>
    {/if}

    <div class="close-row">
      <button class="primary" onclick={saveRegex}>{t("settings.split.save")}</button>
    </div>
  </div>
  <div class="set-pane" data-pane="keys" hidden={tab !== "keys"}>
    <p class="tip">
      {t("settings.keys.desc")}
    </p>
    <div class="checks" style="margin:8px 0">
      <label><input type="checkbox" bind:checked={config.reader.wheelTurnPage} onchange={saveConfig}  /> {t("settings.keys.wheelTurnPage")}</label>
      <label><input type="checkbox" bind:checked={config.reader.anim} onchange={saveConfig}  /> {t("settings.keys.anim")}</label>
      <label><input type="checkbox" bind:checked={config.reader.immersive} onchange={saveConfig}  /> {t("settings.keys.immersive")}</label>
    </div>
    <div class="bind-list">
      {#each BIND_DEFS as def (def.id)}
        <div class="bind-row">
          <span class="bind-name">{labelOfId("bind", def.id)}</span>
          <div class="bind-keys">
            {#each config.reader.bindings[def.id] as k (k)}
              <button class="key-chip" title={t("settings.keys.chipDelete")} onclick={() => removeBind(def.id, k)}>{k} ×</button>
            {/each}
            <button class="key-add" class:cap={capturing === def.id} onclick={() => (capturing = capturing === def.id ? null : def.id)}>
              {capturing === def.id ? t("settings.keys.capture") : ADD_BIND_GLYPH}
            </button>
          </div>
        </div>
      {/each}
    </div>

    <h4 class="sub-h">{t("settings.keys.zonesTitle")}</h4>
    <p class="tip">{t("settings.keys.zonesTip")}</p>
    <div class="checks" style="margin:6px 0">
      <label><input type="checkbox" bind:checked={config.reader.zoneHints} onchange={saveConfig}  /> {t("settings.keys.zoneHints")}</label>
    </div>
    <div class="zone-grid">
      {#each ZONE_DEFS as z (z.id)}
        <div class="zone-cell">
          <span class="zone-label">{labelOfId("zone", z.id)}</span>
          <select
            value={config.reader.zones[z.id]}
            onchange={(e) => { config.reader.zones[z.id] = (e.currentTarget as HTMLSelectElement).value as ZoneAction; saveConfig(); }}
          >
            <option value="none">{t("settings.keys.zoneNone")}</option>
            {#each BIND_DEFS as d (d.id)}
              <option value={d.id}>{labelOfId("bind", d.id)}</option>
            {/each}
          </select>
        </div>
      {/each}
    </div>

    <div class="close-row">
      <button onclick={() => { config.reader.bindings = structuredClone(DEFAULT_BINDINGS); config.reader.zones = structuredClone(DEFAULT_ZONES); saveConfig(); toast(t("settings.keys.resetDone")); }}>{t("settings.keys.reset")}</button>
    </div>
  </div>
  <div class="set-pane" data-pane="tts" hidden={tab !== "tts"}>
    <div class="field">
      <label>{t("settings.tts.engine")}</label>
      <select bind:value={config.tts.engine} onchange={saveConfig}>
        <option value="system">{t("settings.tts.engineSystem")}</option>
        <option value="online">{t("settings.tts.engineOnline")}</option>
      </select>
    </div>

    {#if config.tts.engine === "online"}
      <div class="field">
        <label>{t("settings.tts.serviceList")}</label>
        <div class="row-inline">
          <select bind:value={config.tts.activeService} onchange={saveConfig}>
            {#each config.tts.services as s, i (s.id)}
              <option value={i}>{s.name || t("settings.tts.serviceFallback", { n: i + 1 })}</option>
            {/each}
          </select>
          <button class="icon" title={t("settings.tts.addTitle")} onclick={addTtsService}>{t("settings.tts.add")}</button>
          <button class="icon" title={t("settings.tts.delTitle")} onclick={delTtsService}>{t("settings.tts.del")}</button>
        </div>
      </div>

      <div class="field">
        <label>{t("settings.tts.protocol")}</label>
        <select bind:value={tsvc.provider} onchange={() => onTtsProtocolChange(tsvc)}>
          {#each TTS_PROVIDER_METAS as p (p.id)}
            <option value={p.id}>{t(p.labelKey)}</option>
          {/each}
        </select>
      </div>

      <div class="grid2">
        <div class="field">
          <label>{t("settings.tts.name")}</label>
          <input bind:value={tsvc.name} onchange={saveConfig} placeholder={t("settings.tts.namePlaceholder")} />
        </div>
        <div class="field">
          <label>{t("settings.tts.baseUrl")}</label>
          <input bind:value={tsvc.baseUrl} onchange={saveConfig} placeholder={tsvcMeta.baseHint} title={tsvcMeta.baseHint} />
        </div>
      </div>
      {#if !tsvcMeta.hideKeyModel}
        <div class="grid2">
          <div class="field">
            <label>{t("settings.tts.apiKey")}</label>
            <input type="password" bind:value={tsvc.apiKey} onchange={saveConfig} placeholder={t("settings.tts.apiKeyPlaceholder")} />
          </div>
          <div class="field">
            <label>{t("settings.tts.model")}</label>
            <input data-testid="tts-model" bind:value={tsvc.model} onchange={saveConfig} placeholder={tsvcMeta.modelHintKey ? t(tsvcMeta.modelHintKey) : tsvcMeta.modelHint} title={t("settings.tts.modelTip")} />
          </div>
        </div>
      {/if}
      {#if tsvc.provider === "baidu"}
        <div class="field">
          <label>Secret Key</label>
          <input type="password" bind:value={tsvc.secretKey} onchange={saveConfig} placeholder={t("settings.tts.bdSecret")} />
        </div>
      {/if}
      {#if tsvc.provider === "volcano"}
        <div class="grid2">
          <div class="field">
            <label>AppID</label>
            <input bind:value={tsvc.appId} onchange={saveConfig} placeholder={t("settings.tts.volcAppId")} />
          </div>
          <div class="field">
            <label>Cluster</label>
            <input bind:value={tsvc.cluster} onchange={saveConfig} placeholder="volcano_tts" />
          </div>
        </div>
      {/if}
      <div class="grid2">
        <div class="field">
          <label>{t("settings.tts.voice")}</label>
          <div class="row-inline">
            {#if ttsVoiceList.length}
              <select bind:value={tsvc.voice} onchange={saveConfig}>
                <option value="">{t("settings.tts.voiceDefault")}</option>
                {#if ttsVoiceProfiles.length}
                  <optgroup label={t("settings.tts.voiceClonedGroup")}>
                    {#each ttsVoiceProfiles as v (v.id)}
                      <option value={v.id}>{v.name || v.id}{v.language ? ` · ${v.language}` : ""}</option>
                    {/each}
                  </optgroup>
                {/if}
                {#if ttsVoiceOthers.length}
                  <optgroup label={t("settings.tts.voiceOtherGroup")}>
                    {#each ttsVoiceOthers as v (v.id)}
                      <option value={v.id}>{v.name && v.name !== v.id ? `${v.name} (${v.id})` : v.id}</option>
                    {/each}
                  </optgroup>
                {/if}
              </select>
            {:else}
              <input data-testid="tts-voice" bind:value={tsvc.voice} onchange={saveConfig} placeholder={tsvcMeta.voiceHintKey ? t(tsvcMeta.voiceHintKey) : tsvcMeta.voiceHint} />
            {/if}
            {#if tsvcMeta.voiceList}
              <button class="icon" onclick={loadTtsVoices} disabled={ttsVoiceLoading}>{ttsVoiceLoading ? t("settings.tts.loading") : t("settings.tts.loadVoices")}</button>
            {/if}
          </div>
        </div>
        <div class="field">
          <label>{t("settings.tts.format")}</label>
          <select bind:value={tsvc.format} onchange={saveConfig}>
            <option value="wav">wav</option>
            <option value="mp3">mp3</option>
            <option value="opus">opus</option>
            <option value="aac">aac</option>
            <option value="flac">flac</option>
          </select>
        </div>
      </div>
      {#if ttsVoiceMsg}
        <p class="tip">{ttsVoiceMsg}</p>
      {/if}
    {:else if !ttsState.supported}
      <p class="tip" style="color:var(--danger)">{t("settings.tts.systemUnsupported")}</p>
    {:else}
      <div class="field">
        <label>{t("settings.tts.systemVoice")}</label>
        <select bind:value={config.tts.voice} onchange={saveConfig} title={t("settings.tts.systemVoiceTip")}>
          <option value="">{t("settings.tts.auto")}</option>
          {#each ttsState.voices as v (v.voiceURI)}
            <option value={v.name}>{v.name} · {v.lang}{v.default ? ` · ${t("settings.tts.voiceDefault")}` : ""}</option>
          {/each}
        </select>
        {#if !ttsState.voices.length}
          <p class="tip">{t("settings.tts.noVoice")}</p>
        {/if}
      </div>
    {/if}

    {#if config.tts.engine === "system"}
      <div class="field">
        <label>{t("settings.tts.autoLang")}</label>
        <input bind:value={config.tts.lang} onchange={saveConfig} placeholder="zh-CN" title={t("settings.tts.autoLangTip")} />
      </div>
    {/if}

    <div class="grid3">
      <div class="field">
        <label>{t("settings.tts.rate", { n: config.tts.rate.toFixed(2) })}</label>
        <input type="range" min="0.5" max="2" step="0.05" bind:value={config.tts.rate} oninput={saveConfig} />
      </div>
      <div class="field">
        <label>{t("settings.tts.pitch", { n: config.tts.pitch.toFixed(2) })}{config.tts.engine === "online" ? t("settings.tts.pitchOnlineNote") : ""}</label>
        <input type="range" min="0" max="2" step="0.05" bind:value={config.tts.pitch} oninput={saveConfig} disabled={config.tts.engine === "online"} />
      </div>
      <div class="field">
        <label>{t("settings.tts.volume", { n: Math.round(config.tts.volume * 100) })}</label>
        <input type="range" min="0" max="1" step="0.05" bind:value={config.tts.volume} oninput={saveConfig} />
      </div>
    </div>

    <div class="field">
      <label>{t("settings.tts.timerStop")}</label>
      <select bind:value={config.tts.sleepMinutes} onchange={saveConfig}>
        <option value={0}>{t("settings.tts.off")}</option>
        <option value={5}>{t("tts.minutes", { n: 5 })}</option>
        <option value={10}>{t("tts.minutes", { n: 10 })}</option>
        <option value={15}>{t("tts.minutes", { n: 15 })}</option>
        <option value={30}>{t("tts.minutes", { n: 30 })}</option>
        <option value={60}>{t("tts.minutes", { n: 60 })}</option>
      </select>
    </div>

    <div class="checks">
      <label><input type="checkbox" bind:checked={config.tts.autoNextChapter} onchange={saveConfig}  /> {t("settings.tts.autoNext")}</label>
      <label><input type="checkbox" bind:checked={config.tts.highlight} onchange={saveConfig}  /> {t("settings.tts.highlight")}</label>
    </div>
  </div>
  <div class="set-pane" data-pane="webdav" hidden={tab !== "webdav"}>
    <div class="checks" style="margin-bottom:12px">
      <label><input type="checkbox" bind:checked={config.webdav.enabled} onchange={saveConfig}  /> {t("settings.dav.enabled")}</label>
      <label><input type="checkbox" bind:checked={config.webdav.syncOnOpen} onchange={saveConfig}  /> {t("settings.dav.syncOnOpen")}</label>
      <label><input type="checkbox" bind:checked={config.webdav.uploadBooks} onchange={saveConfig}  /> {t("settings.dav.uploadBooks")}</label>
    </div>

    <div class="field">
      <label>{t("settings.dav.server")}</label>
      <input placeholder="https://dav.jianguoyun.com/dav/" bind:value={config.webdav.base} onchange={saveConfig} />
    </div>
    <div class="grid2">
      <div class="field">
        <label>{t("settings.dav.account")}</label>
        <input bind:value={config.webdav.user} onchange={saveConfig} />
      </div>
      <div class="field">
        <label>{t("settings.dav.password")}</label>
        <input type="password" bind:value={config.webdav.pass} onchange={saveConfig} />
      </div>
    </div>

    <div class="field">
      <label>{t("settings.dav.downloadDir")}</label>
      <div class="row-inline">
        <span class="flex1" title={config.webdav.downloadDir}>{config.webdav.downloadDir || t("settings.dav.notSet")}</span>
        <button onclick={pickWebdavDir}>{t("settings.dav.pick")}</button>
      </div>
    </div>

    <div class="row-inline" style="margin: 6px 0 16px">
      <button onclick={testWebdav} disabled={webdavTesting}>{t("settings.dav.test")}</button>
      {#if webdavStatus}<span class="status">{webdavStatus}</span>{/if}
    </div>

    <p class="sec-title" style="margin-top:18px">{t("settings.dav.backupTitle")}</p>
    <div class="row-inline" style="margin: 6px 0 10px">
      <button onclick={backupWebdavSettingsNow} disabled={webdavBackupBusy}>{webdavBackupBusy ? t("settings.dav.backuping") : t("settings.dav.backupNow")}</button>
      <button onclick={loadBackupList} disabled={backupListBusy}>{backupListBusy ? t("settings.tts.loading") : t("settings.dav.reloadList")}</button>
    </div>
    <div class="row-inline" style="margin: 6px 0 10px">
      <select class="flex1" bind:value={selectedBackup} disabled={!backupList.length}>
        {#if !backupList.length}<option value="">{t("settings.dav.noBackup")}</option>{/if}
        {#each backupList as b (b.name)}
          <option value={b.name}>{backupLabel(b)}</option>
        {/each}
      </select>
      <button onclick={restoreWebdavSettings} disabled={webdavRestoreBusy || !selectedBackup}>{webdavRestoreBusy ? t("settings.dav.restoring") : t("settings.dav.restore")}</button>
    </div>
    <div class="checks">
      <label><input type="checkbox" bind:checked={config.webdav.backup.enabled} onchange={saveConfig}  /> {t("settings.dav.backupEnabled")}</label>
    </div>
    <div class="grid2" style="margin-top:6px">
      <div class="field">
        <label>{t("settings.dav.interval")}</label>
        <select bind:value={config.webdav.backup.intervalHours} onchange={saveConfig}>
          <option value={6}>{t("common.hours", { n: 6 })}</option>
          <option value={12}>{t("common.hours", { n: 12 })}</option>
          <option value={24}>{t("common.hours", { n: 24 })}</option>
          <option value={48}>{t("common.hours", { n: 48 })}</option>
        </select>
      </div>
      <div class="field">
        <label>{t("settings.dav.lastBackup")}</label>
        <span class="status" style="padding:6px 0;display:block">{lastBackupText}{config.webdav.backup.lastError ? t("settings.dav.recentFail", { err: config.webdav.backup.lastError }) : ""}</span>
      </div>
    </div>
  </div>
  <div class="set-pane" data-pane="ai" hidden={tab !== "ai"}>
    <div class="checks" style="margin-bottom:12px">
      <label><input type="checkbox" bind:checked={config.ai.enabled} onchange={saveConfig}  /> {t("settings.ai.enabled")}</label>
      <label><input type="checkbox" bind:checked={config.ai.autoOnChapter} onchange={saveConfig}  /> {t("settings.ai.autoOnChapter")}</label>
    </div>

    <!-- 三个可折叠、可拖拽排序的分类；折叠态不持久化（本组件每次打开重挂载，天然全展开）。 -->
    <!-- 展开中的分组禁止拖动（避免拖到内部控件）；折叠后仍可拖拽排序。 -->
    <div class="ai-groups" use:dragSort={{ onDrop: aiGroupDrop, axis: "y", disabled: (id) => !!aiOpen[id] }}>
      {#each aiGroups as g (g.id)}
        <section class="ai-group" data-drag-id={g.id}>
          <div class="ai-group-head" class:open={aiOpen[g.id]} onclick={() => (aiOpen[g.id] = !aiOpen[g.id])}>
            {g.title}
          </div>
          {#if aiOpen[g.id]}
            <!-- 分类体阻止 pointerdown 冒泡：拖滑块 / 文本框选字不会误触发整组排序。 -->
            <div class="ai-group-body" transition:slide={{ duration: 200 }} onpointerdown={(e) => e.stopPropagation()}>
              {#if g.id === "node"}
                <div class="field">
                  <label>{t("settings.ai.currentEndpoint")}</label>
                  <div class="row-inline">
                    <select bind:value={config.ai.active} onchange={saveConfigNow}>
                      {#each config.ai.profiles as p, i (p.id)}
                        <option value={i}>{p.name || t("settings.ai.endpointFallback", { n: i + 1 })}</option>
                      {/each}
                    </select>
                    <button class="icon" title={t("settings.ai.addEndpointTitle")} onclick={addProfile}>{t("settings.ai.add")}</button>
                    <button class="icon" title={t("settings.ai.delEndpointTitle")} onclick={delProfile}>{t("settings.ai.del")}</button>
                  </div>
                  <p class="tip">{t("settings.ai.endpointTip")}</p>
                </div>

                <div class="grid2">
                  <div class="field">
                    <label>{t("settings.ai.endpointName")}</label>
                    <input bind:value={ap.name} onchange={saveConfig} />
                  </div>
                  <div class="field">
                    <label for="ai-provider-sel">{t("settings.ai.provider")}</label>
                    <select id="ai-provider-sel" bind:value={ap.provider} onchange={saveConfig}>
                      {#each AI_PROVIDERS as prov (prov.id)}
                        <option value={prov.id}>{labelOfId("aiProvider", prov.id)}</option>
                      {/each}
                    </select>
                  </div>
                </div>

                <div class="field">
                  <label>{t("settings.ai.baseUrl")}</label>
                  <input bind:value={ap.baseUrl} onchange={saveConfig} placeholder={apMeta.baseUrlHint} title={apMeta.id === "ollama" ? t("settings.ai.baseUrlOllama") : apMeta.azure ? t("settings.ai.baseUrlAzure") : apMeta.id === "gemini" ? t("settings.ai.baseUrlGemini") : apMeta.id === "anthropic" ? t("settings.ai.baseUrlAnthropic") : t("settings.ai.baseUrlGeneric")} />
                  <p class="tip">{apMeta.id === "ollama" ? t("settings.ai.baseUrlTipOllama") : apMeta.azure ? t("settings.ai.baseUrlTipAzure") : apMeta.id === "gemini" ? t("settings.ai.baseUrlTipGemini") : apMeta.id === "anthropic" ? t("settings.ai.baseUrlTipAnthropic") : t("settings.ai.baseUrlTipGeneric")}</p>
                </div>

                {#if apMeta.azure}
                  <div class="grid2">
                    <div class="field">
                      <label>{t("settings.ai.deployment")}</label>
                      <input bind:value={ap.deployment} onchange={saveConfig} placeholder="gpt-4o-deploy" title={t("settings.ai.deploymentTip")} />
                    </div>
                    <div class="field">
                      <label>{t("settings.ai.apiVersion")}</label>
                      <input bind:value={ap.apiVersion} onchange={saveConfig} placeholder="2024-06-01" title="Azure api-version" />
                    </div>
                  </div>
                {/if}

                <div class="grid2">
                  <div class="field">
                    <label>{t("settings.tts.model")}</label>
                    <input bind:value={ap.model} onchange={saveConfig} placeholder={apMeta.modelHint} />
                  </div>
                  {#if apMeta.needsKey}
                    <div class="field">
                      <label>API Key</label>
                      <input type="password" bind:value={ap.apiKey} onchange={saveConfig} />
                    </div>
                  {/if}
                </div>

                <div class="grid2">
                  <div class="field">
                    <label>{t("settings.ai.proxy")}</label>
                    <select bind:value={ap.proxyMode} onchange={saveConfig}>
                      <option value="auto">{t("settings.ai.proxyAuto")}</option>
                      <option value="direct">{t("settings.ai.proxyDirect")}</option>
                      <option value="custom">{t("settings.ai.proxyCustom")}</option>
                    </select>
                  </div>
                  {#if ap.proxyMode === "custom"}
                    <div class="field">
                      <label>{t("settings.ai.proxyUrl")}</label>
                      <input bind:value={ap.proxyUrl} onchange={saveConfig} placeholder="http://127.0.0.1:7897" />
                    </div>
                  {/if}
                </div>

                <div class="field">
                  <label class="check">
                    <input type="checkbox" bind:checked={config.ai.failover} onchange={saveConfigNow} />
                    {t("settings.ai.failover")}
                  </label>
                </div>

                <div class="row-inline">
                  <button onclick={testAi} disabled={aiTesting}>{aiTesting ? t("settings.ai.testing") : t("settings.ai.test")}</button>
                  <div class="flex1"></div>
                </div>
                {#if aiTestResult}
                  <div class="ai-test-result" class:ok={aiTestResult.startsWith("✓")} class:bad={aiTestResult.startsWith("✗")}>{aiTestResult}</div>
                {/if}

                <p class="tip">{t("settings.ai.endpointOrderTip")}</p>
              {:else if g.id === "window"}
                <div class="field">
                  <label>{t("settings.ai.windowStyle")}</label>
                  <select bind:value={config.ai.display} onchange={saveConfig}>
                    <option value="chat">{t("settings.ai.styleChat")}</option>
                    <option value="reader">{t("settings.ai.styleReader")}</option>
                  </select>
                  <p class="tip">{t("settings.ai.windowStyleTip")}</p>
                </div>

                {#if config.ai.display === "reader"}
                  <div class="field">
                    <label class="check">
                      <input type="checkbox" bind:checked={config.ai.style.followReader} onchange={saveConfig} />
                      {t("settings.ai.followReader")}
                    </label>
                  </div>
                  {#if !config.ai.style.followReader}
                    <div class="field">
                      <div class="row-inline">
                        <button onclick={copyReaderStyleToAi} title={t("settings.ai.copyReaderStyleTitle")}>{t("settings.ai.copyReaderStyle")}</button>
                        <span class="tip" style="align-self:center">{t("settings.ai.notFollowTip")}</span>
                      </div>
                    </div>
                    <div class="grid2">
                      <div class="field">
                        <label>{t("settings.font.title")}</label>
                        <input bind:value={config.ai.style.fontFamily} onchange={saveConfig} placeholder={config.reader.fontFamily} />
                      </div>
                      <div class="field">
                        <label>{t("settings.ai.fontSize", { n: config.ai.style.fontSize || config.reader.fontSize })}</label>
                        <input type="range" min="0" max="64" bind:value={config.ai.style.fontSize} oninput={saveConfig} />
                      </div>
                    </div>
                    <div class="grid2">
                      <div class="field">
                        <label>{t("settings.ai.lineHeight", { n: config.ai.style.lineHeight || config.reader.lineHeight })}</label>
                        <input type="range" min="0" max="3" step="0.05" bind:value={config.ai.style.lineHeight} oninput={saveConfig} />
                      </div>
                      <div class="field">
                        <label>{t("settings.ai.paraSpacing", { n: config.ai.style.paraSpacing || config.reader.paraSpacing })}</label>
                        <input type="range" min="0" max="2" step="0.1" bind:value={config.ai.style.paraSpacing} oninput={saveConfig} />
                      </div>
                    </div>
                    <div class="grid2">
                      <div class="field">
                        <label>{t("settings.ai.marginH", { n: config.ai.style.marginH || config.reader.marginH })}</label>
                        <input type="range" min="0" max="560" bind:value={config.ai.style.marginH} oninput={saveConfig} />
                      </div>
                      <div class="field">
                        <label>{t("settings.ai.marginV", { n: config.ai.style.marginV || config.reader.marginV })}</label>
                        <input type="range" min="0" max="400" bind:value={config.ai.style.marginV} oninput={saveConfig} />
                      </div>
                    </div>
                    <p class="tip">{t("settings.ai.zeroFollows")}</p>
                    <div class="grid2">
                      <div class="field">
                        <label>{t("settings.color.bg")}</label>
                        <div class="color-row">
                          <input placeholder={t("settings.ai.bgPlaceholder")} title={t("settings.ai.bgTip")} bind:value={config.ai.style.bgColor} oninput={saveConfig} />
                          <ColorPicker bind:value={config.ai.style.bgColor} allowEmpty emptyLabel={t("settings.ai.followReaderColor")} title={t("settings.ai.bgColorTitle")} onchange={saveConfig} />
                        </div>
                      </div>
                      <div class="field">
                        <label>{t("settings.color.fg")}</label>
                        <div class="color-row">
                          <input placeholder={t("settings.ai.fgPlaceholder")} title={t("settings.ai.fgTip")} bind:value={config.ai.style.fgColor} oninput={saveConfig} />
                          <ColorPicker bind:value={config.ai.style.fgColor} allowEmpty emptyLabel={t("settings.ai.followReaderColor")} title={t("settings.ai.fgColorTitle")} onchange={saveConfig} />
                        </div>
                      </div>
                    </div>
                    <div class="field">
                      <label>{t("settings.font.indent")}</label>
                      <select bind:value={config.ai.style.indent} onchange={saveConfig}>
                        <option value="0em">{t("settings.font.indent0")}</option>
                        <option value="1em">{t("settings.font.indent1")}</option>
                        <option value="2em">{t("settings.font.indent2")}</option>
                        <option value="3em">{t("settings.font.indent3")}</option>
                      </select>
                    </div>
                  {/if}
                  <div class="field">
                    <label>{t("settings.ai.css")}</label>
                    <textarea class="cssbox" rows="3" bind:value={config.ai.style.cssExtra} oninput={saveConfig} placeholder={".ai-mark { color: #333; }"}></textarea>
                    <p class="tip">{t("settings.ai.cssTip")}</p>
                  </div>
                {/if}
              {:else if g.id === "prompt"}
                <div class="field">
                  <label>{t("settings.ai.systemPrompt")}</label>
                  <input bind:value={config.ai.system} onchange={saveConfig} title={t("settings.ai.systemPromptTip")} />
                </div>

                <div class="field">
                  <label>{t("settings.ai.selPrompt")}</label>
                  <textarea rows="3" bind:value={config.ai.quickPrompt} oninput={saveConfig}></textarea>
                  <p class="tip">{t("settings.ai.selPromptTip")}</p>
                </div>

                <div class="field">
                  <label>{t("settings.ai.myPrompts")}</label>
                  {#each config.ai.prompts as p, i (p.id)}
                    <div class="prompt-edit">
                      <div class="row-inline" style="margin-bottom:4px">
                        <input class="pname" placeholder={t("settings.ai.promptName")} bind:value={p.name} onchange={saveConfig} />
                        <button class="key-chip-inline" class:cap={capPromptIdx === i} title={t("settings.ai.bindKey")}
                          onclick={() => (capPromptIdx = capPromptIdx === i ? null : i)}>
                          {p.key || t("settings.ai.bindKey")}{capPromptIdx === i ? "…" : ""}
                        </button>
                        {#if capPromptIdx === i}
                          <button class="icon" onclick={() => (capPromptIdx = null)}>{t("settings.ai.cancel")}</button>
                        {/if}
                        <button class="icon" title={t("settings.ai.delPromptTitle")} onclick={() => delPrompt(i)}>×</button>
                      </div>
                      <textarea rows="3" bind:value={p.text} oninput={saveConfig}></textarea>
                    </div>
                  {/each}
                  <button onclick={addPrompt} style="margin-top:6px">{t("settings.ai.addPrompt")}</button>
                </div>

                <p class="tip">{t("settings.ai.promptTip")}</p>
              {/if}
            </div>
          {/if}
        </section>
      {/each}
    </div>
  </div>
  <div class="set-pane" data-pane="prefs" hidden={tab !== "prefs"}>
    <p class="sec-title">{t("settings.tab.prefs")}</p>
    <div class="field">
      <label>{t("settings.language.label")}</label>
      <select
        value={localeSetting.get()}
        onchange={(e) => setLocale((e.currentTarget as HTMLSelectElement).value)}
      >
        <option value="auto">{t("settings.language.auto")}</option>
        <option value="zh-CN">{t("settings.language.zhCN")}</option>
        <option value="en">{t("settings.language.en")}</option>
      </select>
      <p class="tip">{t("settings.language.tip")}</p>
    </div>

    <div class="field">
      <label class="check">
        <input type="checkbox" bind:checked={config.reader.multiWindow} onchange={saveConfig} />
        {t("settings.prefs.multiWindow")}
      </label>
      <p class="tip">{t("settings.prefs.multiWindowTip")}</p>
    </div>

    <div class="field">
      <label class="check">
        <input
          type="checkbox"
          checked={!config.library.showCoverProgress}
          onchange={(e) => { config.library.showCoverProgress = !(e.currentTarget as HTMLInputElement).checked; saveConfig(); }}
        />
        {t("settings.prefs.hideCoverProgress")}
      </label>
      <p class="tip">{t("settings.prefs.hideCoverProgressTip")}</p>
    </div>

    <p class="sec-title" style="margin-top:18px">{t("settings.storage.title")}</p>
    {#if storage}
      <div class="storage-card">
        <div class="st-row">
          <span class="st-label">{t("settings.storage.dataDir")}</span>
          <span class="st-val path" title={storage.data_dir}>{storage.data_dir}</span>
          <button class="icon" title={t("settings.storage.copyPath")} onclick={copyDataDir}>⧉</button>
        </div>
        <div class="st-row">
          <span class="st-label">{t("settings.storage.total")}</span>
          <span class="st-val strong">{fmtBytes(storage.total_bytes)}</span>
        </div>
        <div class="st-row">
          <span class="st-label">{t("settings.storage.covers")}</span>
          <span class="st-val">{fmtBytes(storage.covers_bytes)}</span>
        </div>
        <div class="st-row">
          <span class="st-label">{t("settings.storage.res")}</span>
          <span class="st-val">{fmtBytes(storage.res_bytes)}</span>
        </div>
        <div class="st-row">
          <span class="st-label">{t("settings.storage.fonts")}</span>
          <span class="st-val">{fmtBytes(storage.fonts_bytes)}</span>
        </div>
        <div class="st-row">
          <span class="st-label">{t("settings.storage.db")}</span>
          <span class="st-val">{fmtBytes(storage.db_bytes)}</span>
        </div>
        <div class="st-actions">
          <span class="st-cache">{t("settings.storage.clearable", { size: fmtBytes(storage.cache_bytes) })}</span>
          <button onclick={() => clearCache("covers")} disabled={storageBusy}>{t("settings.storage.clearCovers")}</button>
          <button onclick={() => clearCache("res")} disabled={storageBusy}>{t("settings.storage.clearRes")}</button>
          <button class="danger" onclick={() => clearCache("all")} disabled={storageBusy}>{t("settings.storage.clearAll")}</button>
        </div>
      </div>
    {:else}
      <p class="tip">{t("settings.storage.loading")}</p>
    {/if}

    <p class="sec-title" style="margin-top:18px">{t("settings.backup.title")}</p>
    <div class="row-inline" style="margin-bottom:10px">
      <button onclick={exportConfig}>{t("settings.backup.copy")}</button>
      <button onclick={importConfig}>{t("settings.backup.paste")}</button>
      <button onclick={rebuildIndex} disabled={rebuildingFts}>{rebuildingFts ? t("settings.backup.rebuilding") : t("settings.backup.rebuild")}</button>
    </div>

    <p class="sec-title" style="margin-top:18px">MOBI / AZW3 / TXT</p>
    <div class="field" style="margin-bottom:10px">
      <label>{t("settings.prefs.encoding")}</label>
      <select bind:value={mobiEnc} onchange={saveMobiEnc}>
        <option value="auto">{t("settings.ai.proxyAuto")}</option>
        <option value="utf-8">UTF-8</option>
        <option value="gbk">GBK / GB18030</option>
        <option value="big5">Big5</option>
        <option value="windows-1252">Windows-1252</option>
      </select>
    </div>
  </div>
  <div class="set-pane" data-pane="other" hidden={tab !== "other"}>
    <p class="sec-title">{t("settings.other.title")}</p>
    <div class="field">
      <label>{t("settings.other.version")}</label>
      <div class="sec-val">TidePage · {APP_VERSION}</div>
    </div>
  </div>
    </section>
  </div>

  <footer class="set-foot">
    <span class="ver">TidePage {APP_VERSION}</span>
    <button class="primary" data-testid="set-done" onclick={() => (ui.settingsOpen = false)}>{t("settings.done")}</button>
  </footer>
</div>

<style>
  /* ============================================================
     设置弹窗外壳：顶栏 + 左导航 + 右内容 + 底栏
     仅改观感与组织方式，标签页与字段逻辑不变。
     ============================================================ */
  .settings-root {
    display: flex;
    flex-direction: column;
    width: min(980px, 94vw);
    height: min(760px, 90vh);
    background: var(--bg-elev);
    color: var(--fg);
  }

  .set-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 20px;
    border-bottom: 1px solid var(--line);
    flex-shrink: 0;
  }

  .set-title {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-size: var(--fs-lg, 16px);
  }

  .set-title-main { font-weight: 600; letter-spacing: var(--ls-tight, -0.015em); }
  .set-crumb { color: var(--fg2); font-weight: 400; }
  .set-crumb-now { color: var(--fg2); font-size: var(--fs-sm, 13px); }

  .set-close { font-size: 18px; line-height: 1; }

  .set-main {
    flex: 1;
    display: flex;
    min-height: 0;
  }

  /* 左侧导航 */
  .set-nav {
    width: 208px;
    flex-shrink: 0;
    border-right: 1px solid var(--line);
    background: var(--panel2);
    padding: 12px 10px;
    overflow-y: auto;
  }

  .set-search { position: relative; margin-bottom: 10px; }
  .set-search input {
    width: 100%;
    min-height: 30px;
    font-size: var(--fs-xs, 12px);
    padding-right: 26px;
    box-sizing: border-box;
  }
  .set-search-clear {
    position: absolute;
    right: 4px;
    top: 50%;
    transform: translateY(-50%);
    width: 20px;
    height: 20px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: none;
    background: transparent;
    color: var(--fg2);
    cursor: pointer;
    border-radius: 50%;
    font-size: 14px;
    line-height: 1;
    transition: background var(--dur) var(--ease), color var(--dur) var(--ease);
  }
  .set-search-clear:hover {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--fg);
  }

  .set-nav-group {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    color: var(--fg2);
    opacity: 0.75;
    padding: 12px 10px 4px;
    text-transform: none;
  }

  .set-nav-item {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    text-align: left;
    border: none;
    background: transparent;
    color: var(--fg2);
    padding: 8px 10px;
    border-radius: var(--radius-sm, 8px);
    font-size: var(--fs-sm, 13px);
    min-height: 34px;
    box-shadow: none;
    transition: background var(--dur, 0.2s) var(--ease), color var(--dur, 0.2s) var(--ease);
  }

  .set-nav-item:hover:not(:disabled) {
    background: var(--hover-bg, rgba(0, 0, 0, 0.05));
    color: var(--fg);
    border-color: transparent;
  }

  .set-nav-item.active {
    background: var(--accent-soft, var(--hover-bg));
    color: var(--accent);
    font-weight: 600;
  }

  .set-nav-glyph {
    width: 18px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    opacity: 0.9;
    flex-shrink: 0;
  }

  .set-nav-label { flex: 1; }
  .set-nav-empty { padding: 16px 10px; color: var(--fg2); font-size: var(--fs-xs, 12px); }

  /* 搜索命中的「具体设置」结果列表 */
  .set-nav-results {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .set-result {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    width: 100%;
    text-align: left;
    border: none;
    background: transparent;
    color: var(--fg);
    padding: 6px 10px;
    border-radius: 8px;
    font-size: var(--fs-xs, 12px);
    cursor: pointer;
    transition: background var(--dur, 0.2s) var(--ease), color var(--dur, 0.2s) var(--ease);
  }
  .set-result:hover { background: color-mix(in srgb, var(--accent) 12%, transparent); }
  .set-result-label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .set-result-tab { flex-shrink: 0; color: var(--fg2); font-size: 11px; opacity: 0.8; }

  /* 定位高亮：类名由 JS 运行时添加，必须用 :global，否则会被 Svelte 当作未用选择器删除。
     box-shadow 用于画强调色光环（.field 未设 box-shadow，不冲突）；
     transition 用 !important 覆盖 .field 自带的 transition，使光环淡入淡出。 */
  :global(.fr-locate) {
    border-radius: 10px;
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 65%, transparent) !important;
    transition: box-shadow 0.35s var(--ease) !important;
  }

  /* 右侧内容 */
  .set-content {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    /* 兜底：任何子项（如漫画区块的滑块栅格）都不得撑出横向滚动条。 */
    overflow-x: hidden;
    padding: 20px 24px;
  }

  /* pane 常驻渲染、按需 hidden：normal 流下隐藏即可（显式声明防止被其它 display 规则覆盖）。 */
  .set-pane[hidden] { display: none; }

  /* 底栏 */
  .set-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 12px 20px;
    border-top: 1px solid var(--line);
    flex-shrink: 0;
  }

  .tabs {
    display: flex;
    gap: 4px;
    margin-bottom: 16px;
    border-bottom: 1px solid var(--line);
    padding-bottom: 0;
  }

  .tabs button {
    border: none;
    background: transparent;
    padding: 8px 18px;
    border-radius: 8px 8px 0 0;
    border-bottom: 2px solid transparent;
  }

  .tabs button.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
    font-weight: 600;
  }

  .grid2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }

  .grid3 {
    display: grid;
    /* minmax(0,1fr) 允许列收缩（1fr 隐含 min-width:auto 会被 range 固有最小宽撑出横向条）。 */
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr) minmax(0, 1fr);
    gap: 12px;
  }

  /* 栅格内滑块允许收缩并占满单元格，杜绝其固有最小宽把栅格撑出横向滚动条。 */
  .grid3 input[type="range"],
  .grid2 input[type="range"] {
    min-width: 0;
    width: 100%;
  }

  /* 漫画区块专用：窄面板下自动降为 2 列 / 1 列并换行，不溢出（仅漫画 tab 生效，不动其它 tab）。 */
  .mg-grid {
    grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
    align-items: start;
  }

  /* 按漫画显示方式禁用无关的那个边距字段（瀑布流只调左右、横向只调上下）：置灰 + 不可交互 */
  .mg-grid .field.mg-off {
    opacity: 0.45;
  }
  .mg-grid .field.mg-off input {
    cursor: not-allowed;
  }

  /* 高级折叠区：把顶栏/底栏显示项这类低频配置收起来，减少阅读排版页的视觉负担 */
  details.adv {
    margin: 4px 0 12px;
    border: 1px solid var(--line);
    border-radius: var(--radius-sm, 8px);
    background: var(--panel2);
  }
  details.adv > summary {
    cursor: pointer;
    padding: 8px 12px;
    font-size: var(--fs-sm, 13px);
    color: var(--fg2);
    list-style: none;
    user-select: none;
  }
  details.adv > summary::-webkit-details-marker { display: none; }
  details.adv > summary::before {
    content: "▸";
    display: inline-block;
    margin-right: 6px;
    transition: transform var(--dur, 0.2s) var(--ease);
  }
  details.adv[open] > summary::before { transform: rotate(90deg); }
  details.adv > summary:hover { color: var(--fg); }
  details.adv > .field { padding: 0 12px 10px; margin-bottom: 0; }

  /* AI 设置分类：可折叠 + 可拖拽排序（视觉对齐 details.adv，间距节奏同 .field）。 */
  .ai-groups {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .ai-group {
    border: 1px solid var(--line);
    border-radius: var(--radius-sm, 8px);
    background: var(--panel2);
  }
  .ai-group-head {
    cursor: pointer;
    padding: 8px 12px;
    font-size: var(--fs-sm, 13px);
    color: var(--fg2);
    user-select: none;
  }
  .ai-group-head::before {
    content: "▸";
    display: inline-block;
    margin-right: 6px;
    transition: transform var(--dur, 0.2s) var(--ease);
  }
  .ai-group-head.open::before { transform: rotate(90deg); }
  .ai-group-head:hover { color: var(--fg); }
  .ai-group-body {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 0 12px 12px;
  }

  .stat-grid {
    display: grid;
    grid-template-columns: 1fr auto auto auto;
    gap: 4px 10px;
    align-items: center;
    margin: 6px 0;
  }
  .stat-head {
    font-size: 12px;
    color: var(--fg2);
    font-weight: 500;
  }
  .stat-name {
    font-size: 13px;
  }
  .stat-cell {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .stat-mv {
    padding: 0 5px;
    font-size: 12px;
    line-height: 1.4;
  }

  /* ---- 存储空间卡片 ---- */
  .storage-card {
    border: 1px solid var(--line);
    border-radius: var(--radius-md, 12px);
    background: var(--bg-elev);
    overflow: hidden;
  }

  .st-row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 9px 14px;
    border-bottom: 1px solid var(--line);
    font-size: var(--fs-sm, 13px);
  }

  .st-row:last-of-type { border-bottom: none; }
  .st-label { color: var(--fg2); width: 88px; flex-shrink: 0; }
  .st-val { color: var(--fg); }
  .st-val.strong { font-weight: 600; }
  .st-val.path { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--fg2); font-size: var(--fs-xs, 12px); }

  .st-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
    border-top: 1px solid var(--line);
    background: var(--panel2);
  }

  .st-cache { flex: 1; color: var(--fg2); font-size: var(--fs-xs, 12px); }

  /* ---- 默认封面底图控件 ---- */
  .cover-ctl {
    display: flex;
    gap: 12px;
    align-items: flex-start;
  }
  .cover-preview {
    width: 96px;
    height: 144px;
    flex-shrink: 0;
    object-fit: cover;
    border-radius: var(--radius-sm, 8px);
    border: 1px solid var(--line);
    background: var(--panel2);
  }
  .cover-ctl-main { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 8px; }
  .cover-ctl-acts { display: flex; gap: 8px; flex-wrap: wrap; }

  /* ---- 外壳皮肤卡片 ---- */
  .skin-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
    gap: 10px;
  }

  .skin-card {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
    border-radius: var(--radius-md);
    border: 1.5px solid var(--line);
    background: var(--bg-elev);
    cursor: pointer;
  }

  .skin-card.active {
    border-color: var(--accent);
  }

  .sk-preview {
    position: relative;
    height: 54px;
    border-radius: var(--radius-sm);
    border: 1px solid;
    display: block;
    overflow: hidden;
  }

  .sk-panel {
    position: absolute;
    left: 8px;
    top: 8px;
    bottom: 8px;
    width: 26px;
    border-radius: 6px;
  }

  .sk-accent {
    position: absolute;
    right: 8px;
    top: 8px;
    width: 20px;
    height: 8px;
    border-radius: 4px;
  }

  .sk-bar {
    position: absolute;
    right: 8px;
    top: 24px;
    width: 52px;
    height: 6px;
    border-radius: 3px;
    opacity: 0.75;
  }

  .sk-bar.short {
    top: 36px;
    width: 34px;
    opacity: 0.5;
  }

  .sk-name {
    font-size: var(--fs-xs);
    text-align: center;
  }

  .sk-check {
    position: absolute;
    right: 8px;
    bottom: 6px;
    color: var(--accent);
    font-weight: 700;
  }

  /* ---- 阅读配色方案 ---- */
  .scheme-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .scheme-actions button {
    min-height: 26px;
    padding: 3px 10px;
    font-size: var(--fs-xs);
  }

  .scheme-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(104px, 1fr));
    gap: 10px;
    margin-top: 6px;
  }

  .scheme-grid.compact {
    grid-template-columns: repeat(auto-fill, minmax(88px, 1fr));
    margin: 8px 0 14px;
  }

  .scheme-card {
    position: relative;
    border-radius: var(--radius-md);
    border: 1.5px solid var(--line);
    background: var(--bg-elev);
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }

  .scheme-card.active {
    border-color: var(--accent);
  }

  .scheme-preview {
    position: relative;
    display: block;
    width: 100%;
    height: 62px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--line);
    cursor: pointer;
    padding: 9px;
  }

  .sp-line {
    display: block;
    height: 5px;
    border-radius: 3px;
    margin-bottom: 5px;
  }

  .sp-line.short {
    width: 60%;
  }

  .sp-chip {
    display: block;
    width: 22px;
    height: 10px;
    border-radius: 5px;
    margin-top: 2px;
  }

  .scheme-name {
    font-size: var(--fs-xs);
    text-align: center;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* 「本书已单独套用方案」提示 + 清除按钮 */
  .book-scheme-note {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    flex-wrap: wrap;
    margin-top: 6px;
    font-size: var(--fs-xs, 12px);
    color: var(--fg2);
  }
  .link-btn {
    border: none;
    background: transparent;
    color: var(--accent);
    min-height: auto;
    padding: 2px 4px;
    font-size: var(--fs-xs, 12px);
    font-weight: 600;
    border-radius: var(--radius-xs, 6px);
    cursor: pointer;
  }
  .link-btn:hover:not(:disabled) {
    background: var(--accent-soft);
    color: var(--accent);
  }

  .scheme-menu {
    display: flex;
    gap: 3px;
    justify-content: center;
  }

  .scheme-menu button {
    min-height: 22px;
    padding: 1px 6px;
    font-size: 12px;
  }

  button.save-target {
    cursor: pointer;
    border: 1.5px solid var(--line);
    background: var(--bg-elev);
    padding: 6px;
  }

  button.save-target:hover {
    border-color: var(--accent);
  }

  /* ---- 保存方案弹层 ---- */
  .scheme-save-backdrop {
    position: fixed;
    inset: 0;
    z-index: 1100;
    background: rgba(0, 0, 0, 0.4);
    display: grid;
    place-items: center;
  }

  .scheme-save {
    width: min(520px, 88vw);
    max-height: 80vh;
    overflow: auto;
    background: var(--bg-elev);
    color: var(--fg);
    border: 1px solid var(--line);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-lg);
    padding: var(--sp-4);
  }

  .ss-head {
    font-size: var(--fs-lg);
    font-weight: 600;
    margin-bottom: 4px;
  }

  .ss-new {
    display: flex;
    gap: 8px;
  }

  .ss-new input {
    flex: 1;
  }

  .color-row {
    display: flex;
    gap: 6px;
    align-items: center;
  }

  .color-row input:first-child {
    flex: 1;
  }

  .numbox {
    width: 74px;
    flex: none !important;
    text-align: center;
  }

  .flex1 { flex: 1; }

  .ai-test-result {
    font-size: 13px;
    padding: 8px 12px;
    border-radius: 8px;
    background: var(--panel2);
    word-break: break-all;
    line-height: 1.6;
  }

  .ai-test-result.ok { color: #2e7d43; border: 1px solid #2e7d4355; }
  .ai-test-result.bad { color: var(--danger); border: 1px solid #c0564f55; }

  .prompt-edit {
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 8px;
    margin-bottom: 8px;
  }

  .prompt-edit .pname {
    flex: 1;
    min-width: 120px;
  }

  .prompt-edit textarea {
    width: 100%;
    resize: vertical;
  }

  .key-chip-inline {
    font-size: 12px;
    font-family: Consolas, monospace;
    padding: 4px 10px;
    white-space: nowrap;
  }

  .key-chip-inline.cap {
    border-color: var(--accent);
    color: var(--accent);
    animation: pulse 1s infinite;
  }

  .picker {
    padding: 2px;
    width: 42px;
    height: 34px;
    cursor: pointer;
  }

  .bg-thumb {
    width: 72px;
    height: 46px;
    object-fit: cover;
    border-radius: 6px;
    border: 1px solid var(--line);
  }

  .font-input { flex: 1; }
  .font-value { flex: 1; font-family: inherit; }

  .font-list {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
    max-height: 130px;
    overflow-y: auto;
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 8px;
    margin-top: 8px;
  }

  .font-opt {
    font-size: 13px;
    padding: 3px 10px;
    border-radius: 14px;
  }

  .font-opt.active {
    border-color: var(--accent);
    color: var(--accent);
    font-weight: 600;
  }

  .font-group-title {
    flex-basis: 100%;
    font-size: 12px;
    font-weight: 700;
    color: var(--fg2);
    letter-spacing: 0.04em;
    margin-top: 2px;
  }

  .font-opt-wrap {
    position: relative;
    display: inline-flex;
  }

  .font-del {
    position: absolute;
    top: -6px;
    right: -6px;
    width: 16px;
    height: 16px;
    padding: 0;
    line-height: 1;
    border-radius: 50%;
    font-size: 12px;
    display: grid;
    place-items: center;
    background: var(--panel);
    border: 1px solid var(--line);
    color: var(--fg2);
  }

  .font-del:hover {
    border-color: var(--danger);
    color: var(--danger);
  }

  .checks {
    display: flex;
    flex-wrap: wrap;
    gap: 14px;
    margin: 6px 0 12px;
  }

  .checks label {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
    cursor: pointer;
  }

  .cssbox {
    font-family: Consolas, monospace;
    font-size: 13px;
    resize: vertical;
  }

  .status {
    font-size: 13px;
    align-self: center;
  }

  .status.ok {
    color: #2e7d43;
    font-weight: 600;
  }

  .presets {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .presets button {
    font-size: 12px;
    padding: 4px 10px;
  }

  textarea {
    font-family: Consolas, monospace;
    resize: vertical;
  }

  .toc-preview {
    border: 1px solid var(--line);
    border-radius: 8px;
    max-height: 200px;
    overflow-y: auto;
    padding: 6px 0;
    margin-bottom: 8px;
  }

  .tp-row {
    padding: 3px 14px;
    font-size: 13px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .tp-row.muted {
    color: var(--fg2);
  }

  .sub-h {
    margin: 14px 0 4px;
    font-size: 13px;
  }

  .zone-grid {
    display: grid;
    grid-template-columns: 1fr 1fr 1fr;
    gap: 8px;
    margin-top: 6px;
  }

  .zone-cell {
    display: flex;
    flex-direction: column;
    gap: 4px;
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 8px 10px;
  }

  .zone-label {
    font-size: 12px;
    color: var(--fg2);
  }

  .zone-cell select {
    width: 100%;
    font-size: 13px;
  }

  .bind-list {
    max-height: 46vh;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
    border: 1px solid var(--line);
    border-radius: 10px;
    padding: 6px;
  }

  .bind-row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 5px 8px;
    border-radius: 6px;
  }

  .bind-row:hover { background: var(--panel2); }

  .bind-name {
    width: 150px;
    flex-shrink: 0;
    font-size: 13px;
  }

  .bind-keys {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
    align-items: center;
  }

  .key-chip {
    background: var(--panel2);
    border: 1px solid var(--line);
    border-radius: 6px;
    padding: 2px 8px;
    font-size: 12px;
    font-family: Consolas, monospace;
  }

  .key-chip:hover {
    border-color: var(--danger);
    color: var(--danger);
  }

  .key-add {
    border-style: dashed;
    padding: 2px 10px;
    font-size: 12px;
    min-width: 34px;
  }

  .key-add.cap {
    border-color: var(--accent);
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, white);
    animation: pulse 1s infinite;
  }

  @keyframes pulse {
    50% { opacity: 0.55; }
  }

  .tip {
    font-size: 12px;
    color: var(--fg2);
    line-height: 1.7;
  }

  .close-row {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 10px;
    margin-top: 10px;
  }

  .ver {
    font-size: 12px;
    color: var(--fg2);
    opacity: 0.7;
    margin-right: auto;
  }

  .sec-title {
    font-size: 12px;
    font-weight: 700;
    color: var(--accent);
    letter-spacing: 0.06em;
    margin: 0 0 10px;
    text-transform: uppercase;
  }

  .sec-val {
    font-size: 13px;
    color: var(--fg);
    word-break: break-all;
    line-height: 1.5;
  }

  .tabs {
    gap: 2px;
  }

  .tabs button {
    position: relative;
    border: none;
    font-size: 13px;
    color: var(--fg2);
    padding: 8px 14px 10px;
    transition: color 0.18s;
  }

  .tabs button:hover {
    color: var(--fg);
    background: transparent;
  }

  .tabs button.active {
    color: var(--accent);
    font-weight: 600;
  }

  .tabs button.active::after {
    content: "";
    position: absolute;
    left: 14px;
    right: 14px;
    bottom: 2px;
    height: 2px;
    border-radius: 1px;
    background: var(--accent);
    transform-origin: left center;
    animation: tab-in 0.3s cubic-bezier(0.32, 0.72, 0, 1) both;
  }

  @keyframes tab-in {
    from { transform: scaleX(0); opacity: 0; }
    to { transform: scaleX(1); opacity: 1; }
  }

  .checks input[type="checkbox"],
  input[type="checkbox"] {
    width: 15px;
    height: 15px;
    accent-color: var(--accent);
    cursor: pointer;
  }

  input[type="range"] {
    accent-color: var(--accent);
    cursor: pointer;
  }

  .field {
    background: color-mix(in srgb, var(--panel) 60%, transparent);
    border-radius: 10px;
    padding: 9px 12px;
    border: 1px solid transparent;
    transition: border-color 0.18s, background 0.18s;
  }

  .field:focus-within {
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
    background: var(--panel);
  }

  /* 窄窗口（如弹出的 AI 独立窗内打开设置）：
     侧栏改为顶部横向导航条，内容占满整宽，杜绝横向滚动条并优化排版。
     判据用视口宽度：主窗口视口很大不会命中；AI 独立窗视口窄才命中。 */
  @media (max-width: 620px) {
    .settings-root {
      width: 100vw;
      height: 100vh;
    }
    .set-main {
      flex-direction: column;
      min-width: 0;
    }
    .set-nav {
      width: auto;
      flex-shrink: 0;
      border-right: none;
      border-bottom: 1px solid var(--line);
      padding: 8px 10px;
    }
    .set-nav-list {
      flex-direction: row;
      flex-wrap: nowrap;
      overflow-x: auto;
      gap: 6px;
      scrollbar-width: none;
    }
    .set-nav-list::-webkit-scrollbar { height: 0; }
    .set-nav-item {
      flex: 0 0 auto;
      white-space: nowrap;
    }
    /* 窄窗口下结果列表限高，避免撑高横向导航条。 */
    .set-nav-results {
      max-height: 40vh;
      overflow-y: auto;
    }
    .set-content {
      padding: 16px 16px 20px;
      overflow-x: hidden;
    }
  }
</style>
