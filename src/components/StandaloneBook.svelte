<script lang="ts">
  import { onMount } from "svelte";
  import Reader from "./Reader.svelte";

  const bookId = Number(new URLSearchParams(window.location.search).get("book") || "0");
  let reader: { open: (id: number) => Promise<void> } | null = $state(null);

  onMount(() => {
    const t = setInterval(() => {
      if (reader && bookId) {
        clearInterval(t);
        reader.open(bookId);
      }
    }, 30);
  });
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
