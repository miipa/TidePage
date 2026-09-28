<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api } from "../lib/api";
  import { config, saveConfig, toast, ui } from "../lib/stores.svelte";

  interface Entry { name: string; is_dir: boolean; size: number; mtime: number }
  interface DlItem { key: string; name: string; pct: number; state: "downloading" | "done" | "error"; msg: string }

  const EXT_BOOK = ["txt", "epub", "mobi", "azw3", "prc"];

  let path = $state("");
  let entries = $state<Entry[]>([]);
  let loading = $state(false);
  let error = $state("");
  // 是否已完成至少一次列目录尝试（用于区分「首帧未加载」与「真·空目录」，避免闪现空态）。
  let attempted = $state(false);

  const downloads = $state<DlItem[]>([]);
  let dlUnlisteners: Record<string, () => void> = {};
  let dlSeq = 0;

  // 三要素齐全才认为可连接；否则展示「去设置」空状态。
  const configReady = $derived(!!(config.webdav.base && config.webdav.user && config.webdav.pass));
  // 仅统计真正在下载中的项来判断并发上限（已完成/失败项会在列表停留 2.6s，不应计入）。
  const activeDownloads = $derived(downloads.filter((d) => d.state === "downloading").length);

  // ---- 排序：字段 / 升降 / 文件夹置顶，三者持久化到 localStorage ----
  function lsGet(k: string, fallback: string): string {
    try { return localStorage.getItem(k) ?? fallback; } catch { return fallback; }
  }
  const sb0 = lsGet("flowread.wd.sortBy", "time");
  const sd0 = lsGet("flowread.wd.sortDir", "desc");
  let sortBy = $state<"name" | "time" | "size">(sb0 === "name" || sb0 === "size" ? sb0 : "time");
  let sortDir = $state<"asc" | "desc">(sd0 === "asc" ? "asc" : "desc");
  let foldersFirst = $state(lsGet("flowread.wd.foldersFirst", "1") !== "0");

  // 任一排序状态变化即写回（首次运行写入默认值，无害）。
  $effect(() => {
    try {
      localStorage.setItem("flowread.wd.sortBy", sortBy);
      localStorage.setItem("flowread.wd.sortDir", sortDir);
      localStorage.setItem("flowread.wd.foldersFirst", foldersFirst ? "1" : "0");
    } catch { /* ignore */ }
  });

  // 展示用的有序列表：先按「文件夹置顶」分组（与升降无关），再按所选字段+方向比较。
  const sortedEntries = $derived.by(() => {
    const list = [...entries];
    list.sort((a, b) => {
      if (foldersFirst && a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
      let cmp = 0;
      if (sortBy === "name") cmp = a.name.localeCompare(b.name, "zh");
      else if (sortBy === "time") cmp = (a.mtime || 0) - (b.mtime || 0);
      else cmp = (a.size || 0) - (b.size || 0);
      // 同值时用名称兜底，保证顺序稳定
      if (cmp === 0) cmp = a.name.localeCompare(b.name, "zh");
      return sortDir === "asc" ? cmp : -cmp;
    });
    return list;
  });

  function rememberedPath(): string {
    try { return localStorage.getItem("flowread.wd.path") ?? ""; } catch { return ""; }
  }

  /// 列目录：成功返回 true。失败只置 error（保留 path、不回连接页），交由界面展示错误+重试。
  async function load(p?: string): Promise<boolean> {
    if (p !== undefined) {
      path = p;
      try { localStorage.setItem("flowread.wd.path", p); } catch { /* ignore */ }
    }
    loading = true;
    error = "";
    let ok = false;
    try {
      const w = config.webdav;
      const items = await api.webdavList(w.base, w.user, w.pass, path);
      entries = items.filter((e) => e.name !== "" && e.name !== "." && e.name !== "..");
      ok = true;
    } catch (e) {
      error = String(e);
      entries = [];
    } finally {
      loading = false;
      attempted = true;
    }
    return ok;
  }

  /// 面板挂载即自动连接；首次失败约 600ms 后自动再试一次，仍失败才展示错误+重试。
  async function autoConnect() {
    if (!configReady) return;
    const ok = await load(rememberedPath());
    if (!ok) {
      await new Promise((r) => setTimeout(r, 600));
      await load();
    }
  }

  function up() {
    const parts = path.split("/").filter(Boolean);
    parts.pop();
    load(parts.join("/"));
  }

  function openEntry(e: Entry) {
    load(path ? `${path}/${e.name}` : e.name);
  }

  async function pickDir() {
    const d = await api.pickFolder("选择书籍下载目录");
    if (!d) return;
    config.webdav.downloadDir = d;
    saveConfig();
    toast(`下载目录：${d}`);
  }

  function isBook(e: Entry) {
    const ext = e.name.split(".").pop()?.toLowerCase() || "";
    return EXT_BOOK.includes(ext);
  }

  async function download(e: Entry) {
    if (!config.webdav.downloadDir) {
      toast("请先选择下载目录", "error");
      return;
    }
    const remote = path ? `${path}/${e.name}` : e.name;
    const key = `dl${++dlSeq}`;
    const item: DlItem = { key, name: e.name, pct: 0, state: "downloading", msg: "" };
    downloads.push(item);
    dlUnlisteners[key] = () => {};
    const unlisten = await listen<any>(`wd-dl:${key}`, (ev) => {
      const p = ev.payload;
      const it = downloads.find((d) => d.key === key);
      if (!it) return;
      if (p.total > 0) it.pct = Math.min(100, Math.round((p.received / p.total) * 100));
      else it.pct = it.pct >= 90 ? 90 : it.pct + 5;
      if (p.done) {
        if (p.err) {
          it.state = "error";
          it.msg = String(p.err);
        } else {
          it.state = "done";
          it.pct = 100;
        }
        setTimeout(() => {
          const i = downloads.findIndex((d) => d.key === key);
          if (i >= 0) downloads.splice(i, 1);
        }, 2600);
        dlUnlisteners[key]?.();
        delete dlUnlisteners[key];
      }
    });
    dlUnlisteners[key] = () => unlisten && unlisten();
    try {
      await api.webdavDownloadProgress(config.webdav.base, config.webdav.user, config.webdav.pass, remote, config.webdav.downloadDir, key);
    } catch (e2) {
      const it = downloads.find((d) => d.key === key);
      if (it) { it.state = "error"; it.msg = String(e2); }
      dlUnlisteners[key]?.();
      delete dlUnlisteners[key];
      toast(`下载失败：${e2}`, "error");
    }
  }

  function fmt(n: number): string {
    if (n >= 1048576) return (n / 1048576).toFixed(1) + " MB";
    if (n >= 1024) return (n / 1024).toFixed(1) + " KB";
    return n + " B";
  }

  function fmtTime(t: number): string {
    if (!t) return "";
    const d = new Date(t * 1000);
    const p = (x: number) => String(x).padStart(2, "0");
    return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
  }

  const hasUp = $derived(path.split("/").filter(Boolean).length > 0);

  onMount(() => {
    if (configReady) autoConnect();
  });
</script>

<div class="wd-browse">
  <div class="wd-head">
    <span class="wd-title">WebDAV 文件管理</span>
    <button class="icon" title="关闭" aria-label="关闭" onclick={() => (ui.webdavOpen = false)}>×</button>
  </div>

  {#if !configReady}
    <div class="wd-empty-cfg">
      <div class="wd-empty-ico">🔌</div>
      <p class="wd-empty-t">尚未配置 WebDAV 服务器</p>
      <p class="wd-empty-d">请先在 <b>设置 → WebDAV</b> 中填写服务器地址与账号密码并启用，之后打开本面板即可自动连接。</p>
    </div>
  {:else}
    <!-- 工具行：导航 + 当前路径 + 排序控件（窄抽屉下自动换行，路径省略号不撑破） -->
    <div class="wd-tools">
      <button class="wd-btn" onclick={() => load("")} disabled={!path} title="根目录" aria-label="根目录">⌂</button>
      <button class="wd-btn" onclick={up} disabled={!hasUp} title="上级目录" aria-label="上级目录">↑</button>
      <button class="wd-btn" onclick={() => load()} title="刷新当前目录" aria-label="刷新">⟳</button>
      <span class="wd-path" title={path || "/"}>{path || "/"}</span>
      <span class="wd-tools-gap"></span>
      <select class="wd-sel" bind:value={sortBy} title="排序方式" aria-label="排序方式">
        <option value="name">名称</option>
        <option value="time">时间</option>
        <option value="size">大小</option>
      </select>
      <button
        class="wd-btn"
        onclick={() => (sortDir = sortDir === "asc" ? "desc" : "asc")}
        title={sortDir === "asc" ? "升序（点击改降序）" : "降序（点击改升序）"}
        aria-label="切换升降序"
      >{sortDir === "asc" ? "↑" : "↓"}</button>
      <label class="wd-check" title="目录始终排在文件前面">
        <input type="checkbox" bind:checked={foldersFirst} /> 文件夹置顶
      </label>
    </div>

    <div class="wd-info">
      <span class="wd-info-label">下载目录</span>
      <span class="wd-info-val" title={config.webdav.downloadDir || "未设置"}>{config.webdav.downloadDir || "未设置"}</span>
      <button class="wd-btn" onclick={pickDir}>选择…</button>
    </div>

    <div class="wd-list">
      {#if loading || (!attempted && !error)}
        <div class="wd-state"><span class="wd-spin">⟳</span> 加载中…</div>
      {:else if error}
        <div class="wd-state wd-state-error">
          <div class="wd-err-msg">列目录失败：{error}</div>
          <button class="wd-btn wd-retry" onclick={() => load()}>重试</button>
        </div>
      {:else if sortedEntries.length === 0}
        <div class="wd-state">空目录或无权限</div>
      {:else}
        {#each sortedEntries as e (path + "/" + e.name + String(e.size))}
          {#if e.is_dir}
            <button class="wd-row wd-dir" onclick={() => openEntry(e)}>
              <span class="wd-ico">📁</span>
              <span class="wd-name">{e.name}</span>
            </button>
          {:else}
            <div class="wd-row">
              <span class="wd-ico">📄</span>
              <span class="wd-name">{e.name}</span>
              <span class="wd-meta">{fmt(e.size)}{e.mtime ? " · " + fmtTime(e.mtime) : ""}</span>
              <button class="wd-dl-btn" onclick={() => download(e)} disabled={activeDownloads >= 3}>
                {isBook(e) ? "下载入书库" : "下载"}
              </button>
            </div>
          {/if}
        {/each}
      {/if}
    </div>

    {#if downloads.length}
      <div class="wd-dlbox">
        {#each downloads as d (d.key)}
          <div class="wd-dl">
            <div class="wd-dl-top">
              <span class="wd-dl-name" title={d.name}>{d.name}</span>
              <span class="wd-dl-state" class:ok={d.state === "done"} class:bad={d.state === "error"}>
                {d.state === "done" ? "✓ 下载已完成" : d.state === "error" ? "下载失败" : `${d.pct}%`}
              </span>
            </div>
            <div class="wd-bar"><div class="wd-bar-fill" class:fin={d.state === "done"} class:fail={d.state === "error"} style="width:{d.pct}%"></div></div>
            {#if d.state === "error"}<div class="wd-dl-err">{d.msg}</div>{/if}
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .wd-browse {
    display: flex;
    flex-direction: column;
    min-width: 0;
    height: 100%;
    padding: 18px;
    box-sizing: border-box;
  }

  .wd-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 12px;
  }

  .wd-title {
    font-weight: 600;
    font-size: 16px;
  }

  /* ---- 未配置空状态 ---- */
  .wd-empty-cfg {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    text-align: center;
    padding: 0 24px;
  }

  .wd-empty-ico {
    font-size: 34px;
    opacity: 0.7;
  }

  .wd-empty-t {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }

  .wd-empty-d {
    margin: 0;
    font-size: 13px;
    line-height: 1.7;
    color: var(--fg2);
    max-width: 320px;
  }

  /* ---- 工具行 ---- */
  .wd-tools {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 8px;
  }

  .wd-tools-gap {
    flex: 1;
    min-width: 0;
  }

  .wd-path {
    font-family: Consolas, monospace;
    font-size: 12px;
    color: var(--fg2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
    flex: 1 1 120px;
    min-width: 0;
  }

  .wd-btn {
    padding: 4px 10px;
    font-size: 12px;
    line-height: 1.4;
    border: 1px solid var(--line);
    border-radius: 6px;
    background: var(--panel2);
    color: var(--fg);
    cursor: pointer;
  }

  .wd-btn:hover:not(:disabled) {
    border-color: var(--accent);
  }

  .wd-btn:disabled {
    opacity: 0.42;
    cursor: default;
  }

  .wd-sel {
    padding: 4px 8px;
    font-size: 12px;
    border: 1px solid var(--line);
    border-radius: 6px;
    background: var(--panel2);
    color: var(--fg);
    cursor: pointer;
  }

  .wd-check {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
    color: var(--fg2);
    white-space: nowrap;
    cursor: pointer;
  }

  .wd-check input {
    margin: 0;
    cursor: pointer;
  }

  /* ---- 下载目录行 ---- */
  .wd-info {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    margin-bottom: 10px;
  }

  .wd-info-label {
    color: var(--fg2);
    flex: 0 0 auto;
  }

  .wd-info-val {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--fg);
  }

  /* ---- 列表 ---- */
  .wd-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 4px;
  }

  .wd-row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    box-sizing: border-box;
    padding: 7px 8px;
    border-radius: 6px;
    text-align: left;
  }

  /* 目录行为 <button>：清除按钮默认样式，保留可点击观感 */
  button.wd-row {
    border: none;
    background: none;
    color: var(--fg);
    font: inherit;
    cursor: pointer;
  }

  .wd-row + .wd-row {
    border-top: 1px solid rgba(128, 128, 128, 0.12);
  }

  .wd-row:hover {
    background: var(--panel2);
  }

  .wd-dir:hover .wd-name {
    color: var(--accent);
  }

  .wd-ico {
    flex: 0 0 auto;
    font-size: 14px;
  }

  .wd-name {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .wd-meta {
    flex: 0 0 auto;
    opacity: 0.5;
    font-size: 12px;
    white-space: nowrap;
  }

  .wd-dl-btn {
    flex: 0 0 auto;
    padding: 3px 10px;
    font-size: 12px;
    border: 1px solid var(--line);
    border-radius: 6px;
    background: var(--panel2);
    color: var(--fg);
    cursor: pointer;
  }

  .wd-dl-btn:hover:not(:disabled) {
    border-color: var(--accent);
  }

  .wd-dl-btn:disabled {
    opacity: 0.42;
    cursor: default;
  }

  /* ---- 三态：加载 / 空 / 错误 ---- */
  .wd-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    min-height: 120px;
    padding: 20px;
    font-size: 13px;
    color: var(--fg2);
    text-align: center;
  }

  .wd-state-error .wd-err-msg {
    color: var(--danger);
    word-break: break-all;
    max-width: 100%;
  }

  .wd-retry {
    padding: 5px 18px;
  }

  .wd-spin {
    display: inline-block;
    animation: wd-rotate 1s linear infinite;
  }

  @keyframes wd-rotate {
    to { transform: rotate(360deg); }
  }

  /* ---- 下载进度 ---- */
  .wd-dlbox {
    margin-top: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-height: 40%;
    overflow-y: auto;
  }

  .wd-dl {
    border: 1px solid var(--line);
    border-radius: 10px;
    padding: 8px 10px;
    background: var(--panel2);
  }

  .wd-dl-top {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    font-size: 12px;
    margin-bottom: 6px;
  }

  .wd-dl-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .wd-dl-state.ok { color: #2e8b57; font-weight: 600; }
  .wd-dl-state.bad { color: var(--danger); font-weight: 600; }

  .wd-bar {
    height: 6px;
    border-radius: 3px;
    background: rgba(128, 128, 128, 0.18);
    overflow: hidden;
  }

  .wd-bar-fill {
    height: 100%;
    border-radius: 3px;
    background: linear-gradient(90deg, var(--accent), #3fae7a);
    background-size: 200% 100%;
    transition: width 0.25s ease;
    animation: wd-slide 1.6s linear infinite;
  }

  .wd-bar-fill.fin {
    background: #2e8b57;
    animation: none;
  }

  .wd-bar-fill.fail {
    background: var(--danger);
    animation: none;
  }

  .wd-dl-err {
    margin-top: 6px;
    font-size: 12px;
    color: var(--danger);
    word-break: break-all;
    max-height: 44px;
    overflow-y: auto;
  }

  @keyframes wd-slide {
    from { background-position: 200% 0; }
    to { background-position: -200% 0; }
  }
</style>
