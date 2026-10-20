<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { emit, type UnlistenFn } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import Reader from "./Reader.svelte";

  const bookId = Number(new URLSearchParams(window.location.search).get("book") || "0");
  let reader: { open: (id: number) => Promise<void> } | null = $state(null);
  let unClose: UnlistenFn | null = null;

  onMount(async () => {
    const t = setInterval(() => {
      if (reader && bookId) {
        clearInterval(t);
        reader.open(bookId);
      }
    }, 30);
    // 本窗关闭（右上角 X 与返回按钮都会走窗口关闭）：通知面板窗清空当前书。
    // 必须 await emit：Tauri 在 handler 返回后立即销毁窗口，不等 IPC 送达就没了。
    unClose = await getCurrentWindow().onCloseRequested(async () => {
      await emit("tidepage-book-closed", { bookId });
    });
  });

  onDestroy(() => { unClose?.(); });
</script>

<div class="sb-root">
  <Reader bind:this={reader} standalone />
</div>

<style>
  .sb-root {
    height: 100vh;
    background: var(--rdr-bg);
  }
</style>
