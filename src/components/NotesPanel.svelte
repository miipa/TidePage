<!-- 笔记界面（G3）：按书分组列出「有感想(note!='')」的划线；样式同主书库网格。
     书名优先级：books 实时名 → orphan 快照名 → 空（显示「已删除」）。 -->
<script lang="ts">
  import { api } from "../lib/api";
  import { toast, ui, requestOpenBook } from "../lib/stores.svelte";
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";

  type NoteBook = { bookId: number; title: string; author: string; deleted: boolean; noteCount: number; lastAt: number };
  type NoteItem = { id: number; chapterIndex: number; chapterTitle: string; text: string; note: string; createdAt: number };

  let books: NoteBook[] = $state([]);
  let loading = $state(true);
  let detail: NoteBook | null = $state(null);
  let detailNotes: NoteItem[] = $state([]);
  let detailLoading = $state(false);

  async function load() {
    loading = true;
    try {
      books = await api.listNoteBooks();
    } catch (e) {
      toast(String(e), "error");
    } finally {
      loading = false;
    }
  }

  async function openDetail(b: NoteBook) {
    detail = b;
    detailLoading = true;
    try {
      detailNotes = await api.listNotesOfBook(b.bookId);
    } catch (e) {
      toast(String(e), "error");
    } finally {
      detailLoading = false;
    }
  }

  function openBook(b: NoteBook) {
    if (b.deleted) {
      toast("原书已删除，仅保留笔记", "error");
      return;
    }
    ui.notesOpen = false;
    requestOpenBook(b.bookId);
  }

  async function removeNote(n: NoteItem) {
    try {
      await api.setHighlightNote(n.id, "");
      detailNotes = detailNotes.filter((x) => x.id !== n.id);
      await load();
    } catch (e) {
      toast(String(e), "error");
    }
  }

  function fmtDate(ms: number): string {
    if (!ms) return "";
    return new Date(ms).toLocaleDateString("zh-CN");
  }

  /** 封面：书在 → 共享默认封面端点（叠实时名）；书已删（孤儿 id<0 或无书）→ 用书名文字兜底。 */
  function noteCover(b: NoteBook): string {
    if (b.bookId < 0 || b.deleted) return ""; // 交由 <img> 的 onerror 显示文字兜底
    return `http://flowread.localhost/cover-default/${b.bookId}`;
  }

  onMount(() => {
    load();
    const un = listen("notes-changed", () => load());
    return () => un.then((f) => f());
  });
</script>

