<script lang="ts">
  import { dateLocale, t } from "../lib/i18n/index.svelte";
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { IconClipboard, IconCopy, IconCut, IconTrash } from "@tabler/icons-svelte";
  import { api } from "../lib/api";
  import { config, saveConfig, toast, ui } from "../lib/stores.svelte";

  interface Entry { name: string; is_dir: boolean; size: number; mtime: number }
  interface DlItem { key: string; name: string; pct: number; state: "downloading" | "done" | "error"; msg: string }
  /** 剪贴板项：记录来源目录 srcDir 便于判断粘贴目标是否与来源相同。 */
  interface ClipItem { op: "copy" | "move"; name: string; path: string; is_dir: boolean; srcDir: string }

  const EXT_BOOK = ["txt", "epub", "mobi", "azw3", "prc"];

  let path = $state("");
  let entries = $state<Entry[]>([]);
  let loading = $state(false);
  let error = $state("");
  // 是否已完成至少一次列目录尝试（用于区分「首帧未加载」与「真·空目录」，避免闪现空态）。
  let attempted = $state(false);

  // 右键菜单 / 剪贴板式复制移动 / 面板内删除确认
  let browseEl: HTMLDivElement | undefined = $state();
  let ctxMenu = $state<{ x: number; y: number; entry: Entry | null } | null>(null);
  let ctxEl: HTMLDivElement | undefined = $state();
  let ctxPos = $state({ left: 0, top: 0 });
  let clipboard = $state<ClipItem | null>(null);
  let confirmDel = $state<Entry | null>(null);

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
  const sb0 = lsGet("tidepage.wd.sortBy", "time");
  const sd0 = lsGet("tidepage.wd.sortDir", "desc");
  let sortBy = $state<"name" | "time" | "size">(sb0 === "name" || sb0 === "size" ? sb0 : "time");
  let sortDir = $state<"asc" | "desc">(sd0 === "asc" ? "asc" : "desc");
  let foldersFirst = $state(lsGet("tidepage.wd.foldersFirst", "1") !== "0");

  // 任一排序状态变化即写回（首次运行写入默认值，无害）。
  $effect(() => {
    try {
      localStorage.setItem("tidepage.wd.sortBy", sortBy);
      localStorage.setItem("tidepage.wd.sortDir", sortDir);
      localStorage.setItem("tidepage.wd.foldersFirst", foldersFirst ? "1" : "0");
    } catch { /* ignore */ }
  });

  // 展示用的有序列表：先按「文件夹置顶」分组（与升降无关），再按所选字段+方向比较。
  const sortedEntries = $derived.by(() => {
    const list = [...entries];
    list.sort((a, b) => {
      if (foldersFirst && a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
      let cmp = 0;
      if (sortBy === "name") cmp = a.name.localeCompare(b.name, dateLocale());
      else if (sortBy === "time") cmp = (a.mtime || 0) - (b.mtime || 0);
      else cmp = (a.size || 0) - (b.size || 0);
      // 同值时用名称兜底，保证顺序稳定
      if (cmp === 0) cmp = a.name.localeCompare(b.name, dateLocale());
      return sortDir === "asc" ? cmp : -cmp;
    });
    return list;
  });

  function rememberedPath(): string {
    try { return localStorage.getItem("tidepage.wd.path") ?? ""; } catch { return ""; }
  }

  /// 列目录：成功返回 true。失败只置 error（保留 path、不回连接页），交由界面展示错误+重试。
  async function load(p?: string): Promise<boolean> {
    if (p !== undefined) {
      path = p;
      try { localStorage.setItem("tidepage.wd.path", p); } catch { /* ignore */ }
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
    const d = await api.pickFolder(t("webdav.pickDownloadDir"));
    if (!d) return;
    config.webdav.downloadDir = d;
    saveConfig();
    toast(t("webdav.downloadDirToast", { dir: d }));
  }

  function isBook(e: Entry) {
    const ext = e.name.split(".").pop()?.toLowerCase() || "";
    return EXT_BOOK.includes(ext);
  }

  /** 当前目录下某条目的完整远端路径。 */
  function fullPath(e: Entry): string {
    return path ? `${path}/${e.name}` : e.name;
  }

  /** 右键菜单：先按指针位置（相对面板）显示，渲染后按菜单尺寸夹取到面板内。
   *  注意：抽屉 .wd-drawer 带 transform 动画（fill both）会成为 fixed 的包含块，
   *  故菜单用面板内 absolute 定位 + 按面板尺寸夹取（而非视口 fixed 定位）。 */
  function openCtx(ev: MouseEvent, entry: Entry | null) {
    ev.preventDefault();
    const rect = browseEl?.getBoundingClientRect();
    const x = ev.clientX - (rect?.left ?? 0);
    const y = ev.clientY - (rect?.top ?? 0);
    ctxMenu = { x, y, entry };
    ctxPos = { left: x, top: y };
  }

  // 夹取到面板范围内（照 Library 的夹取思路，坐标改为相对面板）。
  $effect(() => {
    if (!ctxMenu || !ctxEl) return;
    const r = ctxEl.getBoundingClientRect();
    const pr = browseEl?.getBoundingClientRect();
    const w = pr?.width ?? window.innerWidth;
    const h = pr?.height ?? window.innerHeight;
    ctxPos = {
      left: Math.max(8, Math.min(ctxMenu.x, w - r.width - 8)),
      top: Math.max(8, Math.min(ctxMenu.y, h - r.height - 8)),
    };
  });

  // 菜单外点关闭（窗口捕获阶段监听，项目既有弹层范式）。
  $effect(() => {
    if (!ctxMenu) return;
    const h = (e: PointerEvent) => {
      if (ctxEl && !ctxEl.contains(e.target as Node)) ctxMenu = null;
    };
    window.addEventListener("pointerdown", h, true);
    return () => window.removeEventListener("pointerdown", h, true);
  });

  /** 列表空白处右键：仅有剪贴板内容时弹出「粘贴到此处」。 */
  function onListCtx(ev: MouseEvent) {
    const t = ev.target as HTMLElement | null;
    if (t?.closest?.(".wd-row")) return; // 行内右键由行自身处理
    ev.preventDefault();
    if (!clipboard) return;
    openCtx(ev, null);
  }

  /** 复制/移动到剪贴板（记录来源目录 srcDir，用于判断粘贴目标是否同源）。 */
  function setClip(op: "copy" | "move", e: Entry) {
    clipboard = { op, name: e.name, path: fullPath(e), is_dir: e.is_dir, srcDir: path };
    ctxMenu = null;
    toast(op === "copy" ? t("webdav.copiedToClipboard", { name: e.name }) : t("webdav.cutToClipboard", { name: e.name }));
  }

  /** 粘贴到当前目录：复制走 webdavCopy，移动走 webdavMove；失败保留剪贴板便于重试。 */
  async function paste() {
    ctxMenu = null;
    if (!clipboard) return;
    const clip = clipboard;
    if (path === clip.srcDir) {
      toast(t("webdav.sameDir"));
      return;
    }
    const dst = path ? `${path}/${clip.name}` : clip.name;
    try {
      if (clip.op === "copy") {
        await api.webdavCopy(config.webdav.base, config.webdav.user, config.webdav.pass, clip.path, dst);
      } else {
        await api.webdavMove(config.webdav.base, config.webdav.user, config.webdav.pass, clip.path, dst);
      }
      toast(clip.op === "copy" ? t("webdav.copied") : t("webdav.moved"));
      clipboard = null;
      await load();
    } catch (e) {
      const s = String(e);
      if (s.includes("412") || s.includes("409")) toast(t("webdav.nameConflict"), "error");
      else toast(s, "error");
    }
  }

  /** 确认删除当前条目，成功后刷新目录。 */
  async function doDelete() {
    const entry = confirmDel;
    if (!entry) return;
    confirmDel = null;
    try {
      await api.webdavDelete(config.webdav.base, config.webdav.user, config.webdav.pass, fullPath(entry));
      toast(t("common.toast.deleted"));
      await load();
    } catch (e) {
      toast(String(e), "error");
    }
  }

  async function download(e: Entry) {
    if (!config.webdav.downloadDir) {
      toast(t("webdav.pickDirFirst"), "error");
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
      toast(t("webdav.downloadFailed", { err: String(e2) }), "error");
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

<div class="wd-browse" bind:this={browseEl}>
  <div class="wd-head">
    <span class="wd-title">{t("library.topbar.webdav.label")}</span>
    <button class="icon" data-testid="wd-close" title={t("common.action.close")} aria-label={t("common.action.close")} onclick={() => (ui.webdavOpen = false)}>×</button>
  </div>

  {#if !configReady}
    <div class="wd-empty-cfg">
      <div class="wd-empty-ico">🔌</div>
      <p class="wd-empty-t">{t("webdav.notConfigured")}</p>
      <p class="wd-empty-d">{t("webdav.notConfiguredHint")}</p>
    </div>
  {:else}
    <!-- 工具行：导航 + 当前路径 + 排序控件（窄抽屉下自动换行，路径省略号不撑破） -->
    <div class="wd-tools">
      <button class="wd-btn" data-testid="wd-root" onclick={() => load("")} disabled={!path} title={t("webdav.rootDir")} aria-label={t("webdav.rootDir")}>⌂</button>
      <button class="wd-btn" data-testid="wd-up" onclick={up} disabled={!hasUp} title={t("webdav.parentDir")} aria-label={t("webdav.parentDir")}>↑</button>
      <button class="wd-btn" data-testid="wd-refresh" onclick={() => load()} title={t("webdav.refreshDir")} aria-label={t("common.action.refresh")}>⟳</button>
      {#if clipboard}
        <button class="wd-btn" data-testid="wd-paste" onclick={paste} title={t("webdav.pasteHereTip", { name: clipboard.name })} aria-label={t("webdav.pasteHereAria")}>{t("common.action.paste")}</button>
      {/if}
      <span class="wd-path" title={path || "/"}>{path || "/"}</span>
      <span class="wd-tools-gap"></span>
      <select class="wd-sel" data-testid="wd-sort" bind:value={sortBy} title={t("webdav.sortBy")} aria-label={t("webdav.sortBy")}>
        <option value="name">{t("webdav.sortName")}</option>
        <option value="time">{t("webdav.sortTime")}</option>
        <option value="size">{t("webdav.sortSize")}</option>
      </select>
      <button
        class="wd-btn"
        onclick={() => (sortDir = sortDir === "asc" ? "desc" : "asc")}
        data-testid="wd-sortdir"
        title={sortDir === "asc" ? t("webdav.ascTip") : t("webdav.descTip")}
        aria-label={t("webdav.toggleOrderAria")}
      >{sortDir === "asc" ? "↑" : "↓"}</button>
      <label class="wd-check" title={t("webdav.dirsFirstTip")}>
        <input type="checkbox" bind:checked={foldersFirst} /> {t("webdav.foldersFirst")}
      </label>
    </div>

    <div class="wd-info">
      <span class="wd-info-label">{t("webdav.downloadDirLabel")}</span>
      <span class="wd-info-val" title={config.webdav.downloadDir || t("common.notSet")}>{config.webdav.downloadDir || t("common.notSet")}</span>
      <button class="wd-btn" data-testid="wd-pickdir" onclick={pickDir}>{t("webdav.choose")}</button>
    </div>

    <div class="wd-list" oncontextmenu={onListCtx} role="presentation">
      {#if loading || (!attempted && !error)}
        <div class="wd-state"><span class="wd-spin">⟳</span> {t("common.loading")}</div>
      {:else if error}
        <div class="wd-state wd-state-error">
          <div class="wd-err-msg">{t("webdav.listFailed")}{error}</div>
          <button class="wd-btn wd-retry" data-testid="wd-retry" onclick={() => load()}>{t("webdav.retry")}</button>
        </div>
      {:else if sortedEntries.length === 0}
        <div class="wd-state">{t("webdav.emptyDir")}</div>
      {:else}
        {#each sortedEntries as e (path + "/" + e.name + String(e.size))}
          {#if e.is_dir}
            <button class="wd-row wd-dir" onclick={() => openEntry(e)} oncontextmenu={(ev) => openCtx(ev, e)}>
              <span class="wd-ico">📁</span>
              <span class="wd-name" title={e.name}>{e.name}</span>
            </button>
          {:else}
            <div class="wd-row" oncontextmenu={(ev) => openCtx(ev, e)} role="presentation">
              <span class="wd-ico">📄</span>
              <span class="wd-name" title={e.name}>{e.name}</span>
              <span class="wd-meta">{fmt(e.size)}{e.mtime ? " · " + fmtTime(e.mtime) : ""}</span>
              <button class="wd-dl-btn" onclick={() => download(e)} disabled={activeDownloads >= 3}>
                {isBook(e) ? t("webdav.downloadToLib") : t("webdav.download")}
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
                {d.state === "done" ? t("webdav.downloadDone") : d.state === "error" ? t("webdav.downloadFailedShort") : `${d.pct}%`}
              </span>
            </div>
            <div class="wd-bar"><div class="wd-bar-fill" class:fin={d.state === "done"} class:fail={d.state === "error"} style="width:{d.pct}%"></div></div>
            {#if d.state === "error"}<div class="wd-dl-err">{d.msg}</div>{/if}
          </div>
        {/each}
      </div>
    {/if}
  {/if}

  {#if ctxMenu}
    <div class="ctx" bind:this={ctxEl} style="left:{ctxPos.left}px; top:{ctxPos.top}px;">
      {#if ctxMenu.entry}
        <button data-testid="wd-ctx-copy" onclick={() => setClip("copy", ctxMenu!.entry!)}><span class="ci"><IconCopy size={17} stroke={1.75} /></span>{t("common.action.copy")}</button>
        <button data-testid="wd-ctx-move" onclick={() => setClip("move", ctxMenu!.entry!)}><span class="ci"><IconCut size={17} stroke={1.75} /></span>{t("webdav.move")}</button>
        <div class="ctx-sep"></div>
        <button class="danger" data-testid="wd-ctx-del" onclick={() => { confirmDel = ctxMenu!.entry!; ctxMenu = null; }}><span class="ci"><IconTrash size={17} stroke={1.75} /></span>{t("common.action.delete")}</button>
      {:else}
        <button data-testid="wd-ctx-paste" onclick={paste} disabled={!clipboard}><span class="ci"><IconClipboard size={17} stroke={1.75} /></span>{t("webdav.pasteHere")}</button>
      {/if}
    </div>
  {/if}

  {#if confirmDel}
    <!-- 删除确认：仅覆盖 WebDAV 面板区域（绝对定位在 .wd-browse 内） -->
    <div class="wd-confirm-wrap" role="dialog" aria-modal="true" aria-label={t("webdav.delConfirmAria")}>
      <button type="button" class="wd-confirm-mask" aria-label={t("webdav.cancelDelAria")} onclick={() => (confirmDel = null)}></button>
      <div class="wd-confirm">
        <p class="wd-confirm-t">{t("webdav.delConfirmTitle", { name: confirmDel.name })}</p>
        {#if confirmDel.is_dir}<p class="wd-confirm-s">{t("webdav.delWithContent")}</p>{/if}
        <p class="wd-confirm-s">{t("webdav.irreversible")}</p>
        <div class="wd-confirm-actions">
          <button class="wd-btn" data-testid="wd-del-cancel" onclick={() => (confirmDel = null)}>{t("common.confirm.cancel")}</button>
          <button class="wd-btn danger" data-testid="wd-del-ok" onclick={doDelete}>{t("common.action.delete")}</button>
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .wd-browse {
    position: relative; /* 右键菜单与删除确认均按面板内绝对定位 */
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

  /* ---------- 右键菜单（观感照 Library 的 .ctx） ---------- */
  .ctx {
    position: absolute;
    display: flex;
    flex-direction: column;
    gap: 1px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: var(--radius, 12px);
    box-shadow: var(--shadow-lg, 0 12px 32px rgba(0, 0, 0, 0.28));
    padding: 6px;
    min-width: 168px;
    z-index: 30;
    transform-origin: top left;
    animation: wd-ctx-in 0.18s ease;
  }

  @keyframes wd-ctx-in {
    from { opacity: 0; transform: translateY(-6px) scale(0.96); }
    to { opacity: 1; transform: none; }
  }

  .ctx button {
    border: none;
    background: transparent;
    text-align: left;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 12px 7px 10px;
    border-radius: var(--radius-sm, 8px);
    font-size: var(--fs-sm, 13px);
    color: var(--fg);
    cursor: pointer;
  }

  .ctx .ci {
    width: 18px;
    height: 18px;
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    opacity: 0.85;
  }

  .ctx-sep {
    height: 1px;
    margin: 5px 8px;
    background: var(--line);
  }

  .ctx button:hover { background: var(--accent-soft, var(--hover-bg)); }
  .ctx button:disabled { opacity: 0.42; cursor: default; }
  .ctx button.danger { color: var(--danger); }
  .ctx button.danger:hover { background: color-mix(in srgb, var(--danger) 12%, transparent); color: var(--danger); }

  /* ---------- 面板内删除确认（仅覆盖 webdav 面板区域） ---------- */
  .wd-confirm-wrap {
    position: absolute;
    inset: 0;
    z-index: 20;
    display: grid;
    place-items: center;
    padding: 16px;
  }

  /* 遮罩做成 <button> 以关闭且不引入 a11y 告警；用 !important 压过全局 button:hover 背景 */
  .wd-confirm-mask {
    position: absolute;
    inset: 0;
    margin: 0;
    padding: 0;
    border: none;
    background: rgba(0, 0, 0, 0.35) !important;
    cursor: default;
  }

  .wd-confirm {
    position: relative;
    z-index: 1;
    box-sizing: border-box;
    width: min(320px, 100%);
    background: var(--panel);
    color: var(--fg);
    border: 1px solid var(--line);
    border-radius: var(--radius, 12px);
    box-shadow: var(--shadow-lg, 0 12px 32px rgba(0, 0, 0, 0.28));
    padding: 18px;
  }

  .wd-confirm-t {
    margin: 0 0 8px;
    font-size: 14px;
    font-weight: 600;
    word-break: break-all;
  }

  .wd-confirm-s {
    margin: 0 0 4px;
    font-size: 12px;
    color: var(--fg2);
    line-height: 1.6;
  }

  .wd-confirm-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 14px;
  }

  .wd-btn.danger {
    color: var(--danger);
    border-color: color-mix(in srgb, var(--danger) 42%, var(--line));
  }

  .wd-btn.danger:hover:not(:disabled) {
    border-color: var(--danger);
    background: color-mix(in srgb, var(--danger) 12%, transparent);
  }
</style>
