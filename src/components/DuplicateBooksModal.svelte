<script lang="ts">
  /** D7：重复书检测结果与删除（只走现有 deleteBooks 回收站路径）。 */
  import { onMount, onDestroy } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { api, type DuplicateBook, type DuplicateGroup } from "../lib/api";
  import { appConfirm, toast } from "../lib/stores.svelte";
  import { errText, t } from "../lib/i18n/index.svelte";
  import {
    clearGroupSelection,
    collectGroupDirs,
    confidenceText,
    filterGroupsByDir,
    parentDirOf,
    pruneDeleted,
    representativeForDir,
    restoreIgnoredGroups,
    selectedForDeletion,
    sortDuplicateGroups,
    suggestKeepIndex,
    visibleDuplicateGroups,
  } from "../lib/duplicates";
  import { IconX } from "@tabler/icons-svelte";

  let { onclose }: { onclose: () => void } = $props();

  let scanning = $state(false);
  let progress = $state<{ done: number; total: number; groups: number; skipped: number } | null>(null);
  let groups: DuplicateGroup[] = $state([]);
  let skipped = $state(0);
  let elapsed = $state(0);
  let query = $state("");
  let page = $state(0);
  let ignored: string[] = $state([]);
  let selection = $state<Record<number, boolean>>({});
  let expanded = $state<Set<string>>(new Set());
  let dirFilter = $state("");
  let unlisten: UnlistenFn | null = null;
  const PAGE_SIZE = 20;
  const OPT_KEY = "tp.dupes.scanOptions";
  let scanOptions = $state<{ includeSuspected: boolean; titleThreshold: number; sizeTolerancePct: number; maxCandidates: number }>((() => {
    try {
      return { includeSuspected: false, titleThreshold: 0.82, sizeTolerancePct: 0.10, maxCandidates: 500, ...JSON.parse(localStorage.getItem(OPT_KEY) || "{}") };
    } catch { return { includeSuspected: false, titleThreshold: 0.82, sizeTolerancePct: 0.10, maxCandidates: 500 }; }
  })());
  function saveScanOptions() {
    try { localStorage.setItem(OPT_KEY, JSON.stringify(scanOptions)); } catch { /* ignore */ }
  }

  const searchedGroups = $derived(
    groups.filter(
      (g) =>
        !query.trim() ||
        g.books.some((b) =>
          (b.title + " " + b.author + " " + b.path).toLowerCase().includes(query.trim().toLowerCase()),
        ),
    ),
  );
  // 目录 chips 随搜索收敛（搜索后、目录筛选前）；选中目录后不因再次筛选而消失。
  const dirs = $derived(collectGroupDirs(searchedGroups));
  const dirGroups = $derived(filterGroupsByDir(searchedGroups, dirFilter));
  const shown = $derived(visibleDuplicateGroups(dirGroups, ignored, page, PAGE_SIZE));
  const filteredCount = $derived(dirGroups.filter((g) => !ignored.includes(g.key)).length);
  const selectedBooks = $derived(selectedForDeletion(dirGroups, ignored, selection));
  /** 全选范围：当前目录筛选下所有未忽略组的可见副本（跨分页）。 */
  const selectableBooks = $derived(dirGroups.filter((g) => !ignored.includes(g.key)).flatMap((g) => g.books));
  const allSelected = $derived(selectableBooks.length > 0 && selectableBooks.every((b) => selection[b.id] === true));

  function applyGroups(next: DuplicateGroup[]) {
    groups = sortDuplicateGroups(next);
    selection = {}; // 扫描/打开/删除后默认零选中，绝不自动勾选任何书
    expanded = new Set();
    dirFilter = dirFilter && collectGroupDirs(groups).includes(dirFilter) ? dirFilter : "";
    page = 0;
  }

  async function scan() {
    if (scanning) return;
    scanning = true;
    groups = [];
    skipped = 0;
    page = 0;
    progress = null;
    try {
      const r = await api.findDuplicateBooks(scanOptions);
      if (r.cancelled) {
        toast(t("dupes.cancelled"));
        return;
      }
      applyGroups(r.groups);
      skipped = r.skipped;
      elapsed = r.elapsedMs;
      if (!r.groups.length) toast(t("dupes.empty"));
    } catch (e) {
      toast(errText(e), "error");
    } finally {
      scanning = false;
    }
  }

  async function cancel() {
    try { await api.cancelDuplicateScan(); } catch { /* ignore */ }
  }

  function toggle(id: number) {
    selection = { ...selection, [id]: !selection[id] };
  }

  function toggleExpand(key: string) {
    const next = new Set(expanded);
    if (next.has(key)) next.delete(key); else next.add(key);
    expanded = next;
  }

  function groupDirBooks(g: DuplicateGroup, dir: string): DuplicateBook[] {
    return g.books.filter((b) => parentDirOf(b.path) === dir);
  }

  function dirAllSelected(g: DuplicateGroup, dir: string): boolean {
    const list = groupDirBooks(g, dir);
    return list.length > 0 && list.every((b) => selection[b.id] === true);
  }

  function toggleDir(g: DuplicateGroup, dir: string) {
    const list = groupDirBooks(g, dir);
    const on = !dirAllSelected(g, dir);
    const next = { ...selection };
    for (const b of list) next[b.id] = on;
    selection = next;
  }

  function toggleAll() {
    const on = !allSelected;
    const next = { ...selection };
    for (const b of selectableBooks) next[b.id] = on;
    selection = next;
  }

  function groupRepresentative(g: DuplicateGroup): DuplicateBook | null {
    return representativeForDir(g.books, dirFilter);
  }

  function groupDirs(g: DuplicateGroup): string[] {
    return collectGroupDirs([g]);
  }

  function groupDirCount(g: DuplicateGroup): number {
    return groupDirs(g).length;
  }

  function ignoreGroup(key: string) {
    const g = groups.find((x) => x.key === key);
    if (g) selection = clearGroupSelection(selection, g.books);
    ignored = [...ignored, key];
  }

  function restoreIgnored() {
    const r = restoreIgnoredGroups(groups, ignored, selection);
    ignored = r.ignored;
    selection = r.selection;
  }

  function fmtSize(n: number): string {
    if (n >= 1048576) return (n / 1048576).toFixed(1) + " MB";
    if (n >= 1024) return (n / 1024).toFixed(0) + " KB";
    return n + " B";
  }

  function fmtTime(ms: number): string {
    if (!ms) return "—";
    try { return new Date(ms).toLocaleDateString(); } catch { return "—"; }
  }

  /** A：仅从书库移除（文件保留、路径进 ignoring/忽略表）。 */
  async function removeSelected() {
    if (!selectedBooks.length || scanning) return;
    const ok = await appConfirm(t("dupes.removeConfirm", { n: selectedBooks.length }));
    if (!ok) return;
    const ids = new Set(selectedBooks.map((b) => b.id));
    try {
      await api.deleteBooks(selectedBooks.map((b) => b.path), false, true);
      applyGroups(pruneDeleted(groups, ids));
      toast(t("dupes.removed", { n: ids.size }));
    } catch (e) {
      toast(errText(e), "error");
    }
  }

  /** B：移入回收站（文件进回收站）。 */
  async function deleteSelected() {
    if (!selectedBooks.length || scanning) return;
    const suspectedSelected = dirGroups.some((g) => g.suspected && g.books.some((b) => selection[b.id]));
    const ok = await appConfirm(t(suspectedSelected ? "dupes.deleteConfirmSuspected" : "dupes.deleteConfirm", { n: selectedBooks.length }));
    if (!ok) return;
    const ids = new Set(selectedBooks.map((b) => b.id));
    try {
      await api.deleteBooks(selectedBooks.map((b) => b.path), true, true);
      applyGroups(pruneDeleted(groups, ids));
      toast(t("dupes.deleted", { n: ids.size }));
    } catch (e) {
      toast(errText(e), "error");
    }
  }

  onMount(() => {
    void listen<{ running: boolean; phase: string; done: number; total: number; groups: number; skipped: number }>(
      "duplicate-progress",
      (e) => {
        if (e.payload.phase === "scan") {
          progress = { done: e.payload.done, total: e.payload.total, groups: e.payload.groups, skipped: e.payload.skipped };
        }
      },
    ).then((u) => (unlisten = u));
    void scan();
  });

  onDestroy(() => {
    unlisten?.();
    // 扫描中关窗：取消后台扫描，避免 DUP_SCAN_RUNNING 残留到下一轮。
    if (scanning) void api.cancelDuplicateScan().catch(() => undefined);
  });
