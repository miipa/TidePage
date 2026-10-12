<script lang="ts">
  import { errText, t } from "../lib/i18n/index.svelte";
import { IconRestore, IconX } from "@tabler/icons-svelte";
  import { untrack } from "svelte";
  import { api } from "../lib/api";
  import { config, convertState, saveConfig, toast } from "../lib/stores.svelte";
  import { stripBookExt } from "../lib/bookExt";

  let coverPath = $state("");
  let author = $state("");
  let title = $state("");
  let css = $state("");
  let busy = $state(false);
  let suggested = $state(false);
  let target = $state<"txt" | "epub">("epub");

  const book = $derived(convertState.book);

  const targetOptions = $derived.by(() => {
    if (!book) return [] as ("txt" | "epub")[];
    if (book.format === "txt") return ["epub"] as ("txt" | "epub")[];
    return ["txt", "epub"] as ("txt" | "epub")[];
  });

  const showMeta = $derived(target === "epub");

  $effect(() => {
    const b = convertState.book;
    if (!convertState.open || !b) return;
    // 必须 untrack：autoFill() 会写 suggested 等本组件 $state，
    // 若在 effect 的追踪作用域内执行会触发“写自己读的 state”死循环，
    // 导致 effect_update_depth_exceeded，整个界面更新冻结
    untrack(() => {
      css = config.reader.conv?.css || "";
      coverPath = "";
      author = "";
      title = b.title;
      target = b.format === "txt" ? "epub" : "txt";
      busy = false;
      suggested = false;
      autoFill(b);
    });
  });

  async function autoFill(b = convertState.book) {
    if (!b || suggested) return;
    suggested = true;
    try {
      const m = await api.suggestMeta(b.path);
      if (m.title && !title) title = m.title;
      author = m.author || "";
    } catch { /* ignore */ }
  }

  function close() {
    convertState.open = false;
    convertState.book = null;
  }

  function baseName(): string {
    // 局部变量改名 bn：避免与 i18n 的 t() 同名遮蔽
    const bn = stripBookExt(
      (title || book?.title || "book").replace(/[\\/:*?"<>|]/g, "_"),
    );
    return bn || "book";
  }

  async function pickCover() {
    const p = await api.pickFile(t("library.dialog.pickCover"), [
      { name: t("library.dialog.imageFilter"), exts: ["png", "jpg", "jpeg", "webp", "gif"] },
    ]);
    if (p) coverPath = p;
  }

  async function run() {
    if (!book || busy || targetOptions.length === 0) return;
    const out = await api.pickSave(t("convert.saveFile", { ext: target.toUpperCase() }), target, `${baseName()}.${target}`);
    if (!out) return;
    busy = true;
    try {
      await api.convertBook(book.path, target, out, {
        title: showMeta && title.trim() ? title.trim() : undefined,
        author: showMeta && author.trim() ? author.trim() : undefined,
        coverPath: showMeta ? coverPath || undefined : undefined,
        css: showMeta ? css.trim() || undefined : undefined,
        pattern: config.regex || undefined,
      });
      config.reader.conv = {
        ...(config.reader.conv || { title: "", author: "", css: "", coverPath: "" }),
        css: css,
        coverPath: coverPath,
      };
      saveConfig();
      toast(t("convert.done", { out }));
      api.rescan().catch(() => {});
      convertState.open = false;
      convertState.book = null;
    } catch (e) {
      toast(errText(e), "error");
    } finally {
      busy = false;
    }
  }
</script>

<div class="modal-body conv">
  <div class="conv-head">
    <h3>{t("convert.title")}</h3>
    <button class="icon" data-testid="conv-close" title={t("common.action.close")} onclick={close}><IconX /></button>
  </div>
  {#if book}
    <p class="src">{t("convert.sourceFile")} {book.title} · {book.format.toUpperCase()}</p>

    {#if targetOptions.length > 1}
      <div class="row-inline" style="margin-bottom:12px">
        <span class="lbl">{t("convert.targetFormat")}</span>
        {#each targetOptions as fmt (fmt)}
          <button class="pill" class:active={target === fmt} onclick={() => (target = fmt)}>{fmt.toUpperCase()}</button>
        {/each}
      </div>
    {/if}

    {#if showMeta}
      <div class="grid2">
        <div class="field">
          <label>{t("bookDetail.field.title")}</label>
          <input data-testid="conv-title" placeholder={t("convert.titleHint")} bind:value={title} />
        </div>
        <div class="field">
          <label>{t("bookDetail.field.author")}</label>
          <input data-testid="conv-author" placeholder={t("convert.authorHint")} bind:value={author} title={t("convert.authorAutoHint")} />
        </div>
      </div>
      <div class="field">
        <label>{t("convert.cover")}</label>
        <div class="row-inline">
          <button data-testid="conv-pick-cover" onclick={pickCover}>{coverPath ? t("convert.changeCover") : t("convert.pickCover")}</button>
          <button
            class="ico"
            onclick={() => { coverPath = config.reader.conv?.coverPath || ""; }}
            disabled={!config.reader.conv?.coverPath}
            data-testid="conv-last-cover"
            title={t("convert.reuseCoverTip")}
          ><IconRestore />{t("convert.lastCover")}</button>
          {#if coverPath}
            <span class="covername" title={coverPath}>{coverPath.split(/[\\/]/).pop()}</span>
            <button class="icon" data-testid="conv-clear-cover" title={t("common.action.clear")} onclick={() => (coverPath = "")}><IconX /></button>
          {/if}
        </div>
        <p class="tip">{t("convert.coverAutoRemember")}</p>
      </div>
      <div class="field">
        <label>{t("convert.embeddedCss")}</label>
        <textarea rows="8" bind:value={css} spellcheck="false" placeholder={"body { font-family: 'Songti SC', serif; }\np { line-height: 2; text-indent: 2em; }"}></textarea>
        <p class="tip">{t("convert.cssAutoRemember")}</p>
      </div>
    {:else}
      <p class="src">{t("convert.stripStyleHint")}</p>
    {/if}
  {/if}

  <div class="confirm-row">
    <button data-testid="conv-cancel" onclick={close}>{t("common.confirm.cancel")}</button>
    <button class="primary" data-testid="conv-run" onclick={run} disabled={busy || !book}>{busy ? t("convert.working") : t("convert.pickAndRun")}</button>
  </div>
</div>

<style>
  .conv-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 6px;
  }

  .conv-head h3 {
    margin: 0;
  }

  .lbl {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    color: var(--fg2);
  }

  .pill {
    padding: 4px 14px;
    border-radius: var(--radius-xl);
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
  }

  .pill.active {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }

  .src {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    color: var(--fg2);
    margin: 0 0 12px;
    word-break: break-all;
  }

  .covername {
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    color: var(--fg2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 260px;
  }

  textarea {
    font-family: Consolas, monospace;
    font-size: var(--fs-sm);
    letter-spacing: var(--ls-sm);
    resize: vertical;
  }

  .confirm-row {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 6px;
  }
</style>