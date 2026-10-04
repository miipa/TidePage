<script lang="ts">
  import { errText, t } from "../lib/i18n/index.svelte";
import { IconRestore, IconX } from "@tabler/icons-svelte";
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
    // 局部变量改名 bn：避免与 i18n 的 t() 同名遮蔽
    const bn = (title || book?.title || "book")
      .replace(/[\\/:*?"<>|]/g, "_")
      .replace(/\.(txt|epub|mobi|azw3|prc)$/i, "");
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
        <!-- 有轨道的分段控件：结构与 app.css 的 .fr-seg 同构（轨道 --panel2 + 段内 2px 内边距
             + 2px 段间距 + 药丸圆角），选中段从轨道里抬起（--shadow-sm）。标签留在轨道外面 ——
             标签不是分段控件的组成部分。 -->
        <div class="pill-track">
          {#each targetOptions as fmt (fmt)}
            <button class="pill" class:active={target === fmt} onclick={() => (target = fmt)}>{fmt.toUpperCase()}</button>
          {/each}
        </div>
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

/* flex: 0 0 auto —— 全局 .row-inline > * { flex: 1 } 会把剩余空间分给每一个直接子元素。
     目标格式这一排改成轨道容器后，标签成了唯一吃剩余空间的 flex 项，盒宽涨到 191px、
     把轨道推到 x≈222（胶囊原本从 x≈101.5 开始），中间空出约 120px。收紧成内容宽后，
     标签贴住文字、控件紧随其后（第一颗段约落在 x≈83，比改动前左移约 19px —— 这是对的：
     表单排版里标签与控件本来就该贴在一起）。
     本文件里 .lbl 只有目标格式这一处用法，收紧的影响面就是这一行。 */
  .lbl {
    flex: 0 0 auto;
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    color: var(--fg2);
  }

  /* 轨道容器：这一族「有轨道的分段控件」的容器，配方与 app.css 的 .fr-seg 相同
     （--panel2 底 + 1px --line 边 + 2px 内边距 + 2px 段间距 + 药丸圆角）。
     为什么要轨道：中性色板只有三级表面，没有第四级能表达「从轨道里抬起」——
     没有轨道时，选中段与容器的色差只有约 1.1:1（浅色 #ffffff → #f2f1ec、
     深色 #1e2126 → #252932），识别信号几乎全压在 600 字重上。有了轨道 + 抬起投影，
     「层级」才回来，而不必把填充加深（加深就变重了）。
     ⚠️ 反过来也成立：**没有轨道的容器上不能用「选中比容器更亮一档」这条关系** ——
     直接坐在 dialog(--bg-elev) 上的胶囊曾因此让选中态与底色同色、填充感消失，
     那是本文件 .pill 注释里记着的坑，不要重复它。

     flex: 0 0 auto —— 全局 .row-inline > * { flex: 1; min-width: 0 } 会把轨道拉满整行宽度，
     那样这一排会变成等分满宽、胶囊跟着一起撑开。显式收回内容宽。

     结构对齐 app.css 的 .fr-seg（本项目内这个模式的先例）：同样的轨道配方
     （inline-flex + 2px gap + 2px padding + 1px 边框 + 圆角 + 底色），同样的段高档
     var(--ctl-h-sm)，轨道高度因此由「padding + border + 段高」自然涌现 = 32px，
     与设置里明暗分段控件的总高自动一致。

     ⚠️ 之前这里是「钉死 height: var(--ctl-h)(30px) + 段 min-height: 0 由 stretch 拉伸」，
        靠一段高度补偿注释撑着，与 .fr-seg 差 2px —— 同一类控件在同一应用里两个总高，
        读起来像没做完。结构同源之后不需要任何补偿：没有钉死的高度，也就无从漂移。
        代价是这一排从 30px 变成 32px（+2px），换取两处结构同源、以后不会再次漂移。 */
  .pill-track {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: stretch;
    gap: 2px;
    padding: 2px;
    border: 1px solid var(--line);
    border-radius: var(--radius-pill);
    background: var(--panel2);
  }

  /* 段自身：未选中透明、无边（底色与边框都由轨道提供，这也是 .fr-seg button 的做法）。
     min-height 与 .fr-seg button 同为 var(--ctl-h-sm)：全局 button 带
     min-height: var(--ctl-h)(30px)，若沿用它会比轨道还高、把段顶出轨道；
     归零或另钉一个数都是「各自钉死」，同构到 --ctl-h-sm 才是结构对齐。
     圆角保持 --radius-xl 不动 —— 它在 --ctl-h-sm 高的段上已被视口夹成药丸形，作为「段」的圆角足够圆。 */
  .pill {
    padding: 4px 14px;
    border-radius: var(--radius-xl);
    font-size: var(--fs-xs);
    letter-spacing: var(--ls-xs);
    min-height: var(--ctl-h-sm);
    border: none;
    background: transparent;
  }

  /* 选中段：从轨道里抬起。投影是这里唯一允许的静态表面投影（物理上抬起的段，
     同 .fr-seg 选中段与书库 .card:hover 的抬升）。内环保留，深色主题下多一重信号。
     ⚠️ 方向提醒见上面 .pill-track 的注释：这套「轨道 + 抬起」不能搬到没有轨道的容器上。 */
  .pill.active {
    background: var(--bg-elev);
    color: var(--fg);
    box-shadow: var(--shadow-sm), var(--edge-hi);
    font-weight: 600;
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