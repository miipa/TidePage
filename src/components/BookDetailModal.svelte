<script lang="ts">
  import { dateLocale, errText, locale, t } from "../lib/i18n/index.svelte";
  import { fmtDurationParts, formatCountShort } from "../lib/countFormat";
  import { onMount, onDestroy } from "svelte";
  import { emit } from "@tauri-apps/api/event";
  import type { TagInfo } from "../lib/api";
  import { api, coverUrl } from "../lib/api";

  import { COVER_FAIL_KEY } from "../lib/openError";
import { IconCopy, IconFolderOpen, IconX } from "@tabler/icons-svelte";
  import { appConfirm, copyText, toast } from "../lib/stores.svelte";
  import { outDissolve, outFade } from "../lib/motion";
  import type { BookRow, BookSession } from "../lib/types";

  let {
    book,
    onclose,
    onupdated,
  }: {
    book: BookRow;
    onclose: () => void;
    /** 保存成功后的回调：由父组件把新标题/作者写回结果集（不再就地改父对象的 book）。 */
    onupdated?: (title: string, author: string) => void;
  } = $props();

  let title = $state(book.title);
  let author = $state(book.author);
  let saving = $state(false);
  let sessions: BookSession[] = $state([]);
  let wc = $state<{ total: number; pure: number } | null>(null);
  let progress = $state<{ chapter_index: number; chapter_ratio: number; pct: number; updated_at: number } | null>(null);
  let coverTick = $state(0);
  /** 从文件名识别出的建议（只展示，不自动保存/写库）。 */
  let suggestion = $state<{ title: string; author: string } | null>(null);
  let allTags: TagInfo[] = $state([]);
  let bookTags: TagInfo[] = $state([]);
  let newTag = $state("");

  const cover = $derived(coverUrl(book) + (coverTick ? `&t=${coverTick}` : ""));

  // Esc 关闭：捕获阶段拦截，避免同一次 Esc 继续触发下层阅读器的返回/关闭快捷键。
  let keyHandler: ((e: KeyboardEvent) => void) | undefined;
  onMount(async () => {
    keyHandler = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      e.preventDefault();
      onclose();
    };
    window.addEventListener("keydown", keyHandler, true);
    try { sessions = await api.getBookSessions(book.id); } catch { /* ignore */ }
    try { wc = await api.bookWordCount(book.id); } catch { /* ignore */ }
    try { progress = await api.loadProgress(book.id); } catch { /* ignore */ }
    try { suggestion = await api.suggestMeta(book.path); } catch { /* 建议失败不影响详情框 */ }
    try {
      allTags = await api.listTags();
      bookTags = await api.listBookTags(book.id);
    } catch { /* 标签读取失败不影响详情框 */ }
  });

  async function saveBookTags(next: TagInfo[]) {
    try {
      await api.setBookTags(book.id, next.map((x) => x.id));
      bookTags = next;
      allTags = await api.listTags();
      await emit("library-tags-changed");
    } catch {
      toast(t("toast.tags.saveFailed"), "error");
    }
  }

  function toggleBookTag(tag: TagInfo) {
    const has = bookTags.some((x) => x.id === tag.id);
    void saveBookTags(has ? bookTags.filter((x) => x.id !== tag.id) : [...bookTags, tag]);
  }

  async function addNewTag() {
    const name = newTag.trim();
    if (!name) return;
    try {
      const tag = await api.createTag(name);
      newTag = "";
      allTags = await api.listTags();
      if (!bookTags.some((x) => x.id === tag.id)) {
        await saveBookTags([...bookTags, { ...tag, book_count: 0 }]);
      }
    } catch (e) {
      toast(errText(e), "error");
    }
  }
  onDestroy(() => {
    if (keyHandler) window.removeEventListener("keydown", keyHandler, true);
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
    // E14：整秒拆分，避免 3599.6s 显示成 1 小时 0 分（旧实现 m=60 与 s 并存）。
    const { h, m, s: ss } = fmtDurationParts(sec);
    if (h) return t("common.dur.hm", { h, m });
    if (m && ss) return t("common.dur.ms", { m, s: ss });
    if (m) return t("common.dur.mShort", { m });
    return t("common.dur.s", { sec: ss });
  }

  function fmtChars(n: number): string {
    // H22：中文「万」/英文「k」按 locale 走同一纯函数。
    const unit = t("common.unit.tenThousand");
    return formatCountShort(n, unit, unit, locale());
  }

  async function save() {
    // E15：仅标题必填；作者允许为空（未知作者是合法状态）。
    if (!title.trim()) {
      toast(t("bookDetail.nameRequired"), "error");
      return;
    }
    saving = true;
    try {
      await api.updateBookMeta(book.id, title.trim(), author.trim());
      // 不就地改父对象：交给父组件回调更新结果集（本地 title/author 已是新值，显示不受影响）。
      onupdated?.(title.trim(), author.trim());
      toast(t("bookDetail.savedInApp"));
    } catch (e) {
      toast(errText(e), "error");
    } finally {
      saving = false;
    }
  }

  let writing = $state(false);
  async function saveToFile() {
    // E15：写回文件同样只校验标题，作者可为空并原样写回。
    if (!title.trim()) {
      toast(t("bookDetail.nameRequired"), "error");
      return;
    }
    const ok = await appConfirm(
      t("bookDetail.writeBackConfirm", { path: book.path }),
    );
    if (!ok) return;
    writing = true;
    try {
      await api.writeBookMeta(book.id, title.trim(), author.trim());
      onupdated?.(title.trim(), author.trim());
      toast(t("bookDetail.written"));
    } catch (e) {
      toast(errText(e), "error");
    } finally {
      writing = false;
    }
  }

  /** 只把建议填进输入框：不保存、不写库、不改 book。 */
  function applySuggestion() {
    if (!suggestion) return;
    if (suggestion.title) title = suggestion.title;
    if (suggestion.author) author = suggestion.author;
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
      const name = book.format === "pdf" ? await (await import("../lib/pdf")).regeneratePdfCover(book.id, 320) : await api.reextractCover(book.id);
      book.cover_path = name;
      book.cover_ver = (book.cover_ver ?? 0) + 1;
      coverTick = Date.now();
      toast(t("toast.library.coverReextracted"));
    } catch {
      // R13-A4：封面失败统一短文案，不暴露底层异常。
      toast(t(COVER_FAIL_KEY), "error");
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

        {#if suggestion && (suggestion.title || suggestion.author)
            && (suggestion.title !== title || suggestion.author !== author)}
          <div class="bd-suggest" data-testid="bd-suggest">
            <span class="bd-suggest-label">{t("bookDetail.suggest.label")}</span>
            <span class="bd-suggest-value">
              {#if suggestion.title}《{suggestion.title}》{/if}
              {#if suggestion.title && suggestion.author} · {/if}
              {#if suggestion.author}{suggestion.author}{/if}
            </span>
            <button class="bd-suggest-apply" data-testid="bd-suggest-apply" onclick={applySuggestion}>{t("bookDetail.suggest.apply")}</button>
          </div>
        {/if}

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

        <div class="bd-field">
      <div class="bd-sec">{t("bookDetail.tags")}</div>
      <div class="bd-row" style="flex-wrap:wrap; gap:6px;">
        {#each allTags as tag (tag.id)}
          <button class="tag" class:active={bookTags.some((x) => x.id === tag.id)} onclick={() => toggleBookTag(tag)}>{tag.name}</button>
        {:else}
          <span class="tip">{t("bookDetail.noTags")}</span>
        {/each}
      </div>
      <div class="bd-row">
        <input placeholder={t("bookDetail.newTagPlaceholder")} bind:value={newTag} />
        <button class="bd-suggest-apply" onclick={addNewTag}>{t("bookDetail.addTag")}</button>
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

  .bd-suggest {
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin: 2px 0 10px;
    font-size: var(--fs-sm);
    color: var(--fg2);
    min-width: 0;
  }
  .bd-suggest-label { flex-shrink: 0; }
  .bd-suggest-value { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .bd-suggest-apply {
    flex-shrink: 0;
    padding: 0;
    border: none;
    background: none;
    color: var(--fg2);
    font-size: var(--fs-sm);
    text-decoration: underline;
    cursor: pointer;
  }
  .bd-suggest-apply:hover { color: var(--fg); }

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
