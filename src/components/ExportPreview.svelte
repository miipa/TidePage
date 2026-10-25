<script lang="ts">
  /** D1：面板内联导出（按钮 + 格式切换 + 预览 + 保存）；不新增弹窗层级。 */
  import { t } from "../lib/i18n/index.svelte";
  import { api } from "../lib/api";
  import { toast } from "../lib/stores.svelte";
  import {
    toCsv,
    toMarkdown,
    previewText,
    type HighlightExportRow,
  } from "../lib/highlightExport";

  let {
    loadRows,
    defaultName,
  }: {
    loadRows: () => Promise<HighlightExportRow[]>;
    defaultName: string;
  } = $props();

  let open = $state(false);
  let fmt = $state<"md" | "csv">("md");
  let rows = $state<HighlightExportRow[]>([]);
  let loading = $state(false);
  let saving = $state(false);

  const content = $derived(fmt === "md" ? toMarkdown(rows) : toCsv(rows));
  const preview = $derived(content ? previewText(content) : "");

  async function toggle() {
    if (open) {
      open = false;
      return;
    }
    open = true;
    loading = true;
    try {
      rows = await loadRows();
    } catch (e) {
      toast(String(e), "error");
      rows = [];
    } finally {
      loading = false;
    }
  }

  async function save() {
    if (!rows.length) {
      toast(t("export.empty"), "error");
      return;
    }
    saving = true;
    try {
      const path = await api.pickSave(
        t("export.dialogTitle"),
        fmt,
        `${defaultName}.${fmt}`,
      );
      if (!path) return; // 取消不提示
      await api.writeTextFile(path, content);
      const name = path.split(/[\\/]/).pop() || path;
      toast(t("export.success", { name }));
    } catch {
      toast(t("export.failed"), "error");
    } finally {
      saving = false;
    }
  }
</script>

<button class="icon export-btn" data-testid="export-open" title={t("export.button")} onclick={toggle}>
  <span aria-hidden="true">⇩</span>
</button>
{#if open}
  <div class="export-box" data-testid="export-preview">
    <div class="export-bar">
      <select bind:value={fmt} aria-label={t("export.format")}>
        <option value="md">Markdown</option>
        <option value="csv">CSV</option>
      </select>
      <button onclick={save} disabled={saving || loading || !rows.length}>{t("export.save")}</button>
      <button onclick={() => (open = false)}>{t("export.close")}</button>
    </div>
    {#if loading}
      <div class="export-empty">{t("common.loading")}</div>
    {:else if !rows.length}
      <div class="export-empty">{t("export.empty")}</div>
    {:else}
      <pre class="export-pre">{preview}</pre>
    {/if}
  </div>
{/if}

<style>
  .export-btn {
    font-size: 15px;
    line-height: 1;
  }
  .export-box {
    margin: 6px 8px;
    border: 1px solid var(--border, #8884);
    border-radius: 6px;
    padding: 8px;
    background: var(--panel, transparent);
  }
  .export-bar {
    display: flex;
    gap: 8px;
    align-items: center;
    margin-bottom: 6px;
    flex-wrap: wrap;
  }
  .export-pre {
    max-height: 220px;
    overflow: auto;
    white-space: pre-wrap;
    word-break: break-word;
    font-size: 12px;
    margin: 0;
  }
  .export-empty {
    font-size: 12px;
    opacity: 0.7;
    padding: 8px 0;
  }
</style>
