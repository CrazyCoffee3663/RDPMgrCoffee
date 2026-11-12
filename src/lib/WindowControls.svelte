<script lang="ts">
  // Кнопки окна для собственного заголовка (системная рамка отключена,
  // чтобы верхняя полоса окрашивалась темой приложения).
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";

  const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
  const win = inTauri ? getCurrentWindow() : null;
  let maximized = $state(false);

  onMount(() => {
    if (!win) return;
    let un: (() => void) | undefined;
    const sync = async () => (maximized = await win.isMaximized());
    sync();
    win.onResized(sync).then((f) => (un = f));
    return () => un?.();
  });
</script>

{#if win}
  <div class="wc">
    <button title="Свернуть" onclick={() => win.minimize()} aria-label="Свернуть">
      <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 5h10" stroke="currentColor" stroke-width="1" /></svg>
    </button>
    <button title={maximized ? "Восстановить" : "Развернуть"} onclick={() => win.toggleMaximize()} aria-label="Развернуть">
      {#if maximized}
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1"
          ><path d="M2.5 2.5V0.5h7v7h-2" /><rect x="0.5" y="2.5" width="7" height="7" /></svg
        >
      {:else}
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1"
          ><rect x="0.5" y="0.5" width="9" height="9" /></svg
        >
      {/if}
    </button>
    <button class="close" title="Закрыть" onclick={() => win.close()} aria-label="Закрыть">
      <svg width="10" height="10" viewBox="0 0 10 10" stroke="currentColor" stroke-width="1.1"><path d="M0 0l10 10M10 0L0 10" /></svg>
    </button>
  </div>
{/if}

<style>
  .wc {
    display: flex;
    align-self: stretch;
    margin: -6px -8px -6px 4px;
  }
  button {
    width: 46px;
    display: grid;
    place-items: center;
    border: 0;
    border-radius: 0;
    color: var(--text-muted);
  }
  button:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .close:hover {
    background: #e81123;
    color: #fff;
  }
</style>
