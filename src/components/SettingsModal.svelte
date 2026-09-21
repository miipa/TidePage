<script lang="ts">
  import { api } from "../lib/api";
  import {
    config,
    saveConfig,
    saveConfigNow,
    THEMES,
    toast,
    ui,
    APP_BUILD,
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
    type AiProvider,
    type BindAction,
    type ZoneAction,
    type ThemeKey,
  } from "../lib/stores.svelte";
  import ColorPicker from "./ColorPicker.svelte";
  import { refreshOnlineVoices, refreshVoices, ttsState } from "../lib/tts.svelte";
  import type { TocEntry } from "../lib/types";

  type SettingsTab = "reader" | "split" | "keys" | "tts" | "webdav" | "ai" | "other";
  let { initialTab = "reader" as SettingsTab }: { initialTab?: SettingsTab } = $props();
  let tab = $state<SettingsTab>(initialTab);
  let webdavTesting = $state(false);
  let webdavStatus = $state("");
  let regexText = $state(config.regex || "");
  let rebuildingFts = $state(false);
  let dataDir = $state("");
  let mobiEnc = $state("auto");

  let ttsVoiceList = $state<string[]>([]);
  let ttsVoiceLoading = $state(false);
  let ttsVoiceMsg = $state("");

  async function loadTtsVoices() {
    if (config.tts.onlineSource !== "custom") return;
    ttsVoiceLoading = true;
    ttsVoiceMsg = "";
    try {
      // 同时写入 ttsState.onlineVoices，供阅读界面播放条的音色下拉复用。
      ttsVoiceList = await refreshOnlineVoices();
      ttsVoiceMsg = ttsVoiceList.length
        ? `共 ${ttsVoiceList.length} 个音色（来自服务）`
        : "服务未返回音色列表（需支持 /v1/voices）";
      if (ttsVoiceList.length && !config.tts.onlineVoice) {
        config.tts.onlineVoice = ttsVoiceList[0];
        saveConfig();
      }
    } catch (e) {
      ttsVoiceMsg = `读取失败：${e}`;
    } finally {
      ttsVoiceLoading = false;
    }
  }

  $effect(() => {
    if (tab === "tts") loadTtsVoices();
  });

  $effect(() => {
    if (tab !== "other") return;
    api
      .getSetting("mobi.encoding")
      .then((v) => (mobiEnc = v && v.trim() ? v.trim().toLowerCase() : "auto"))
      .catch(() => {});
  });

  async function saveMobiEnc() {
    try {
      await api.setMobiEncoding(mobiEnc);
      toast("MOBI 编码已设置，重新打开书籍后生效");
    } catch (e) {
      toast(String(e), "error");
    }
  }

  $effect(() => {
    if (tab !== "other") return;
    api
      .getAppDataDir()
      .then((d) => (dataDir = d))
      .catch(() => {});
  });

  async function exportConfig() {
    try {
      await copyText(JSON.stringify(config, null, 2));
      toast("配置已复制到剪贴板（可粘贴保存，日后导入恢复）");
    } catch (e) {
      toast(String(e), "error");
    }
  }

  async function importConfig() {
    try {
      const txt = await navigator.clipboard.readText();
      const obj = JSON.parse(txt);
      if (!obj || typeof obj !== "object" || !obj.reader) throw new Error("剪贴板不是有效的配置");
      Object.assign(config, {
        ...obj,
        reader: { ...config.reader, ...obj.reader },
        ai: {
          ...config.ai,
          ...(obj.ai || {}),
          style: { ...config.ai.style, ...(obj.ai?.style || {}) },
        },
      });
      saveConfig();
      toast("已从剪贴板导入配置");
    } catch (e) {
      toast("导入失败：" + e, "error");
    }
  }

  async function rebuildIndex() {
    rebuildingFts = true;
    try {
      const n = await api.rebuildFts();
      toast(`全文索引已重建，共 ${n} 个章节`);
    } catch (e) {
      toast(String(e), "error");
    } finally {
      rebuildingFts = false;
    }
  }

  function copyDataDir() {
    copyText(dataDir || "%APPDATA%\\com.a.flowread");
    toast("数据目录已复制");
  }

  const SPLIT_PRESETS: { name: string; pattern: string }[] = [
    {
      name: "章节+特殊篇（推荐）",
      pattern:
        "(?<=[　\\s])(?:(?:内容|文章)?简介|文案|前言|序章|楔子|正文(?!完|结)|终章|后记|番外|第?\\s*[\\d〇零一二两三四五六七八九十百千万壹贰叁肆伍陆柒捌玖拾佰仟]+\\s*(?:卷|章))(?![\\w\\d]).{0,30}$",
    },
    { name: "第X章/卷/回（宽松）", pattern: "第\\s*[0-9〇零一二两三四五六七八九十百千]{1,12}\\s*[章节卷回部篇]" },
    { name: "Chapter N", pattern: "^\\s*Chapter\\s+\\d+.*$" },
    { name: "数字编号（1. / 1、）", pattern: "^\\s*\\d+[、.．]\\s*\\S.*$" },
    { name: "分隔符（--- ===）", pattern: "^\\s*[-=*_]{3,}\\s*$" },
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
    "仿宋",
    "幼圆",
    "隶书",
    "思源黑体",
    "思源宋体",
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
      toast(fellBack ? `已删除字体「${family}」，正文字体已回退默认` : `已删除字体「${family}」`);
    } catch (e) {
      toast(String(e), "error");
    }
  }

  async function resetReader() {
    const ok = await appConfirm(
      "确定恢复「阅读」设置为默认吗？字体、主题、版式、快捷键与点击区域都会重置（不含 AI / WebDAV）。"
    );
    if (!ok) return;
    resetReaderConfig();
    toast("阅读设置已恢复默认");
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
      toast("字体平滑已保存，重启应用后生效");
    } catch (e) {
      toast(String(e), "error");
    }
  }

  const ap = $derived(
    config.ai.profiles[Math.min(config.ai.active, config.ai.profiles.length - 1)] ?? config.ai.profiles[0]
  );
  const apMeta = $derived(aiProviderMeta(ap?.provider ?? "openai"));

  function addProfile() {
    const provider: AiProvider = ap?.provider ?? "openai";
    const created = newAiProfile(config.ai.profiles, provider);
    config.ai.profiles.push(created);
    config.ai.active = config.ai.profiles.length - 1;
    saveConfigNow();
    toast(`已新增端点「${created.name}」，请填写地址与密钥`);
  }

  function delProfile() {
    if (config.ai.profiles.length <= 1) {
      toast("至少保留一个端点", "error");
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
    toast("已把当前阅读设置复制为 AI 窗口专用");
  }

  function addPrompt() {
    config.ai.prompts.push({ id: Date.now(), name: `提示${config.ai.prompts.length + 1}`, text: "{chapter}", key: "" });
    saveConfig();
  }

  function delPrompt(i: number) {
    if (config.ai.prompts.length <= 1) {
      toast("至少保留一个提示词", "error");
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
    aiTestResult = "测试中…（发送 1 条测试消息，最多等 25 秒）";
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
    try { key = localStorage.getItem("flowread.font.mig") ?? ""; } catch { /* ignore */ }
    if (key !== "1") {
      try { localStorage.setItem("flowread.font.mig", "1"); } catch { /* ignore */ }
      refreshFontNames(true);
    }
  });

  async function installFont() {
    const p = await api.pickFile("选择字体文件", [
      { name: "字体", exts: ["ttf", "otf", "ttc"] },
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
      toast(`字体「${family}」已安装并应用`);
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
      if (!silent) toast("已按字体内部名称更新");
    } catch (e) {
      if (!silent) toast(String(e), "error");
    }
  }

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
          toast(`${combo} 已从「${def.name}」移除`);
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
    toast("快捷键已恢复默认");
  }

  async function testWebdav() {
    webdavTesting = true;
    webdavStatus = "测试中…";
    try {
      const ok = await api.webdavTest(config.webdav.base, config.webdav.user, config.webdav.pass);
      webdavStatus = ok ? "✓ 连接成功" : "✗ 连接失败";
      if (!ok) toast("WebDAV 连接失败", "error");
    } catch (e) {
      webdavStatus = `✗ ${e}`;
    } finally {
      webdavTesting = false;
    }
  }

  async function pickWebdavDir() {
    const d = await api.pickFolder("选择 WebDAV 下载目录");
    if (!d) return;
    config.webdav.downloadDir = d;
    saveConfig();
  }

  async function saveRegex() {
    try {
      await api.saveSplitRules(regexText.trim());
      config.regex = regexText.trim();
      saveConfig();
      toast("分章规则已保存，重新打开书籍后生效");
    } catch (e) {
      toast(String(e), "error");
    }
  }

  let regexBookPath = $state("");
  let previewToc: TocEntry[] | null = $state(null);
  let previewing = $state(false);

  async function pickBgImage() {
    const p = await api.pickFile("选择背景图片", [
      { name: "图片", exts: ["png", "jpg", "jpeg", "webp", "bmp", "gif"] },
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
    const p = await api.pickFile("选择 TXT 文件测试分章", [{ name: "TXT", exts: ["txt"] }]);
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

<div class="modal-body settings">
  <div class="tabs">
    <button class:active={tab === "reader"} onclick={() => (tab = "reader")}>阅读</button>
    <button class:active={tab === "tts"} onclick={() => { tab = "tts"; refreshVoices(); }}>语音朗读</button>
    <button class:active={tab === "split"} onclick={() => (tab = "split")}>分章</button>
    <button class:active={tab === "keys"} onclick={() => (tab = "keys")}>按键</button>
    <button class:active={tab === "webdav"} onclick={() => (tab = "webdav")}>WebDAV</button>
    <button class:active={tab === "ai"} onclick={() => (tab = "ai")}>AI</button>
    <button class:active={tab === "other"} onclick={() => (tab = "other")}>其他</button>
  </div>

  {#if tab === "reader"}
    <div class="field">
      <label>翻页模式</label>
      <select bind:value={config.reader.mode} onchange={saveConfig}>
        <option value="page">左右翻页（按栏数分屏）</option>
        <option value="scroll">瀑布平滑滚动（单栏连续滚动，不受版式影响）</option>
      </select>
    </div>

    {#if config.reader.mode === "scroll"}
      <div class="field">
        <label>章节过渡</label>
        <select bind:value={config.reader.scrollBounce} onchange={saveConfig}>
          <option value={false}>无缝衔接（滚动到底自动续接上下章）</option>
          <option value={true}>缓冲过渡（超出阈值后触发翻页）</option>
        </select>
      </div>
      {#if config.reader.scrollBounce}
        <div class="field">
          <label>缓冲强度</label>
          <select bind:value={config.reader.scrollBounceLevel} onchange={saveConfig}>
            {#each Object.entries(SCROLL_BOUNCE_PRESETS) as [k, p] (k)}
              <option value={k}>{p.label}</option>
            {/each}
          </select>
          <p class="tip">档位越高，切换上/下一章所需的滚动量越大；仅在「缓冲过渡」下生效。</p>
        </div>
      {/if}
    {:else}
      <div class="grid2">
        <div class="field">
          <label>栏数（固定，不随窗口宽度改变）</label>
          <select bind:value={config.reader.duo} onchange={saveConfig}>
            <option value={false}>单栏</option>
            <option value={true}>双栏</option>
          </select>
        </div>
        <div class="field">
          <label>双栏栏间距 {config.reader.colGap}px</label>
          <input type="range" min="0" max="160" step="4" bind:value={config.reader.colGap} oninput={saveConfig} disabled={!config.reader.duo} />
        </div>
      </div>
    {/if}

    <div class="grid3">
      <div class="field">
        <label>顶栏信息位置</label>
        <select bind:value={config.reader.statusPos} onchange={saveConfig}>
          <option value="left">靠左</option>
          <option value="center">居中</option>
          <option value="right">靠右</option>
          <option value="none">隐藏</option>
        </select>
      </div>
      <div class="field">
        <label>信息文字色（如 #333，空=跟随）</label>
        <input placeholder="自动" bind:value={config.reader.statusColor} oninput={saveConfig} />
      </div>
      <div class="field">
        <label>顶栏背景色（空=跟随）</label>
        <input placeholder="自动" bind:value={config.reader.topbarColor} oninput={saveConfig} />
      </div>
    </div>

    <div class="field">
      <label class="check">
        <input type="checkbox" bind:checked={config.reader.multiWindow} onchange={saveConfig} />
        多窗口同时打开多本书（主窗口保留在书库，双击书用新窗口打开）
      </label>
    </div>
    <div class="field">
      <label>主题背景（预设）</label>
      <div class="themes">
        {#each Object.entries(THEMES) as [key, t] (key)}
          <button
            class="theme-dot"
            class:active={config.reader.theme === key && !config.reader.bgColor && !config.reader.fgColor}
            style="background:{t.bg}; color:{t.fg};"
            onclick={() => { config.reader.theme = key as ThemeKey; config.reader.bgColor = ""; config.reader.fgColor = ""; saveConfig(); }}
          >文</button>
        {/each}
      </div>
    </div>

    <div class="grid2">
      <div class="field">
        <label>背景颜色（自定义颜色代码，优先于预设）</label>
        <div class="color-row">
          <input
            placeholder="如 #f5f1e8 / rgb(245,241,232)，留空用预设"
            bind:value={config.reader.bgColor}
            oninput={saveConfig}
          />
          <ColorPicker bind:value={config.reader.bgColor} allowEmpty emptyLabel="跟随预设" title="背景颜色" onchange={saveConfig} />
        </div>
      </div>
      <div class="field">
        <label>文字颜色（自定义颜色代码）</label>
        <div class="color-row">
          <input
            placeholder="如 #3b3630 / #1a1a2e，留空用预设"
            bind:value={config.reader.fgColor}
            oninput={saveConfig}
          />
          <ColorPicker bind:value={config.reader.fgColor} allowEmpty emptyLabel="跟随预设" title="文字颜色" onchange={saveConfig} />
        </div>
      </div>
    </div>

    <div class="field">
      <label>背景图片（可选，叠加在背景色之上）</label>
      <div class="row-inline">
        {#if config.reader.bgImage}
          <img class="bg-thumb" src={"http://flowread.localhost/bg/" + config.reader.bgImage} alt="" />
          <button onclick={pickBgImage}>更换图片</button>
          <button onclick={removeBgImage}>移除图片</button>
        {:else}
          <button onclick={pickBgImage}>选择图片…</button>
        {/if}
      </div>
    </div>

    {#if config.reader.bgImage}
      <div class="field">
        <label>图片不透明度 {Math.round(config.reader.bgOpacity * 100)}%（100% 表示完全覆盖背景色）</label>
        <input type="range" min="0.05" max="1" step="0.05" bind:value={config.reader.bgOpacity} oninput={saveConfig} />
      </div>
    {/if}

    <div class="field">
      <label>字体（支持搜索系统字体，也可安装字体文件）</label>
      <div class="row-inline">
        <input
          class="font-input"
          placeholder="输入字体名筛选…"
          bind:value={fontFilter}
        />
        <input
          class="font-value"
          placeholder="当前字体"
          bind:value={config.reader.fontFamily}
          onchange={saveConfig}
        />
        <button onclick={installFont}>安装字体文件…</button>
        <button onclick={() => refreshFontNames()} title="读取字体文件中的真实名称，修复已装字体名不匹配">↻ 重读字体名</button>
      </div>
      <div class="font-list">
        {#if customFontOptions.length}
          <div class="font-group-title">已安装字体</div>
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
                title="删除该字体"
                onclick={(e) => { e.stopPropagation(); deleteCustomFont(f.family, f.file); }}
              >×</button>
            </span>
          {/each}
        {/if}
        <div class="font-group-title">系统字体</div>
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
      <label>西文字体（拉丁字母/数字优先使用，中文字形仍走上方字体）</label>
      <div class="row-inline">
        <input
          class="font-value"
          list="latin-font-options"
          placeholder="留空=跟随正文字体；可输入任意系统字体名"
          bind:value={config.reader.latinFont}
          onchange={saveConfig}
        />
        {#if config.reader.latinFont}
          <button class="icon" title="清除" onclick={() => { config.reader.latinFont = ""; saveConfig(); }}>×</button>
        {/if}
      </div>
      <datalist id="latin-font-options">
        {#each latinFontNames as f (f)}
          <option value={f}></option>
        {/each}
      </datalist>
      <p class="tip">用于改善中英混排时拉丁字符字形（如 Georgia / Times New Roman）；AI 窗口同步生效。</p>
    </div>

    <div class="grid3">
      <div class="field">
        <label>字号 {config.reader.fontSize}px</label>
        <input type="range" min="10" max="64" bind:value={config.reader.fontSize} oninput={saveConfig} />
      </div>
      <div class="field">
        <label>行距 {config.reader.lineHeight}</label>
        <input type="range" min="1.2" max="3" step="0.05" bind:value={config.reader.lineHeight} oninput={saveConfig} />
      </div>
      <div class="field">
        <label>段距 {config.reader.paraSpacing}em</label>
        <input type="range" min="0" max="2" step="0.1" bind:value={config.reader.paraSpacing} oninput={saveConfig} />
      </div>
    </div>

    <hr />
    <div class="field">
      <label class="check">
        <input type="checkbox" bind:checked={grayAA} onchange={saveWebviewArgs} />
        灰度抗锯齿（推荐，需重启应用生效）
      </label>
      <p class="tip">勾选后使用灰度抗锯齿，消除文字彩边、笔画更均匀柔和；取消勾选则使用系统 ClearType 子像素渲染。修改后需重启生效。</p>
    </div>

    <div class="grid3">
      <div class="field">
        <label>正文字重</label>
        <select bind:value={config.reader.fontWeight} onchange={saveConfig}>
          <option value={300}>细（300）</option>
          <option value={400}>常规（400）</option>
          <option value={500}>中（500）</option>
        </select>
      </div>
      <div class="field">
        <label>对齐方式</label>
        <select bind:value={config.reader.textAlign} onchange={saveConfig}>
          <option value="justify">两端对齐</option>
          <option value="left">左对齐</option>
        </select>
      </div>
      <div class="field">
        <label>每行字数上限 {config.reader.maxLineChars > 0 ? config.reader.maxLineChars + " 字" : "关闭"}</label>
        <input type="range" min="0" max="60" step="2" bind:value={config.reader.maxLineChars} oninput={saveConfig} />
      </div>
    </div>
    <p class="tip">「每行字数上限」大于 0 时按字号自动计算行宽并居中（忽略左右边距），适合宽屏长文；伪粗体/伪斜体已统一禁止，避免中文字体假加粗/假斜发糊。</p>
    <hr />

    <div class="grid3">
      <div class="field">
        <label>左右边距 {config.reader.maxLineChars > 0 ? "（已由每行字数上限接管）" : config.reader.marginH + "px"}</label>
        <div class="row-inline">
          <input type="range" min="0" max="560" bind:value={config.reader.marginH} oninput={saveConfig} disabled={config.reader.maxLineChars > 0} />
          <input class="numbox" type="number" min="0" max="560" bind:value={config.reader.marginH} onchange={saveConfig} disabled={config.reader.maxLineChars > 0} />
        </div>
      </div>
      <div class="field">
        <label>上下边距 {config.reader.marginV}px</label>
        <div class="row-inline">
          <input type="range" min="0" max="400" bind:value={config.reader.marginV} oninput={saveConfig} />
          <input class="numbox" type="number" min="0" max="400" bind:value={config.reader.marginV} onchange={saveConfig} />
        </div>
      </div>
      <div class="field">
        <label>首行缩进</label>
        <select bind:value={config.reader.indent} onchange={saveConfig}>
          <option value="0em">不缩进</option>
          <option value="1em">缩进 1 字</option>
          <option value="2em">缩进 2 字</option>
          <option value="3em">缩进 3 字</option>
        </select>
      </div>
    </div>

    <div class="checks">
      <label><input type="checkbox" bind:checked={config.reader.overrideBookCss} onchange={saveConfig} /> 用阅读版式覆盖书籍自带排版（推荐）</label>
      <label><input type="checkbox" bind:checked={config.reader.pageTurnKeys} onchange={saveConfig} /> 键盘翻页（方向键/空格）</label>
    </div>
    <p class="tip">开启后强制使用上方字号/字重/行距/段距/缩进/对齐与文字颜色，忽略 EPUB 自带排版；关闭后尽量保留书籍原有版式（字体始终使用上方选择）。</p>

    <hr />
    <div class="grid3">
      <div class="field">
        <label>划线默认样式（选中文字后仍可临时选择其它样式）</label>
        <select bind:value={config.reader.highlightStyle} onchange={saveConfig}>
          {#each HIGHLIGHT_STYLES as s (s.id)}
            <option value={s.id}>{s.name}</option>
          {/each}
        </select>
      </div>
      <div class="field">
        <label>划线颜色（空=默认红棕）</label>
        <div class="color-row">
          <input placeholder="如 #c0392b / rgb(192,57,43)，留空用默认" bind:value={config.reader.highlightColor} oninput={saveConfig} />
          <ColorPicker bind:value={config.reader.highlightColor} allowEmpty emptyLabel="默认红棕" title="划线默认颜色" onchange={saveConfig} />
        </div>
      </div>
      <div class="field">
        <label>浏览器搜索链接（%s 为选中文字）</label>
        <input placeholder="https://www.baidu.com/s?wd=%s" bind:value={config.reader.browserSearchUrl} oninput={saveConfig} />
      </div>
    </div>
    <p class="tip">选中正文文字后会弹出划词菜单：可直接划线（多种样式）、在本书内全文搜索、用浏览器搜索、AI 翻译释义。</p>

    <div class="field">
      <label>自定义 CSS（追加到正文样式之后，优先级最高；行距/字距等可在此微调）</label>
      <textarea class="cssbox" rows="6" bind:value={config.reader.cssExtra} oninput={saveConfig} placeholder={"p { letter-spacing: 0.05em; }\nh1 { color: #a00; }"}></textarea>
    </div>

    <div class="close-row" style="margin-top:12px">
      <button onclick={resetReader}>恢复阅读设置默认…</button>
    </div>
  {:else if tab === "split"}
    <p class="tip">
      TXT 按正则划分章节：匹配到的每一行将作为章节标题，到下一个标题之间为章节内容。支持前后查找断言（lookbehind / lookahead）。留空使用内置默认规则。
    </p>

    <div class="field">
      <label>常用预设（点击填入）</label>
      <div class="presets">
        {#each SPLIT_PRESETS as p (p.name)}
          <button onclick={() => (regexText = p.pattern)}>{p.name}</button>
        {/each}
      </div>
    </div>

    <div class="field">
      <label>正则表达式</label>
      <textarea rows="4" bind:value={regexText} spellcheck="false"></textarea>
    </div>

    <div class="row-inline" style="margin-bottom:12px">
      <button onclick={pickRegexBook}>{regexBookPath ? "换一本书测试" : "选择 TXT 测试"}</button>
      <button onclick={previewRegex} disabled={previewing}>{previewing ? "解析中…" : "预览章节"}</button>
      {#if previewToc !== null}
        <span class="status" class:ok={previewToc.length > 1}>识别出 {previewToc.length} 个章节</span>
      {/if}
    </div>

    {#if previewToc !== null && previewToc.length}
      <div class="toc-preview">
        {#each previewToc.slice(0, 100) as t (t.index)}
          <div class="tp-row">{t.index + 1}. {t.title}</div>
        {/each}
        {#if previewToc.length > 100}
          <div class="tp-row muted">… 共 {previewToc.length} 章</div>
        {/if}
      </div>
    {/if}

    <div class="close-row">
      <button class="primary" onclick={saveRegex}>保存规则</button>
    </div>
  {:else if tab === "keys"}
    <p class="tip">
      一个功能可以绑定多个按键。点「记录」后按下任意键（可含 Ctrl/Alt/Shift 组合）即添加；点按键标签删除。
    </p>
    <div class="checks" style="margin:8px 0">
      <label><input type="checkbox" bind:checked={config.reader.wheelTurnPage} onchange={saveConfig} /> 翻页模式下滚轮翻页</label>
      <label><input type="checkbox" bind:checked={config.reader.anim} onchange={saveConfig} /> 翻页动画</label>
    </div>
    <div class="bind-list">
      {#each BIND_DEFS as def (def.id)}
        <div class="bind-row">
          <span class="bind-name">{def.name}</span>
          <div class="bind-keys">
            {#each config.reader.bindings[def.id] as k (k)}
              <button class="key-chip" title="点击删除" onclick={() => removeBind(def.id, k)}>{k} ×</button>
            {/each}
            <button class="key-add" class:cap={capturing === def.id} onclick={() => (capturing = capturing === def.id ? null : def.id)}>
              {capturing === def.id ? "请按键…" : "＋"}
            </button>
          </div>
        </div>
      {/each}
    </div>

    <h4 class="sub-h">点击区域动作（阅读界面内轻点画面）</h4>
    <p class="tip">选取文字后松开不触发；仅快速单击计为一次点击。可勾选「显示区域提示」查看各区域分布。</p>
    <div class="checks" style="margin:6px 0">
      <label><input type="checkbox" bind:checked={config.reader.zoneHints} onchange={saveConfig} /> 显示区域提示</label>
    </div>
    <div class="zone-grid">
      {#each ZONE_DEFS as z (z.id)}
        <div class="zone-cell">
          <span class="zone-label">{z.name}</span>
          <select
            value={config.reader.zones[z.id]}
            onchange={(e) => { config.reader.zones[z.id] = (e.currentTarget as HTMLSelectElement).value as ZoneAction; saveConfig(); }}
          >
            <option value="none">无</option>
            {#each BIND_DEFS as d (d.id)}
              <option value={d.id}>{d.name}</option>
            {/each}
          </select>
        </div>
      {/each}
    </div>

    <div class="close-row">
      <button onclick={() => { config.reader.bindings = structuredClone(DEFAULT_BINDINGS); config.reader.zones = structuredClone(DEFAULT_ZONES); saveConfig(); toast("已恢复默认"); }}>恢复默认</button>
    </div>
  {:else if tab === "tts"}
    <p class="tip">
      这里集中管理「语音朗读」的全部设置，与 AI 助手设置相互独立。系统语音离线可用；在线神经语音更自然（可接本地 VoxCPM、GPT-SoVITS、CosyVoice 等 OpenAI 兼容服务）。
    </p>

    <div class="field">
      <label>朗读引擎</label>
      <select bind:value={config.tts.engine} onchange={saveConfig}>
        <option value="system">系统语音（离线，使用 Windows 语音）</option>
        <option value="online">在线神经语音（更自然，需一个 OpenAI 兼容 TTS 服务）</option>
      </select>
    </div>

    {#if config.tts.engine === "online"}
      <div class="field">
        <label>语音服务来源</label>
        <select bind:value={config.tts.onlineSource} onchange={saveConfig}>
          <option value="custom">自定义服务地址（本地/远程，推荐接本地 VoxCPM）</option>
          <option value="profile">复用「AI 设置」里的某个端点</option>
        </select>
      </div>

      {#if config.tts.onlineSource === "custom"}
        <div class="grid2">
          <div class="field">
            <label>服务地址（OpenAI 兼容 /v1/audio/speech）</label>
            <input bind:value={config.tts.customBaseUrl} onchange={saveConfig} placeholder="http://127.0.0.1:8000" />
          </div>
          <div class="field">
            <label>密钥（本地服务留空）</label>
            <input type="password" bind:value={config.tts.customApiKey} onchange={saveConfig} placeholder="可留空" />
          </div>
        </div>
      {:else}
        <div class="field">
          <label>使用端点</label>
          <select bind:value={config.tts.onlineProfileId} onchange={saveConfig}>
            <option value={0}>当前 AI 端点</option>
            {#each config.ai.profiles as p (p.id)}
              <option value={p.id}>{p.name || `端点 ${p.id}`}</option>
            {/each}
          </select>
        </div>
      {/if}

      <div class="grid2">
        <div class="field">
          <label>语音模型（本地 VoxCPM 可填 voxcpm；OpenAI 用 tts-1）</label>
          <input bind:value={config.tts.onlineModel} onchange={saveConfig} placeholder="tts-1" />
        </div>
        <div class="field">
          <label>音色</label>
          {#if config.tts.onlineSource === "custom"}
            <div class="row-inline">
              <select bind:value={config.tts.onlineVoice} onchange={saveConfig}>
                <option value="">默认（服务端参考音色）</option>
                {#each ttsVoiceList as v (v)}
                  <option value={v}>{v}</option>
                {/each}
              </select>
              <button onclick={loadTtsVoices} disabled={ttsVoiceLoading}>{ttsVoiceLoading ? "读取中…" : "读取音色"}</button>
            </div>
          {:else}
            <input list="tts-online-voice-list" bind:value={config.tts.onlineVoice} onchange={saveConfig} placeholder="alloy" />
          {/if}
        </div>
      </div>
      {#if config.tts.onlineSource === "custom"}
        <div class="field">
          <label>或直接填参考音频 .wav 完整路径（音色会固定用该音频）</label>
          <input bind:value={config.tts.onlineVoice} onchange={saveConfig} placeholder="D:\VoxCPM\voices\ref_male.wav" />
        </div>
        {#if ttsVoiceMsg}<p class="tip">{ttsVoiceMsg}</p>{/if}
      {/if}
      <div class="field">
        <label>音频格式（本地 VoxCPM 服务请选 wav）</label>
        <select bind:value={config.tts.onlineFormat} onchange={saveConfig}>
          <option value="wav">wav</option>
          <option value="mp3">mp3</option>
          <option value="opus">opus</option>
          <option value="aac">aac</option>
          <option value="flac">flac</option>
        </select>
      </div>
      <datalist id="tts-online-voice-list">
        <option value="alloy"></option>
        <option value="echo"></option>
        <option value="fable"></option>
        <option value="onyx"></option>
        <option value="nova"></option>
        <option value="shimmer"></option>
        <option value="coral"></option>
        <option value="sage"></option>
      </datalist>
      <p class="tip">
        需目标服务提供 OpenAI 兼容的 <code>/v1/audio/speech</code>（OpenAI、Azure OpenAI、vLLM-Omni 的 VoxCPM、GPT-SoVITS 封装等）。
        某句失败会自动回退系统语音；语速映射为播放速度，音调对在线语音无效。
      </p>
    {:else if !ttsState.supported}
      <p class="tip" style="color:var(--danger)">当前环境不支持系统语音朗读。</p>
    {:else}
      <div class="field">
        <label>系统语音（空=按语言自动选择）</label>
        <select bind:value={config.tts.voice} onchange={saveConfig}>
          <option value="">自动（按语言）</option>
          {#each ttsState.voices as v (v.voiceURI)}
            <option value={v.name}>{v.name} · {v.lang}{v.default ? " · 默认" : ""}</option>
          {/each}
        </select>
        {#if !ttsState.voices.length}
          <p class="tip">未检测到语音，请确认系统已安装语音包（设置 → 时间和语言 → 语音）。</p>
        {/if}
      </div>
    {/if}

    {#if config.tts.engine === "system"}
      <div class="field">
        <label>自动选择语言（未指定语音时按此语言挑系统语音）</label>
        <input bind:value={config.tts.lang} onchange={saveConfig} placeholder="zh-CN" />
      </div>
    {/if}

    <div class="grid3">
      <div class="field">
        <label>语速 {config.tts.rate.toFixed(2)}</label>
        <input type="range" min="0.5" max="2" step="0.05" bind:value={config.tts.rate} oninput={saveConfig} />
      </div>
      <div class="field">
        <label>音调 {config.tts.pitch.toFixed(2)}{config.tts.engine === "online" ? "（在线无效）" : ""}</label>
        <input type="range" min="0" max="2" step="0.05" bind:value={config.tts.pitch} oninput={saveConfig} disabled={config.tts.engine === "online"} />
      </div>
      <div class="field">
        <label>音量 {Math.round(config.tts.volume * 100)}%</label>
        <input type="range" min="0" max="1" step="0.05" bind:value={config.tts.volume} oninput={saveConfig} />
      </div>
    </div>

    <div class="field">
      <label>定时停止默认值</label>
      <select bind:value={config.tts.sleepMinutes} onchange={saveConfig}>
        <option value={0}>关闭</option>
        <option value={5}>5 分钟</option>
        <option value={10}>10 分钟</option>
        <option value={15}>15 分钟</option>
        <option value={30}>30 分钟</option>
        <option value={60}>60 分钟</option>
      </select>
    </div>

    <div class="checks">
      <label><input type="checkbox" bind:checked={config.tts.autoNextChapter} onchange={saveConfig} /> 一章读完自动续读下一章</label>
      <label><input type="checkbox" bind:checked={config.tts.highlight} onchange={saveConfig} /> 朗读时高亮当前句并滚动跟随</label>
    </div>
    <p class="tip">
      在阅读界面点「▶」开始朗读：优先从选中文字开始，否则从当前屏幕顶部开始；播放条可暂停/继续、上一句/下一句、停止，并可设置定时停止与语速。
    </p>
  {:else if tab === "webdav"}
    <div class="checks" style="margin-bottom:12px">
      <label><input type="checkbox" bind:checked={config.webdav.enabled} onchange={saveConfig} /> 启用 WebDAV</label>
      <label><input type="checkbox" bind:checked={config.webdav.syncOnOpen} onchange={saveConfig} /> 打开/退出书籍时自动同步进度（同一软件多端，新的覆盖旧的）</label>
      <label><input type="checkbox" bind:checked={config.webdav.uploadBooks} onchange={saveConfig} /> 打开书籍时上传书文件</label>
    </div>

    <div class="field">
      <label>服务器地址</label>
      <input placeholder="https://dav.jianguoyun.com/dav/" bind:value={config.webdav.base} onchange={saveConfig} />
    </div>
    <div class="grid2">
      <div class="field">
        <label>账号</label>
        <input bind:value={config.webdav.user} onchange={saveConfig} />
      </div>
      <div class="field">
        <label>密码 / 应用密码</label>
        <input type="password" bind:value={config.webdav.pass} onchange={saveConfig} />
      </div>
    </div>

    <div class="field">
      <label>书籍下载目录（WebDAV 文件页下载到此目录，并自动加入书库）</label>
      <div class="row-inline">
        <span class="flex1" title={config.webdav.downloadDir}>{config.webdav.downloadDir || "未设置"}</span>
        <button onclick={pickWebdavDir}>选择…</button>
      </div>
    </div>

    <div class="row-inline" style="margin: 6px 0 16px">
      <button onclick={testWebdav} disabled={webdavTesting}>测试连接</button>
      {#if webdavStatus}<span class="status">{webdavStatus}</span>{/if}
    </div>

    <p class="tip">
      进度存于 <code>FlowRead/progress/</code> 下每本书一个文件：进入书籍时拉取、退出书籍时上传、阅读页「⇅ 手动同步」按钮也可随时同步，本地与云端以时间新的一方为准。
      WebDAV 文件管理在书库页（左上「☁ 云盘」或向左滑）进入，点「连接并列出」后浏览整盘，下载书籍到上面的目录并自动入书库。
    </p>
  {:else if tab === "ai"}
    <div class="checks" style="margin-bottom:12px">
      <label><input type="checkbox" bind:checked={config.ai.enabled} onchange={saveConfig} /> 启用 AI 助手</label>
      <label><input type="checkbox" bind:checked={config.ai.autoOnChapter} onchange={saveConfig} /> 打开新章节自动执行第一个提示词</label>
    </div>

    <div class="field" style="margin-bottom:12px">
      <label>AI 窗口样式</label>
      <select bind:value={config.ai.display} onchange={saveConfig}>
        <option value="chat">对话样式（聊天气泡，原始）</option>
        <option value="reader">阅读样式（像正文一样翻页阅读，隐藏自己的提问）</option>
      </select>
      <p class="tip">阅读样式：AI 内容按阅读版式分栏分页，←/→ 或滚轮翻页，新回复自动翻到最新页；可单独设置 AI 窗口版式，也可完全跟随阅读界面。TXT/ePub 导出按钮在 AI 窗口顶部。</p>
    </div>

    {#if config.ai.display === "reader"}
      <div class="field" style="margin-bottom:12px">
        <label class="check">
          <input type="checkbox" bind:checked={config.ai.style.followReader} onchange={saveConfig} />
          跟随阅读界面设置（字体/字号/颜色/边距等与正文一致）
        </label>
      </div>
      {#if !config.ai.style.followReader}
        <div class="field" style="margin-bottom:10px">
          <div class="row-inline">
            <button onclick={copyReaderStyleToAi} title="把「阅读」页当前字体/字号/颜色等复制为 AI 窗口专用设置">复制当前阅读设置 →</button>
            <span class="tip" style="align-self:center">不跟随时可单独调 AI 窗口版式</span>
          </div>
        </div>
        <div class="grid2">
          <div class="field">
            <label>字体</label>
            <input bind:value={config.ai.style.fontFamily} onchange={saveConfig} placeholder={config.reader.fontFamily} />
          </div>
          <div class="field">
            <label>字号 {config.ai.style.fontSize || config.reader.fontSize}px（0=跟随）</label>
            <input type="range" min="0" max="64" bind:value={config.ai.style.fontSize} oninput={saveConfig} />
          </div>
        </div>
        <div class="grid2">
          <div class="field">
            <label>行距 {config.ai.style.lineHeight || config.reader.lineHeight}（0=跟随）</label>
            <input type="range" min="0" max="3" step="0.05" bind:value={config.ai.style.lineHeight} oninput={saveConfig} />
          </div>
          <div class="field">
            <label>段距 {config.ai.style.paraSpacing || config.reader.paraSpacing}em（0=跟随）</label>
            <input type="range" min="0" max="2" step="0.1" bind:value={config.ai.style.paraSpacing} oninput={saveConfig} />
          </div>
        </div>
        <div class="grid2">
          <div class="field">
            <label>左右边距 {config.ai.style.marginH || config.reader.marginH}px（0=跟随）</label>
            <input type="range" min="0" max="560" bind:value={config.ai.style.marginH} oninput={saveConfig} />
          </div>
          <div class="field">
            <label>上下边距 {config.ai.style.marginV || config.reader.marginV}px（0=跟随）</label>
            <input type="range" min="0" max="400" bind:value={config.ai.style.marginV} oninput={saveConfig} />
          </div>
        </div>
        <div class="grid2">
          <div class="field">
            <label>背景颜色（空=跟随阅读配色）</label>
            <div class="color-row">
              <input placeholder="如 #f5f1e8 / rgb(245,241,232)" bind:value={config.ai.style.bgColor} oninput={saveConfig} />
              <ColorPicker bind:value={config.ai.style.bgColor} allowEmpty emptyLabel="跟随阅读" title="AI 背景颜色" onchange={saveConfig} />
            </div>
          </div>
          <div class="field">
            <label>文字颜色（空=跟随阅读配色）</label>
            <div class="color-row">
              <input placeholder="如 #3b3630 / rgb(59,54,48)" bind:value={config.ai.style.fgColor} oninput={saveConfig} />
              <ColorPicker bind:value={config.ai.style.fgColor} allowEmpty emptyLabel="跟随阅读" title="AI 文字颜色" onchange={saveConfig} />
            </div>
          </div>
        </div>
        <div class="field" style="margin-bottom:10px">
          <label>首行缩进（AI 阅读样式）</label>
          <select bind:value={config.ai.style.indent} onchange={saveConfig}>
            <option value="0em">不缩进</option>
            <option value="1em">缩进 1 字</option>
            <option value="2em">缩进 2 字</option>
            <option value="3em">缩进 3 字</option>
          </select>
        </div>
      {/if}
      <div class="field" style="margin-bottom:12px">
        <label>AI 窗口自定义 CSS（仅阅读样式生效；作用于正文 .ai-mark）</label>
        <textarea class="cssbox" rows="3" bind:value={config.ai.style.cssExtra} oninput={saveConfig} placeholder={".ai-mark { color: #333; }"}></textarea>
        <p class="tip">勾选"跟随阅读界面"时，字号/行距/边距/缩进与正文完全一致；取消勾选后可单独设置。</p>
      </div>
    {/if}

    <div class="field">
      <label>当前端点（阅读界面 AI 面板也可快速切换；故障转移按此列表顺序依次尝试，当前端点优先）</label>
      <div class="row-inline">
        <select bind:value={config.ai.active} onchange={saveConfigNow}>
          {#each config.ai.profiles as p, i (p.id)}
            <option value={i}>{p.name || `端点${i + 1}`}</option>
          {/each}
        </select>
        <button class="icon" title="新增端点" onclick={addProfile}>＋ 新增</button>
        <button class="icon" title="删除当前端点" onclick={delProfile}>✕ 删除</button>
      </div>
    </div>

    <div class="grid2">
      <div class="field">
        <label>端点名称</label>
        <input bind:value={ap.name} onchange={saveConfig} />
      </div>
      <div class="field">
        <label for="ai-provider-sel">服务商</label>
        <select id="ai-provider-sel" bind:value={ap.provider} onchange={saveConfig}>
          {#each AI_PROVIDERS as prov (prov.id)}
            <option value={prov.id}>{prov.label}</option>
          {/each}
        </select>
      </div>
    </div>
    <div class="field">
      <label>
        {apMeta.id === "ollama"
          ? "Ollama 地址"
          : apMeta.azure
            ? "API 地址（Azure 资源端点）"
            : apMeta.id === "gemini"
              ? "API 地址（Google 生成式语言 API 域名）"
              : apMeta.id === "anthropic"
                ? "API 地址（Anthropic 域名）"
                : "API 地址（填到域名即可，自动补 /v1）"}
      </label>
      <input bind:value={ap.baseUrl} onchange={saveConfig} placeholder={apMeta.baseUrlHint} />
    </div>
    {#if apMeta.azure}
      <div class="grid2">
        <div class="field">
          <label>部署名（deployment）</label>
          <input bind:value={ap.deployment} onchange={saveConfig} placeholder="gpt-4o-deploy" />
        </div>
        <div class="field">
          <label>API 版本（api-version）</label>
          <input bind:value={ap.apiVersion} onchange={saveConfig} placeholder="2024-06-01" />
        </div>
      </div>
    {/if}
    <div class="grid2">
      <div class="field">
        <label>模型</label>
        <input bind:value={ap.model} onchange={saveConfig} placeholder={apMeta.modelHint} />
      </div>
      {#if apMeta.needsKey}
        <div class="field">
          <label>API Key</label>
          <input type="password" bind:value={ap.apiKey} onchange={saveConfig} />
        </div>
      {/if}
    </div>
    <div class="field">
      <label class="check">
        <input type="checkbox" bind:checked={config.ai.failover} onchange={saveConfigNow} />
        端点失败时自动切换下一个端点并重跑刚才的操作（按列表顺序，最多尝试全部端点）
      </label>
    </div>
    <div class="grid2">
      <div class="field">
        <label>网络代理</label>
        <select bind:value={ap.proxyMode} onchange={saveConfig}>
          <option value="auto">自动（优先生效环境变量，其次 Windows 系统代理；不解析 PAC）</option>
          <option value="direct">强制直连（忽略环境变量与系统代理，TUN 模式选这个）</option>
          <option value="custom">自定义代理</option>
        </select>
      </div>
      {#if ap.proxyMode === "custom"}
        <div class="field">
          <label>代理地址</label>
          <input bind:value={ap.proxyUrl} onchange={saveConfig} placeholder="http://127.0.0.1:7897" />
        </div>
      {/if}
    </div>
    <p class="tip">端点顺序即故障转移优先级；测试结果会显示实际命中的链路（自定义/自动/直连）与各链路失败原因。</p>

    <div class="row-inline" style="margin-bottom:10px">
      <button onclick={testAi} disabled={aiTesting}>{aiTesting ? "测试中…" : "测试当前端点"}</button>
      <div class="flex1"></div>
    </div>
    {#if aiTestResult}
      <div class="ai-test-result" class:ok={aiTestResult.startsWith("✓")} class:bad={aiTestResult.startsWith("✗")}>{aiTestResult}</div>
    {/if}

    <div class="field" style="margin-top:12px">
      <label>System 提示词（全局）</label>
      <input bind:value={config.ai.system} onchange={saveConfig} />
    </div>

    <div class="field">
      <label>划词翻译 / 释义提示词（阅读界面选中文字点「AI 释义」小窗执行；{"{selection}"} 为选中文字）</label>
      <textarea rows="3" bind:value={config.ai.quickPrompt} oninput={saveConfig}></textarea>
      <p class="tip">独立于下方「我的提示词」，不会影响 AI 面板的按钮与快捷键。</p>
    </div>

    <div class="field">
      <label>我的提示词（AI 面板按钮 + 快捷键执行；{"{chapter}"} 为章节内容，{"{title}"} 为章节名）</label>
      {#each config.ai.prompts as p, i (p.id)}
        <div class="prompt-edit">
          <div class="row-inline" style="margin-bottom:4px">
            <input class="pname" placeholder="名称" bind:value={p.name} onchange={saveConfig} />
            <button class="key-chip-inline" class:cap={capPromptIdx === i} title="绑定快捷键"
              onclick={() => (capPromptIdx = capPromptIdx === i ? null : i)}>
              {p.key || "绑定快捷键"}{capPromptIdx === i ? "…" : ""}
            </button>
            {#if capPromptIdx === i}
              <button class="icon" onclick={() => (capPromptIdx = null)}>取消</button>
            {/if}
            <button class="icon" title="删除此提示词" onclick={() => delPrompt(i)}>×</button>
          </div>
          <textarea rows="3" bind:value={p.text} oninput={saveConfig}></textarea>
        </div>
      {/each}
      <button onclick={addPrompt} style="margin-top:6px">＋ 添加提示词</button>
    </div>

    <p class="tip">AI 面板在阅读界面点「AI」按钮打开；提示词芯片点击即执行；F1–F9 快捷键全局有效。</p>
  {:else if tab === "other"}
    <p class="sec-title">关于</p>
    <div class="grid2">
      <div class="field">
        <label>构建版本</label>
        <div class="sec-val">FlowRead · {APP_BUILD}</div>
      </div>
      <div class="field">
        <label>数据目录（书库/封面/字体/配置）</label>
        <div class="row-inline">
          <span class="flex1 sec-val" title={dataDir}>{dataDir || "…"}</span>
          <button onclick={copyDataDir}>复制</button>
        </div>
      </div>
    </div>

    <p class="sec-title" style="margin-top:18px">数据</p>
    <div class="row-inline" style="margin-bottom:10px">
      <button onclick={exportConfig}>复制配置（备份）</button>
      <button onclick={importConfig}>从剪贴板导入配置</button>
      <button onclick={rebuildIndex} disabled={rebuildingFts}>{rebuildingFts ? "重建中…" : "重建全文索引"}</button>
    </div>
    <p class="tip">配置即阅读设置（字体/主题/布局/AI/WebDAV 等），导出为 JSON 文本备份；换机时在新机粘贴导入即可。</p>

    <p class="sec-title" style="margin-top:18px">MOBI / AZW3</p>
    <div class="field" style="margin-bottom:10px">
      <label>正文字符编码</label>
      <select bind:value={mobiEnc} onchange={saveMobiEnc}>
        <option value="auto">自动（内嵌 meta 声明 → 文件头声明 → 嗅探）</option>
        <option value="utf-8">强制 UTF-8</option>
        <option value="gbk">强制 GBK / GB18030（简体）</option>
        <option value="big5">强制 Big5（繁体）</option>
        <option value="windows-1252">强制 Windows-1252（西文）</option>
      </select>
      <p class="tip">旧版中文 MOBI 内部编码混乱导致乱码时手动指定，重新打开该书生效。带 DRM（AWZ3 加密）的书会在打开时直接提示需先解密，不再产生乱码。</p>
    </div>

    <p class="sec-title" style="margin-top:18px">自定义 CSS 说明</p>
    <p class="tip">
      自定义 CSS 作用于正文容器 <code>#chapter-content</code> 内部。可样式化的元素：
      <code>p</code> 段落、<code>h1–h6</code> 标题（TXT 分章标题为 <code>h2</code>）、<code>blockquote</code>、<code>a</code>、<code>img</code>、<code>table/td/th</code>。
      例：<code>#chapter-content p&#123; color:rgba(0,0,0,.75); &#125;</code>
      整体可用：<code>#chapter-content&#123; color:…; font-family:…; &#125;</code>
    </p>
  {/if}

  <div class="close-row">
    <span class="ver">FlowRead 构建 {APP_BUILD}</span>
    <button class="primary" onclick={() => (ui.settingsOpen = false)}>完成</button>
  </div>
</div>

<style>
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
    grid-template-columns: 1fr 1fr 1fr;
    gap: 12px;
  }

  .themes {
    display: flex;
    gap: 10px;
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
    font-size: 12.5px;
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
    font-size: 12.5px;
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
    font-size: 11px;
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
    font-size: 11px;
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

  .theme-dot {
    width: 44px;
    height: 44px;
    border-radius: 10px;
    border: 2px solid var(--line);
    cursor: pointer;
    font-size: 15px;
    display: grid;
    place-items: center;
  }

  .theme-dot.active {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 30%, transparent);
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
    font-size: 12.5px;
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
    font-size: 12.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .tp-row.muted {
    color: var(--fg2);
  }

  .sub-h {
    margin: 14px 0 4px;
    font-size: 13.5px;
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
    font-size: 12.5px;
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
    font-size: 12.5px;
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

  .moonlist {
    border: 1px solid var(--line);
    border-radius: 8px;
    max-height: 180px;
    overflow-y: auto;
  }

  .moonrow {
    display: flex;
    justify-content: space-between;
    padding: 5px 12px;
    font-size: 12.5px;
    border-bottom: 1px solid var(--line);
  }

  .moonrow:last-child { border-bottom: none; }
  .mt { font-weight: 500; }
  .mc { color: var(--fg2); }

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
    font-size: 11px;
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
    animation: tab-in 0.25s ease;
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
</style>
