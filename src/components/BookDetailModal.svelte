<script lang="ts">
  import { dateLocale, errText, t } from "../lib/i18n/index.svelte";
  import { onMount } from "svelte";
  import { api, coverUrl } from "../lib/api";
  import { regeneratePdfCover } from "../lib/pdf";
import { IconCopy, IconFolderOpen, IconX } from "@tabler/icons-svelte";
  import { appConfirm, copyText, toast } from "../lib/stores.svelte";
  import { outDissolve, outFade } from "../lib/motion";
  import type { BookRow, BookSession } from "../lib/types";

  let { book, onclose }: { book: BookRow; onclose: () => void } = $props();

  let title = $state(book.title);
  let author = $state(book.author);
  let saving = $state(false);
  let sessions: BookSession[] = $state([]);
  let wc = $state<{ total: number; pure: number } | null>(null);
  let progress = $state<{ chapter_index: number; chapter_ratio: number; pct: number; updated_at: number } | null>(null);
  let coverTick = $state(0);

  const cover = $derived(coverUrl(book) + (coverTick ? `&t=${coverTick}` : ""));

  onMount(async () => {
    try { sessions = await api.getBookSessions(book.id); } catch { /* ignore */ }
    try { wc = await api.bookWordCount(book.id); } catch { /* ignore */ }
    try { progress = await api.loadProgress(book.id); } catch { /* ignore */ }
  });

  const totalSeconds = $derived(sessions.reduce((a, s) => a + s.seconds, 0));
  const totalChars = $derived(sessions.reduce((a, s) => a + s.chars, 0));

  function fmtSize(n: number): string {
    if (n > 1048576) return (n / 1048576).toFixed(2) + " MB";
    if (n > 1024) return (n / 1024).toFixed(0) + " KB";
    return n + " B";
  }

  function fmtTime(ms: number): string {
    if (!ms) return "—";
    return new Date(ms).toLocaleString(dateLocale());
  }

  function fmtDur(sec: number): string {
    const h = Math.floor(sec / 3600);
    const m = Math.round((sec % 3600) / 60);
    if (h) return t("common.dur.hm", { h, m });
    if (m) return t("common.dur.mShort", { m });
    return t("common.dur.s", { sec });
  }

  function fmtChars(n: number): string {
    if (n >= 10000) return (n / 10000).toFixed(1) + t("common.unit.tenThousand");
    return String(n);
  }

  async function save() {
    saving = true;
    try {
      await api.updateBookMeta(book.id, title.trim(), author.trim());
      book.title = title.trim();
      book.author = author.trim();
      toast(t("bookDetail.savedInApp"));
    } catch (e) {
      toast(errText(e), "error");
    } finally {
      saving = false;
    }
  }

  let writing = $state(false);
  async function saveToFile() {
    const ok = await appConfirm(
      t("bookDetail.writeBackConfirm", { path: book.path }),
    );
    if (!ok) return;
    writing = true;
    try {
      await api.writeBookMeta(book.id, title.trim(), author.trim());
      book.title = title.trim();
      book.author = author.trim();
      toast(t("bookDetail.written"));
    } catch (e) {
      toast(errText(e), "error");
    } finally {
      writing = false;
    }
  }

  function infoText(): string {
    return [
      t("bookDetail.copy.title", { title }),
      t("bookDetail.copy.author", { author: author || "—" }),
      t("bookDetail.copy.format", { format: book.format.toUpperCase() }),
      t("bookDetail.copy.size", { size: fmtSize(book.size) }),
      t("bookDetail.copy.path", { path: book.path }),
      t("bookDetail.copy.added", { time: fmtTime(book.added_at) }),
      t("bookDetail.copy.lastRead", { time: fmtTime(book.last_read) }),
      wc ? t("bookDetail.copy.totalChars", { total: wc.total, pure: wc.pure }) : "",
      t("bookDetail.copy.readTotal", { duration: fmtDur(totalSeconds), chars: fmtChars(totalChars) }),
    ].filter(Boolean).join("\n");
  }

  function copyAll() {
    copyText(infoText()).then(() => toast(t("bookDetail.copied")));
  }

  async function setCover() {
    const p = await api.pickFile(t("library.dialog.pickCover"), [
      { name: t("library.dialog.imageFilter"), exts: ["png", "jpg", "jpeg", "webp", "bmp"] },
    ]);
    if (!p) return;
    try {
      await api.setCustomCover(book.id, p);
      book.cover_path = `custom-${book.id}.png`;
      book.cover_ver = (book.cover_ver ?? 0) + 1;
      coverTick = Date.now();
      toast(t("toast.library.coverUpdated"));
    } catch (e) {
      toast(errText(e), "error");
    }
  }

  async function reextract() {
    try {
      const name = book.format === "pdf" ? await regeneratePdfCover(book.id, 320) : await api.reextractCover(book.id);
      book.cover_path = name;
      book.cover_ver = (book.cover_ver ?? 0) + 1;
      coverTick = Date.now();
      toast(t("toast.library.coverReextracted"));
    } catch (e) {
      toast(errText(e), "error");
    }
  }
