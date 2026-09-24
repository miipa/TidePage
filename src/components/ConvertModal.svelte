<script lang="ts">
  import { untrack } from "svelte";
  import { api } from "../lib/api";
  import { config, convertState, saveConfig, toast } from "../lib/stores.svelte";

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
    const t = (title || book?.title || "book")
      .replace(/[\\/:*?"<>|]/g, "_")
      .replace(/\.(txt|epub|mobi|azw3|prc)$/i, "");
    return t || "book";
  }

  async function pickCover() {
    const p = await api.pickFile("选择封面图片", [
      { name: "图片", exts: ["png", "jpg", "jpeg", "webp", "gif"] },
    ]);
    if (p) coverPath = p;
  }

  async function run() {
    if (!book || busy || targetOptions.length === 0) return;
    const out = await api.pickSave(`保存 ${target.toUpperCase()} 文件`, target, `${baseName()}.${target}`);
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
      toast(`已转换：${out}`);
      api.rescan().catch(() => {});
      convertState.open = false;
      convertState.book = null;
    } catch (e) {
      toast(String(e), "error");
    } finally {
      busy = false;
    }
  }
</script>

<div class="modal-body conv">
  <div class="conv-head">
    <h3>格式转换</h3>
    <button class="icon" title="关闭" onclick={close}>×</button>
  </div>
  {#if book}
    <p class="src">源文件：{book.title} · {book.format.toUpperCase()}</p>

    {#if targetOptions.length > 1}
      <div class="row-inline" style="margin-bottom:12px">
        <span class="lbl">目标格式</span>
        {#each targetOptions as t (t)}
          <button class="pill" class:active={target === t} onclick={() => (target = t)}>{t.toUpperCase()}</button>
        {/each}
      </div>
    {/if}

    {#if showMeta}
      <div class="grid2">
        <div class="field">
          <label>书名</label>
          <input placeholder="默认取自文件名" bind:value={title} />
        </div>
        <div class="field">
          <label>作者</label>
          <input placeholder="留空则不设置" bind:value={author} title="自动从文件或正文提取" />
        </div>
      </div>
      <div class="field">
        <label>封面</label>
        <div class="row-inline">
          <button onclick={pickCover}>{coverPath ? "更换封面图片…" : "选择封面图片…"}</button>
          <button
            onclick={() => { coverPath = config.reader.conv?.coverPath || ""; }}
            disabled={!config.reader.conv?.coverPath}
            title="使用上次转换的封面"
          >↺ 上次封面</button>
          {#if coverPath}
            <span class="covername" title={coverPath}>{coverPath.split(/[\\/]/).pop()}</span>
            <button class="icon" title="清除" onclick={() => (coverPath = "")}>×</button>
          {/if}
        </div>
        <p class="tip">封面会自动记忆上次选择。</p>
      </div>
      <div class="field">
        <label>EPUB 内嵌 CSS</label>
        <textarea rows="8" bind:value={css} spellcheck="false" placeholder={"body { font-family: 'Songti SC', serif; }\np { line-height: 2; text-indent: 2em; }"}></textarea>
        <p class="tip">自动记忆上次输入，与阅读样式无关。</p>
      </div>
    {:else}
      <p class="src">将去除样式，仅保留纯文本与章节标题。</p>
    {/if}
  {/if}

  <div class="confirm-row">
    <button onclick={close}>取消</button>
    <button class="primary" onclick={run} disabled={busy || !book}>{busy ? "转换中…" : "选择保存路径并转换"}</button>
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
    font-size: 12px;
    color: var(--fg2);
  }

  .pill {
    padding: 4px 14px;
    border-radius: 16px;
    font-size: 12px;
  }

  .pill.active {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }

  .src {
    font-size: 12px;
    color: var(--fg2);
    margin: 0 0 12px;
    word-break: break-all;
  }

  .tip {
    font-size: 12px;
    color: var(--fg2);
    line-height: 1.7;
    margin: 4px 0 0;
  }

  .covername {
    font-size: 12px;
    color: var(--fg2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 260px;
  }

  textarea {
    font-family: Consolas, monospace;
    font-size: 13px;
    resize: vertical;
  }

  .confirm-row {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 6px;
  }
</style>