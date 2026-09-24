<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api } from "../lib/api";
  import { config, saveConfig, toast, ui } from "../lib/stores.svelte";

  interface Entry { name: string; is_dir: boolean; size: number; mtime: number }
  interface DlItem { key: string; name: string; pct: number; state: "downloading" | "done" | "error"; msg: string }

  const EXT_BOOK = ["txt", "epub", "mobi", "azw3", "prc"];

  let connected = $state(false);
  let path = $state("");
  let entries = $state<Entry[]>([]);
  let loading = $state(false);
  let error = $state("");

  const downloads = $state<DlItem[]>([]);
  let dlUnlisteners: Record<string, () => void> = {};
  let dlSeq = 0;

  function rememberedPath(): string {
    try { return localStorage.getItem("flowread.wd.path") ?? ""; } catch { return ""; }
  }

  async function connect() {
    if (!config.webdav.base || !config.webdav.user || !config.webdav.pass) {
      toast("请先在 设置→WebDAV 填写服务器并启用", "error");
      return;
    }
    connected = true;
    const p = rememberedPath();
    await load(p);
  }

  async function load(p?: string) {
    if (p !== undefined) {
      path = p;
      try { localStorage.setItem("flowread.wd.path", p); } catch { /* ignore */ }
    }
    loading = true;
    error = "";
    try {
      const w = config.webdav;
      const items = await api.webdavList(w.base, w.user, w.pass, path);
      entries = items
        .filter((e) => e.name !== "" && e.name !== "." && e.name !== "..")
        .sort((a, b) => {
          if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
          return (b.mtime || 0) - (a.mtime || 0) || a.name.localeCompare(b.name, "zh");
        });
    } catch (e) {
      error = String(e);
      entries = [];
    } finally {
      loading = false;
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
</script>

<div class="wd-browse">
  <div class="wd-head">
    <span class="wd-title">WebDAV 文件管理</span>
    <button class="icon" title="关闭" onclick={() => (ui.webdavOpen = false)}>×</button>
  </div>

  {#if !connected}
    <div class="wd-connect">
      <p>连接 {config.webdav.base || "未配置服务器"}<br />
      账号：{config.webdav.user || "—"}</p>
      <button class="primary big" onclick={connect} disabled={!config.webdav.base}>
        🔗 连接并访问
      </button>
      <p class="tip">WebDAV 服务器与账号密码在 设置→WebDAV 里填写。</p>
    </div>
  {:else}
    <div class="row-inline" style="margin-bottom:8px">
      <button onclick={() => load("")} disabled={!path}>根目录</button>
      <button onclick={up} disabled={!hasUp}>↑ 上级</button>
      <button onclick={() => load()} title="刷新当前目录">⟳</button>
      <span class="wd-path">{path || "/"}</span>
    </div>
    <div class="wd-info">
      <span>下载目录：{config.webdav.downloadDir || "未设置"}</span>
      <button onclick={pickDir}>选择…</button>
    </div>
    {#if loading}<p class="tip">加载中…</p>{/if}
    {#if error}<p class="tip" style="color:var(--danger)">{error}</p>{/if}
<div class="wd-list">
      {#each entries as e (path + "/" + e.name + String(e.size))}
        {#if e.is_dir}
          <div class="wd-row" onclick={() => openEntry(e)}>
            <span class="wd-name">📁 {e.name}</span>
          </div>
        {:else}
          <div class="wd-row">
            <span class="wd-name">📄 {e.name} <span class="wd-size">{fmt(e.size)}{e.mtime ? " · " + fmtTime(e.mtime) : ""}</span></span>
            <button onclick={() => download(e)} disabled={downloads.length >= 3}>{isBook(e) ? "下载入书库" : "下载"}</button>
          </div>
        {/if}
      {:else}
        {#if !loading && !error}<p class="tip">空目录或无权限</p>{/if}
      {/each}
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

  .wd-connect {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 14px;
    text-align: center;
  }

  .wd-connect p {
    margin: 0;
    font-size: 13px;
    word-break: break-all;
  }

  .wd-connect .big {
    padding: 10px 26px;
    font-size: 16px;
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

  .wd-path {
    font-family: Consolas, monospace;
    font-size: 12px;
    opacity: 0.75;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    text-align: right;
  }

  .wd-info {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    font-size: 12px;
    margin-bottom: 10px;
    opacity: 0.9;
  }

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
    gap: 10px;
    padding: 6px 8px;
    border-radius: 6px;
    cursor: pointer;
  }

  .wd-row + .wd-row {
    border-top: 1px solid rgba(128, 128, 128, 0.12);
  }

  .wd-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .wd-size {
    opacity: 0.5;
    font-size: 12px;
    margin-left: 6px;
  }

  .wd-info button {
    padding: 3px 10px;
    font-size: 12px;
  }

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