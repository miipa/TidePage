<script lang="ts">
  import { onMount } from "svelte";
  import { api, coverUrl } from "../lib/api";
  import { copyText, toast } from "../lib/stores.svelte";
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
    return new Date(ms).toLocaleString("zh-CN");
  }

  function fmtDur(sec: number): string {
    const h = Math.floor(sec / 3600);
    const m = Math.round((sec % 3600) / 60);
    if (h) return `${h} 小时 ${m} 分`;
    if (m) return `${m} 分`;
    return `${sec} 秒`;
  }

  function fmtChars(n: number): string {
    if (n >= 10000) return (n / 10000).toFixed(1) + "万";
    return String(n);
  }

  async function save() {
    saving = true;
    try {
      await api.updateBookMeta(book.id, title.trim(), author.trim());
      book.title = title.trim();
      book.author = author.trim();
      toast("书籍信息已保存");
    } catch (e) {
      toast(String(e), "error");
    } finally {
      saving = false;
    }
  }

  function infoText(): string {
    return [
      `书名：${title}`,
      `作者：${author || "—"}`,
      `格式：${book.format.toUpperCase()}`,
      `大小：${fmtSize(book.size)}`,
      `路径：${book.path}`,
      `添加时间：${fmtTime(book.added_at)}`,
      `最近阅读：${fmtTime(book.last_read)}`,
      wc ? `全书字数：${wc.total}（纯文字 ${wc.pure}）` : "",
      `累计阅读：${fmtDur(totalSeconds)} · ${fmtChars(totalChars)} 字`,
    ].filter(Boolean).join("\n");
  }

  function copyAll() {
    copyText(infoText()).then(() => toast("书籍信息已复制"));
  }

  async function setCover() {
    const p = await api.pickFile("选择封面图片", [
      { name: "图片", exts: ["png", "jpg", "jpeg", "webp", "bmp"] },
    ]);
    if (!p) return;
    try {
      await api.setCustomCover(book.id, p);
      book.cover_path = `custom-${book.id}.png`;
      coverTick = Date.now();
      toast("封面已更新");
    } catch (e) {
      toast(String(e), "error");
    }
  }

  async function reextract() {
    try {
      const name = await api.reextractCover(book.id);
      book.cover_path = name;
      coverTick = Date.now();
      toast("封面已重新提取");
    } catch (e) {
      toast(String(e), "error");
    }
  }
</script>