</script>

<div class="bd-overlay" out:outDissolve onpointerdown={(e) => { if (e.target === e.currentTarget) onclose(); }}>
  <div class="bd-panel" out:outFade={{ from: 0.97, up: false }} role="dialog" aria-label={t("bookDetail.title")}>
    <div class="bd-head">
      <span>{t("bookDetail.title")}</span>
      <button class="icon" data-testid="bd-close" title={t("common.action.close")} onclick={onclose}><IconX /></button>
    </div>

    <div class="bd-body">
      <div class="bd-left">
        <img class="bd-cover" src={cover} alt={t("library.book.coverAlt", { title: book.title })} />
        <div class="bd-cover-acts">
          <button data-testid="bd-set-cover" onclick={setCover}>{t("library.ctx.setCover")}</button>
          {#if book.format !== "pdf"}
            <button data-testid="bd-reextract" onclick={reextract}>{t("bookDetail.reextract")}</button>
          {/if}
        </div>
      </div>

      <div class="bd-right">
        <div class="bd-field">
          <label>{t("bookDetail.field.title")}</label>
          <div class="bd-row">
            <input data-testid="bd-title" bind:value={title} />
            <button class="icon" data-testid="bd-copy-title" title={t("bookDetail.copyTitle")} onclick={() => copyText(title).then(() => toast(t("common.toast.copied")))}><IconCopy /></button>
          </div>
        </div>
        <div class="bd-field">
          <label>{t("bookDetail.field.author")}</label>
          <div class="bd-row">
            <input data-testid="bd-author" bind:value={author} placeholder={t("common.unknown")} />
            <button class="icon" data-testid="bd-copy-author" title={t("bookDetail.copyAuthor")} onclick={() => copyText(author).then(() => toast(t("common.toast.copied")))}><IconCopy /></button>
          </div>
        </div>

        <div class="bd-grid">
          <div class="bd-kv"><span>{t("bookDetail.field.format")}</span><b>{book.format.toUpperCase()}</b></div>
          <div class="bd-kv"><span>{t("bookDetail.field.size")}</span><b>{fmtSize(book.size)}</b></div>
          <div class="bd-kv"><span>{t("bookDetail.field.added")}</span><b>{fmtTime(book.added_at)}</b></div>
          <div class="bd-kv"><span>{t("bookDetail.field.lastRead")}</span><b>{fmtTime(book.last_read)}</b></div>
          {#if wc}
            <div class="bd-kv"><span>{t("bookDetail.field.totalChars")}</span><b>{fmtChars(wc.total)}</b></div>
            <div class="bd-kv"><span>{t("bookDetail.field.pureChars")}</span><b>{fmtChars(wc.pure)}</b></div>
          {/if}
        </div>

        <div class="bd-field">
          <label>{t("bookDetail.field.path")}</label>
          <div class="bd-row">
            <input class="path" data-testid="bd-path" value={book.path} readonly />
            <button class="icon" data-testid="bd-copy-path" title={t("bookDetail.copyPath")} onclick={() => copyText(book.path).then(() => toast(t("bookDetail.pathCopied")))}><IconCopy /></button>
            <button class="icon" data-testid="bd-reveal" title={t("bookDetail.reveal")} onclick={() => api.revealInExplorer(book.path)}><IconFolderOpen /></button>
          </div>
        </div>

        <div class="bd-sec">{t("bookDetail.history")}</div>
        <div class="bd-summary">
          <div class="bd-s"><b>{fmtDur(totalSeconds)}</b><span>{t("bookDetail.stat.totalTime")}</span></div>
          <div class="bd-s"><b>{fmtChars(totalChars)}</b><span>{t("bookDetail.stat.readChars")}</span></div>
          <div class="bd-s"><b>{progress ? progress.pct.toFixed(1) + "%" : "—"}</b><span>{t("bookDetail.stat.progress")}</span></div>
          <div class="bd-s"><b>{sessions.length}</b><span>{t("bookDetail.stat.times")}</span></div>
        </div>
        <div class="bd-sessions">
          {#each sessions.slice(0, 20) as s (s.started_at)}
            <div class="bd-srow">
              <span class="bd-day">{s.day}</span>
              <span>{new Date(s.started_at).toLocaleTimeString(dateLocale(), { hour: "2-digit", minute: "2-digit" })}</span>
              <span>{fmtDur(s.seconds)}</span>
              <span>{fmtChars(s.chars)} {t("common.unit.chars")}</span>
            </div>
          {:else}
            <div class="bd-empty">{t("bookDetail.historyEmpty")}</div>
          {/each}
        </div>
      </div>
    </div>

    <div class="bd-foot">
      <button data-testid="bd-copy-all" onclick={copyAll}>{t("bookDetail.copyAll")}</button>
      <div class="flex1"></div>
      <button data-testid="bd-cancel" onclick={onclose}>{t("common.action.close")}</button>
      <button data-testid="bd-save" onclick={save} disabled={saving}>{saving ? t("common.saving") : t("bookDetail.saveInfo")}</button>
      <button class="primary" data-testid="bd-save-file" onclick={saveToFile} disabled={writing} title={t("bookDetail.writeBackTip")}>
        {writing ? t("bookDetail.writing") : t("bookDetail.saveToBook")}
      </button>
    </div>
  </div>
</div>

<style>
  .bd-overlay {
    position: fixed;
    inset: 0;
    background: var(--scrim-strong);
    backdrop-filter: blur(3px) saturate(1.08);
    -webkit-backdrop-filter: blur(3px) saturate(1.08);
    z-index: var(--z-modal);
    display: grid;
    place-items: center;
    padding: 24px;
    animation: tp-fade-in var(--dur-scrim) var(--ease-in) both;
  }

  .bd-panel {
    width: min(820px, 94vw);
    max-height: 90vh;
    background: var(--panel);
    color: var(--fg);
    border: 1px solid var(--line);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-lg), var(--edge-hi);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: tp-dialog-in var(--dur-modal) var(--ease-premium) both;
  }

  .bd-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 18px;
    font-weight: 600;
    border-bottom: 1px solid var(--line);
  }

  .bd-body {
    display: flex;
    gap: 20px;
    padding: 18px;
    overflow-y: auto;
  }

  .bd-left { width: 180px; flex-shrink: 0; }

  .bd-cover {
    width: 100%;
    aspect-ratio: 2 / 3;
    object-fit: cover;
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-md), inset 0 1px 0 rgba(255, 255, 255, 0.14);
    background: var(--panel2);
  }

  .bd-cover-acts {
    display: flex;
    gap: 6px;
    margin-top: 10px;
  }

  .bd-cover-acts button { flex: 1; font-size: var(--fs-xs); letter-spacing: var(--ls-xs); padding: 5px 0; }

  .bd-right { flex: 1; min-width: 0; }

  .bd-field { margin-bottom: 10px; }

  .bd-field label {
    display: block;
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    color: var(--fg2);
    margin-bottom: 4px;
  }

  .bd-row { display: flex; gap: 6px; align-items: center; }
  .bd-row input { flex: 1; min-width: 0; }
  .bd-row input.path { font-size: var(--fs-xs); letter-spacing: var(--ls-xs); color: var(--fg2); }

  .bd-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px 16px;
    margin: 12px 0;
  }

  .bd-kv { display: flex; justify-content: space-between; font-size: var(--fs-sm); letter-spacing: var(--ls-sm); }
  .bd-kv span { color: var(--fg2); }
  .bd-kv b { font-weight: 500; }

  .bd-sec {
    font-weight: 600;
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
    margin: 14px 0 8px;
    padding-bottom: 4px;
    border-bottom: 1px solid var(--line);
  }

  .bd-summary {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 8px;
    margin-bottom: 10px;
  }

  .bd-s {
    background: var(--panel2);
    border-radius: var(--radius-md);
    padding: 8px 6px;
    text-align: center;
  }

  .bd-s b { display: block; font-size: var(--fs-md); color: var(--accent); }
  .bd-s span { font-size: var(--fs-xs); letter-spacing: var(--ls-xs); color: var(--fg2); }

  .bd-sessions { max-height: 190px; overflow-y: auto; }

  .bd-srow {
    display: flex;
    gap: 12px;
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
    padding: 5px 2px;
    border-bottom: 1px solid var(--line);
  }

  .bd-day { width: 92px; color: var(--fg2); }
  .bd-empty { padding: 18px; text-align: center; color: var(--fg2); font-size: var(--fs-sm); letter-spacing: var(--ls-sm); }

  .bd-foot {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 18px;
    border-top: 1px solid var(--line);
  }

  .flex1 { flex: 1; }
</style>
