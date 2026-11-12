<script lang="ts">
  import "../app.css";
  import ToolWindow from "$lib/ToolWindow.svelte";
  let { children } = $props();
  // Окно инструмента (ping, tracert…) открывается с ?tool=… — без основного интерфейса
  const toolMode = typeof location !== "undefined" && new URLSearchParams(location.search).has("tool");
</script>

{#if toolMode}
  <ToolWindow />
{:else}
  {@render children()}
{/if}