<div class="bd-overlay" onpointerdown={(e) => { if (e.target === e.currentTarget) onclose(); }}>
  <div class="bd-panel" role="dialog" aria-label="书籍详细信息">
    <div class="bd-head">
      <span>书籍详细信息</span>
      <button class="icon" title="关闭" onclick={onclose}>×</button>
    </div>

    <div class="bd-body">
      <div class="bd-left">
        <img class="bd-cover" src={cover} alt="" />
        <div class="bd-cover-acts">
          <button onclick={setCover}>设置封面</button>
          <button onclick={reextract}>重新提取</button>
        </div>
      </div>

      <div class="bd-right">
        <div class="bd-field">
          <label>书名</label>
          <div class="bd-row">
            <input bind:value={title} />
            <button class="icon" title="复制书名" onclick={() => copyText(title).then(() => toast("已复制"))}>⧉</button>
          </div>
        </div>
        <div class="bd-field">
          <label>作者</label>
          <div class="bd-row">
            <input bind:value={author} placeholder="未知" />
            <button class="icon" title="复制作者" onclick={() => copyText(author).then(() => toast("已复制"))}>⧉</button>
          </div>
        </div>

        <div class="bd-grid">
          <div class="bd-kv"><span>格式</span><b>{book.format.toUpperCase()}</b></div>
          <div class="bd-kv"><span>大小</span><b>{fmtSize(book.size)}</b></div>
          <div class="bd-kv"><span>添加时间</span><b>{fmtTime(book.added_at)}</b></div>
          <div class="bd-kv"><span>最近阅读</span><b>{fmtTime(book.last_read)}</b></div>
          {#if wc}
            <div class="bd-kv"><span>全书字数</span><b>{fmtChars(wc.total)}</b></div>
            <div class="bd-kv"><span>纯文字</span><b>{fmtChars(wc.pure)}</b></div>
          {/if}
        </div>

        <div class="bd-field">
          <label>路径</label>
          <div class="bd-row">
            <input class="path" value={book.path} readonly />
            <button class="icon" title="复制路径" onclick={() => copyText(book.path).then(() => toast("已复制路径"))}>⧉</button>
            <button class="icon" title="在文件夹中显示" onclick={() => api.revealInExplorer(book.path)}>📂</button>
          </div>
        </div>

        <div class="bd-sec">阅读记录</div>
        <div class="bd-summary">
          <div class="bd-s"><b>{fmtDur(totalSeconds)}</b><span>累计时长</span></div>
          <div class="bd-s"><b>{fmtChars(totalChars)}</b><span>已读字数</span></div>
          <div class="bd-s"><b>{progress ? progress.pct.toFixed(1) + "%" : "—"}</b><span>阅读进度</span></div>
          <div class="bd-s"><b>{sessions.length}</b><span>阅读次数</span></div>
        </div>
        <div class="bd-sessions">
          {#each sessions.slice(0, 20) as s (s.started_at)}
            <div class="bd-srow">
              <span class="bd-day">{s.day}</span>
              <span>{new Date(s.started_at).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })}</span>
              <span>{fmtDur(s.seconds)}</span>
              <span>{fmtChars(s.chars)} 字</span>
            </div>
          {:else}
            <div class="bd-empty">暂无阅读记录</div>
          {/each}
        </div>
      </div>
    </div>

    <div class="bd-foot">
      <button onclick={copyAll}>复制全部信息</button>
      <div class="flex1"></div>
      <button onclick={onclose}>关闭</button>
      <button class="primary" onclick={save} disabled={saving}>{saving ? "保存中…" : "保存信息"}</button>
    </div>
  </div>
</div>

<style>
  .bd-overlay {
    position: fixed;
    inset: 0;
    background: rgba(20, 20, 24, 0.42);
    z-index: 1200;
    display: grid;
    place-items: center;
    padding: 24px;
  }

  .bd-panel {
    width: min(820px, 94vw);
    max-height: 90vh;
    background: var(--panel);
    color: var(--fg);
    border: 1px solid var(--line);
    border-radius: 14px;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.35);
    display: flex;
    flex-direction: column;
    overflow: hidden;
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
    border-radius: 8px;
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.22);
    background: var(--panel2);
  }

  .bd-cover-acts {
    display: flex;
    gap: 6px;
    margin-top: 10px;
  }

  .bd-cover-acts button { flex: 1; font-size: 12px; padding: 5px 0; }

  .bd-right { flex: 1; min-width: 0; }

  .bd-field { margin-bottom: 10px; }

  .bd-field label {
    display: block;
    font-size: 12px;
    color: var(--fg2);
    margin-bottom: 4px;
  }

  .bd-row { display: flex; gap: 6px; align-items: center; }
  .bd-row input { flex: 1; min-width: 0; }
  .bd-row input.path { font-size: 12px; color: var(--fg2); }

  .bd-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px 16px;
    margin: 12px 0;
  }

  .bd-kv { display: flex; justify-content: space-between; font-size: 13px; }
  .bd-kv span { color: var(--fg2); }
  .bd-kv b { font-weight: 500; }

  .bd-sec {
    font-weight: 600;
    font-size: 13px;
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
    border-radius: 9px;
    padding: 8px 6px;
    text-align: center;
  }

  .bd-s b { display: block; font-size: 14px; color: var(--accent); }
  .bd-s span { font-size: 11px; color: var(--fg2); }

  .bd-sessions { max-height: 190px; overflow-y: auto; }

  .bd-srow {
    display: flex;
    gap: 12px;
    font-size: 12.5px;
    padding: 5px 2px;
    border-bottom: 1px solid var(--line);
  }

  .bd-day { width: 92px; color: var(--fg2); }
  .bd-empty { padding: 18px; text-align: center; color: var(--fg2); font-size: 13px; }

  .bd-foot {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 18px;
    border-top: 1px solid var(--line);
  }

  .flex1 { flex: 1; }
</style>