</script>

<div class="dupes-backdrop" onclick={(e) => { if (e.target === e.currentTarget) onclose(); }}>
  <div class="dupes-panel" role="dialog" aria-label={t("dupes.title")}>
    <div class="dp-head">
      <span>{t("dupes.title")}</span>
      <div style="display:flex; gap:6px; align-items:center;">
        <button class="icon" title={t("common.action.close")} onclick={onclose}><IconX /></button>
      </div>
    </div>
    <div class="dp-bar">
      <input placeholder={t("dupes.search")} bind:value={query} />
      <button onclick={() => void scan()} disabled={scanning}>{scanning ? t("dupes.scanning") : t("dupes.rescan")}</button>
      {#if scanning}<button onclick={() => void cancel()}>{t("common.confirm.cancel")}</button>{/if}
      {#if ignored.length}<button onclick={restoreIgnored}>{t("dupes.showIgnored", { n: ignored.length })}</button>{/if}
      <span class="dp-stat">
        {#if scanning && progress}
          {t("dupes.progress", { done: progress.done, total: progress.total, groups: progress.groups, skipped: progress.skipped })}
        {:else if !scanning}
          {t("dupes.summary", { groups: filteredCount, skipped, ms: elapsed })}
        {/if}
      </span>
    </div>
    <div class="dp-bar" style="gap:10px; flex-wrap:wrap">
      <label><input type="checkbox" bind:checked={scanOptions.includeSuspected} onchange={saveScanOptions} /> {t("dupes.includeSuspected")}</label>
      {#if scanOptions.includeSuspected}
        <span class="tip">{t("dupes.suspectedWarning")}</span>
        <label>{t("dupes.titleThreshold")} <input type="number" min="0.5" max="1" step="0.05" style="width:64px" bind:value={scanOptions.titleThreshold} onchange={saveScanOptions} /></label>
        <label>{t("dupes.sizeTolerance")} <input type="number" min="0" max="50" step="1" style="width:56px" value={Math.round(scanOptions.sizeTolerancePct * 100)} oninput={(e) => { scanOptions.sizeTolerancePct = Math.max(0, Math.min(50, Number((e.currentTarget as HTMLInputElement).value) || 0)) / 100; saveScanOptions(); }} /></label>
      {/if}
    </div>
    <div class="dp-bar dirs">
      <span class="dp-stat">{t("dupes.dirFilter")}</span>
      <button class:active={!dirFilter} onclick={() => { dirFilter = ""; page = 0; }}>{t("dupes.allDirs")}</button>
      {#each dirs as d (d)}
        <button class:active={dirFilter === d} title={d} onclick={() => { dirFilter = d; page = 0; }}>{d}</button>
      {/each}
    </div>
    <div class="dp-body">
      {#if scanning && !groups.length}
        <div class="dp-empty">{t("dupes.scanning")}</div>
      {:else if !filteredCount}
        <div class="dp-empty">{t("dupes.empty")}</div>
      {:else}
        {#each shown as g (g.key)}
          {@const rep = groupRepresentative(g)}
          <section class="dp-group">
            <div class="dp-group-head">
              <span>
                {#if g.suspected}
                  <span class="badge">{t("dupes.suspected")} · {t(("dupes.reason." + g.reason) as any)} · {confidenceText(g.confidence)}</span>{" "}
                {:else}<span class="badge">{t("dupes.exact")}</span>{" "}{/if}
                {t("dupes.copies", { n: g.books.length, dirs: groupDirCount(g) })}
              </span>
              <span style="display:flex; gap:6px;">
                <button onclick={() => toggleExpand(g.key)}>{expanded.has(g.key) ? t("dupes.collapse") : t("dupes.expand")}</button>
                <button onclick={() => ignoreGroup(g.key)}>{t("dupes.ignore")}</button>
              </span>
            </div>
            {#if rep}
              <div class="dp-row dp-rep">
                <span class="dp-idx">{t("dupes.suggestKeep")}</span>
                <span class="dp-main">
                  <b>{rep.title}</b>
                  {#if rep.author}<span class="dp-author">{rep.author}</span>{/if}
                  <span class="dp-meta">{rep.format.toUpperCase()} · {fmtSize(rep.size)} · {fmtTime(rep.addedAt)}</span>
                  <span class="dp-path" title={rep.path}>{rep.path}</span>
                </span>
                <span class="dp-read">{rep.readPct ? Math.round(rep.readPct) + "%" : "—"}</span>
              </div>
            {/if}
            {#if expanded.has(g.key)}
              {#each groupDirs(g) as d (d)}
                <div class="dp-dir-head">
                  <label>
                    <input type="checkbox" checked={dirAllSelected(g, d)} onchange={() => toggleDir(g, d)} />
                    {t("dupes.selectDir")}
                  </label>
                  <span class="dp-path" title={d}>{d}</span>
                </div>
                {#each groupDirBooks(g, d) as b (b.id)}
                  <label class="dp-row">
                    <input type="checkbox" checked={selection[b.id] === true} onchange={() => toggle(b.id)} />
                    <span class="dp-idx"></span>
                    <span class="dp-main">
                      <b>{b.title}</b>
                      {#if b.author}<span class="dp-author">{b.author}</span>{/if}
                      <span class="dp-meta">{b.format.toUpperCase()} · {fmtSize(b.size)} · {fmtTime(b.addedAt)}</span>
                      <span class="dp-path" title={b.path}>{b.path}</span>
                    </span>
                    <span class="dp-read">{b.readPct ? Math.round(b.readPct) + "%" : "—"}</span>
                  </label>
                {/each}
              {/each}
            {/if}
          </section>
        {/each}
        {#if filteredCount > PAGE_SIZE}
          <div class="dp-pager">
            <button onclick={() => (page = Math.max(0, page - 1))} disabled={page === 0}>←</button>
            <span>{page + 1} / {Math.ceil(filteredCount / PAGE_SIZE)}</span>
            <button onclick={() => (page = Math.min(Math.ceil(filteredCount / PAGE_SIZE) - 1, page + 1))}>→</button>
          </div>
        {/if}
      {/if}
    </div>
    <div class="dp-foot">
      <label><input type="checkbox" checked={allSelected} onchange={toggleAll} /> {t("dupes.selectAllCopies")}</label>
      <span>{t("dupes.selectedCopies", { n: selectedBooks.length })}</span>
      <button onclick={() => void removeSelected()} disabled={!selectedBooks.length || scanning}>{t("dupes.removeFromLibrary")}</button>
      <button class="danger" onclick={() => void deleteSelected()} disabled={!selectedBooks.length || scanning}>{t("dupes.moveToTrash")}</button>
    </div>
  </div>
</div>

<style>
  .dupes-backdrop { position: fixed; inset: 0; background: #0006; display: grid; place-items: center; z-index: var(--z-modal, 1000); }
  .dupes-panel { width: min(920px, 92vw); height: min(720px, 86vh); background: var(--bg, #fff); color: var(--fg, #222); border-radius: 10px; display: flex; flex-direction: column; overflow: hidden; }
  .dp-head { display: flex; justify-content: space-between; align-items: center; padding: 10px 14px; border-bottom: 1px solid var(--border, #8884); font-weight: 600; }
  .dp-bar { display: flex; gap: 8px; align-items: center; padding: 8px 14px; border-bottom: 1px solid var(--border, #8884); flex-wrap: wrap; }
  .dp-bar input { flex: 1; min-width: 180px; }
  .dp-stat { font-size: 12px; opacity: 0.75; }
  .dp-body { flex: 1; overflow: auto; padding: 8px 14px; }
  .dp-empty { padding: 40px 0; text-align: center; opacity: 0.7; }
  .dp-group { border: 1px solid var(--border, #8884); border-radius: 8px; margin: 10px 0; }
  .dp-group-head { display: flex; justify-content: space-between; align-items: center; padding: 8px 10px; background: var(--hover-bg, #8881); font-size: 13px; }
  .dp-group-head button { font-size: 12px; }
  .dp-row { display: flex; gap: 8px; align-items: flex-start; padding: 8px 10px; border-top: 1px solid var(--border, #8882); cursor: pointer; }
  .dp-idx { color: var(--accent); font-size: 12px; min-width: 70px; }
  .dp-main { display: flex; flex-direction: column; gap: 2px; flex: 1; min-width: 0; }
  .dp-author, .dp-meta { font-size: 12px; opacity: 0.7; }
  .dp-path { font-size: 11px; opacity: 0.55; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dp-read { font-size: 12px; opacity: 0.75; }
  .dp-pager { display: flex; gap: 10px; justify-content: center; align-items: center; padding: 8px; }
  .dp-bar.dirs { gap: 6px; overflow-x: auto; flex-wrap: nowrap; }
  .dp-bar.dirs button { max-width: 260px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; }
  .dp-bar.dirs button.active { border-color: var(--accent); color: var(--accent); }
  .dp-rep { cursor: default; }
  .dp-dir-head { display: flex; gap: 8px; align-items: center; padding: 6px 10px; border-top: 1px solid var(--border, #8882); font-size: 12px; background: var(--hover-bg, #8881); }
  .dp-dir-head label { display: flex; gap: 6px; align-items: center; flex: none; }
  .dp-dir-head .dp-path { flex: 1; min-width: 0; }
  .dp-foot { display: flex; justify-content: space-between; align-items: center; gap: 10px; flex-wrap: wrap; padding: 10px 14px; border-top: 1px solid var(--border, #8884); }
</style>
