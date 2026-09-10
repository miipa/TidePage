<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import { toast } from "../lib/stores.svelte";

  let { children } = $props();

  onMount(() => {
    const onErr = (e: ErrorEvent) => {
      toast(`脚本错误: ${e.message}`, "error");
      console.error(e.error);
    };
    const onRej = (e: PromiseRejectionEvent) => {
      toast(`未处理的错误: ${String(e.reason)}`, "error");
      console.error(e.reason);
    };
    window.addEventListener("error", onErr);
    window.addEventListener("unhandledrejection", onRej);
    return () => {
      window.removeEventListener("error", onErr);
      window.removeEventListener("unhandledrejection", onRej);
    };
  });
</script>

<svelte:head>
  <title>FlowRead</title>
</svelte:head>

{@render children()}