<div class="np">
  <header class="np-head">
    <div class="np-title">笔记 <span class="np-count">{books.length}</span></div>
    <button class="icon" title="刷新" onclick={load}>⟳</button>
    <button class="icon" title="关闭" onclick={() => (ui.notesOpen = false)}>×</button>
  </header>

  {#if loading}
    <div class="np-empty">加载中…</div>
  {:else if !books.length}
    <div class="np-empty">
      <div class="np-empty-t">还没有笔记</div>
      <div class="np-empty-d">阅读时划线并「添加感想」，这里会按书汇总。</div>
    </div>
  {:else}
    <div class="np-grid">
      {#each books as b (b.bookId)}
        <button class="np-card" onclick={() => openDetail(b)} title={b.title}>
          <span class="np-cover">
            {#if noteCover(b)}
              <img src={noteCover(b)} alt="《{b.title || "已删除"}》封面" loading="lazy" decoding="async" />
            {:else}
              <span class="np-cover-fallback">
                <span class="np-cf-t">{b.title || "已删除"}</span>
                {#if b.author}<span class="np-cf-a">{b.author}</span>{/if}
              </span>
            {/if}
            {#if b.deleted}<span class="np-del">已删除</span>{/if}
            <span class="np-badge">{b.noteCount}</span>
          </span>
          <span class="np-meta">
            <span class="np-t">{b.title || "已删除"}</span>
            <span class="np-a">{b.author || (b.deleted ? "" : "未知作者")}</span>
          </span>
        </button>
      {/each}
    </div>
  {/if}

  {#if detail}
    <div class="np-drawer-backdrop" onclick={() => (detail = null)}></div>
    <aside class="np-drawer" role="dialog" aria-label="笔记明细">
      <div class="np-drawer-head">
        <div class="np-dh-main">
          <div class="np-dh-title">{detail.title || "已删除"}</div>
          {#if detail.deleted}<div class="np-dh-del">原书已删除 · 笔记保留</div>{/if}
        </div>
        <button class="icon" title="关闭" onclick={() => (detail = null)}>×</button>
      </div>
      {#if !detail.deleted}
        <div class="np-drawer-acts">
          <button onclick={() => detail && openBook(detail)}>打开原书</button>
        </div>
      {/if}
      <div class="np-list">
        {#if detailLoading}
          <div class="np-empty">加载中…</div>
        {:else}
          {#each detailNotes as n (n.id)}
            <div class="np-item">
              <div class="np-quote">{n.text}</div>
              <div class="np-note">{n.note}</div>
              <div class="np-foot">
                <span class="np-ch">{n.chapterTitle || `第 ${n.chapterIndex + 1} 节`} · {fmtDate(n.createdAt)}</span>
                <button class="icon" title="删除这条感想（保留划线）" onclick={() => removeNote(n)}>🗑</button>
              </div>
            </div>
          {/each}
        {/if}
      </div>
    </aside>
  {/if}
</div>

<style>
  .np {
    position: fixed;
    inset: 0;
    z-index: 1090;
    background: var(--bg);
    color: var(--fg);
    display: flex;
    flex-direction: column;
  }

  .np-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 16px;
    border-bottom: 1px solid var(--line);
    background: var(--panel);
    flex-shrink: 0;
  }

  .np-title { font-size: 16px; font-weight: 600; }
  .np-count {
    display: inline-block;
    margin-left: 6px;
    font-size: 12px;
    color: var(--accent);
    background: var(--accent-soft, var(--hover-bg));
    border-radius: 999px;
    padding: 1px 8px;
    font-weight: 500;
  }
  .np-head .icon[title="刷新"] { margin-left: auto; }

  .np-empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    color: var(--fg2);
  }
  .np-empty-t { font-size: 16px; font-weight: 600; color: var(--fg); }
  .np-empty-d { font-size: 13px; }

  /* 网格与主书库一致的语言 */
  .np-grid {
    flex: 1;
    overflow-y: auto;
    padding: 16px 20px 32px;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 16px;
    align-content: start;
  }

  .np-card {
    border: none;
    background: transparent;
    box-shadow: none;
    padding: 8px;
    border-radius: var(--radius, 12px);
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 8px;
    text-align: left;
    transition: background var(--dur, 0.2s) var(--ease), transform 0.2s cubic-bezier(0.4, 0, 0.2, 1);
  }
  .np-card:hover:not(:disabled) {
    background: var(--panel2);
    border-color: transparent;
    transform: translateY(-4px);
  }

  .np-cover {
    position: relative;
    display: block;
    width: 100%;
    aspect-ratio: 2 / 3;
    border-radius: var(--radius-sm, 8px);
    overflow: hidden;
    outline: 1px solid var(--line);
    outline-offset: -1px;
    box-shadow: var(--shadow-sm);
    background: var(--panel2);
  }
  .np-cover img { width: 100%; height: 100%; object-fit: cover; display: block; }

  /* 书已删除：无封面，用简洁文字兜底（仍显示原书名，不丢信息） */
  .np-cover-fallback {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 12px;
    text-align: center;
    background: linear-gradient(160deg, var(--panel2), color-mix(in srgb, var(--panel2) 70%, #000 8%));
  }
  .np-cf-t { font-size: 14px; font-weight: 600; line-height: 1.35; color: var(--fg); }
  .np-cf-a { font-size: 12px; color: var(--fg2); }

  .np-del {
    position: absolute;
    left: 6px;
    top: 6px;
    background: color-mix(in srgb, var(--danger) 86%, transparent);
    color: #fff;
    font-size: 11px;
    padding: 2px 7px;
    border-radius: 999px;
    backdrop-filter: blur(4px);
  }

  .np-badge {
    position: absolute;
    right: 6px;
    top: 6px;
    min-width: 22px;
    height: 22px;
    padding: 0 6px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 88%, transparent);
    color: var(--accent-fg, #fff);
    font-size: 12px;
    font-weight: 600;
    display: grid;
    place-items: center;
  }

  .np-meta { padding: 0 4px; display: flex; flex-direction: column; gap: 2px; }
  .np-t { font-size: 14px; font-weight: 500; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .np-a { font-size: 12px; color: var(--fg2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

  /* 明细抽屉 */
  .np-drawer-backdrop { position: fixed; inset: 0; background: rgba(0, 0, 0, 0.35); z-index: 1090; }
  .np-drawer {
    position: fixed;
    top: 0;
    right: 0;
    bottom: 0;
    width: min(440px, 92vw);
    z-index: 1091;
    background: var(--bg-elev);
    border-left: 1px solid var(--line);
    box-shadow: -10px 0 32px rgba(0, 0, 0, 0.25);
    display: flex;
    flex-direction: column;
  }
  .np-drawer-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 14px 16px;
    border-bottom: 1px solid var(--line);
  }
  .np-dh-main { flex: 1; min-width: 0; }
  .np-dh-title { font-size: 15px; font-weight: 600; }
  .np-dh-del { font-size: 12px; color: var(--danger); margin-top: 2px; }
  .np-drawer-acts { padding: 10px 16px; border-bottom: 1px solid var(--line); }
  .np-list { flex: 1; overflow-y: auto; padding: 8px 12px; }

  .np-item {
    border: 1px solid var(--line);
    border-radius: var(--radius-md, 12px);
    padding: 12px;
    margin-bottom: 10px;
    background: var(--panel);
  }
  .np-quote {
    font-size: 13px;
    color: var(--fg2);
    border-left: 3px solid var(--line-strong);
    padding-left: 8px;
    margin-bottom: 8px;
    line-height: 1.6;
  }
  .np-note { font-size: 14px; line-height: 1.65; white-space: pre-wrap; }
  .np-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-top: 8px;
  }
  .np-ch { font-size: 12px; color: var(--fg2); }
</style>
