<script lang="ts">
  import { api } from "./api";
  import { dialogs, menu } from "./dialogs.svelte";
  import { app, type PromptReq } from "./state.svelte";
  import Icon from "./Icon.svelte";

  // Запросы от сессий показываем по одному
  const prompt = $derived(app.prompts[0] as PromptReq | undefined);
  let promptValue = $state("");
  let shownPrompt = 0;

  $effect(() => {
    if (prompt && prompt.id !== shownPrompt) {
      shownPrompt = prompt.id;
      promptValue = "";
      app.pushOverlay();
    }
  });

  function answer(value: string | null) {
    if (!prompt) return;
    api.promptAnswer(prompt.id, value).catch(() => {});
    app.prompts = app.prompts.slice(1);
    app.popOverlay();
  }

  function tabTitle(session: number) {
    return app.tabs.find((t) => t.sessionId === session)?.title ?? "";
  }

  let menuEl = $state<HTMLDivElement>();
  let menuPos = $state({ x: 0, y: 0 });
  $effect(() => {
    if (menu.open && menuEl) {
      const r = menuEl.getBoundingClientRect();
      menuPos = {
        x: Math.min(menu.x, window.innerWidth - r.width - 6),
        y: Math.min(menu.y, window.innerHeight - r.height - 6),
      };
    }
  });

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      if (menu.open) menu.close();
      else if (dialogs.list.length) dialogs.list[dialogs.list.length - 1].resolve(null);
      else if (prompt) answer(null);
    }
  }

  function focusOnMount(el: HTMLElement) {
    setTimeout(() => el.focus(), 0);
  }
</script>

<svelte:window onkeydown={onKey} />

{#if menu.open}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="menu-backdrop" onmousedown={() => menu.close()} oncontextmenu={(e) => (e.preventDefault(), menu.close())}></div>
  <div class="menu" bind:this={menuEl} style="left:{menuPos.x || menu.x}px; top:{menuPos.y || menu.y}px" role="menu">
    {#each menu.items as it}
      {#if it === "sep"}
        <div class="sep"></div>
      {:else}
        <button
          class:danger={it.danger}
          disabled={it.disabled}
          role="menuitem"
          onclick={() => {
            menu.close();
            it.action();
          }}
        >
          <span class="mi">{#if it.icon}<Icon name={it.icon} size={15} />{/if}</span>
          <span class="ml">{it.label}</span>
          {#if it.hint}<span class="mh">{it.hint}</span>{/if}
        </button>
      {/if}
    {/each}
  </div>
{/if}

{#each dialogs.list as d (d.id)}
  <div class="backdrop">
    <form class="dialog" onsubmit={(e) => (e.preventDefault(), d.resolve(d.kind === "input" ? d.value : "ok"))}>
      <div class="dh">{d.title}</div>
      {#if d.message}<div class="dm">{d.message}</div>{/if}
      {#if d.kind === "input"}
        <input bind:value={d.value} use:focusOnMount />
      {/if}
      <div class="da">
        <button type="button" class="btn" onclick={() => d.resolve(null)}>Отмена</button>
        {#if d.altText}<button type="button" class="btn" onclick={() => d.resolve("alt")}>{d.altText}</button>{/if}
        <button
          type="submit"
          class="btn {d.danger ? 'danger solid' : 'primary'}"
          use:focusOnMount
          disabled={d.kind === "input" && !d.value.trim()}>{d.okText}</button
        >
      </div>
    </form>
  </div>
{/each}

{#if prompt}
  <div class="backdrop">
    <form
      class="dialog"
      onsubmit={(e) => (e.preventDefault(), answer(prompt.kind === "confirm" ? "yes" : promptValue))}
    >
      <div class="dh" class:dangerh={prompt.danger}>
        {#if prompt.danger}<Icon name="alert" size={18} />{/if}
        {prompt.title}
      </div>
      {#if tabTitle(prompt.session)}<div class="faint small">{tabTitle(prompt.session)}</div>{/if}
      <div class="dm selectable">{prompt.message}</div>
      {#if prompt.kind !== "confirm"}
        <input type={prompt.kind === "secret" ? "password" : "text"} bind:value={promptValue} use:focusOnMount />
      {/if}
      <div class="da">
        <button type="button" class="btn" onclick={() => answer(null)}>
          {prompt.kind === "confirm" ? "Отклонить" : "Отмена"}
        </button>
        <button type="submit" class="btn {prompt.danger ? 'danger solid' : 'primary'}">
          {prompt.kind === "confirm" ? "Доверять" : "OK"}
        </button>
      </div>
    </form>
  </div>
{/if}

<div class="toasts">
  {#each app.toasts as t (t.id)}
    <div class="toast" class:err={t.error}>
      <Icon name={t.error ? "alert" : "check"} size={15} />
      <span class="selectable">{t.text}</span>
    </div>
  {/each}
</div>

<style>
  .menu-backdrop {
    position: fixed;
    inset: 0;
    z-index: 90;
  }
  .menu {
    position: fixed;
    z-index: 91;
    min-width: 210px;
    padding: 4px;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    box-shadow: var(--shadow);
  }
  .menu button {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 8px;
    text-align: left;
    border-radius: 4px;
  }
  .menu button:hover:not(:disabled) {
    background: var(--accent);
    color: var(--accent-text);
  }
  .menu button.danger {
    color: var(--danger);
  }
  .menu button.danger:hover {
    background: var(--danger);
    color: #fff;
  }
  .mi {
    width: 16px;
    display: inline-flex;
  }
  .ml {
    flex: 1;
  }
  .mh {
    font-size: 11px;
    opacity: 0.6;
  }
  .sep {
    height: 1px;
    background: var(--border);
    margin: 4px 6px;
  }
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 100;
    background: rgba(10, 12, 18, 0.45);
    display: grid;
    place-items: center;
    animation: fade 0.12s;
  }
  .dialog {
    width: min(460px, 92vw);
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 12px;
    box-shadow: var(--shadow);
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    animation: pop 0.14s ease-out;
  }
  .dh {
    font-size: 15px;
    font-weight: 600;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .dangerh {
    color: var(--danger);
  }
  .dm {
    white-space: pre-wrap;
    color: var(--text-muted);
    line-height: 1.5;
    word-break: break-word;
  }
  .da {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }
  .small {
    font-size: 12px;
  }
  .toasts {
    position: fixed;
    right: 16px;
    bottom: 34px;
    z-index: 200;
    display: flex;
    flex-direction: column;
    gap: 8px;
    pointer-events: none;
  }
  .toast {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    max-width: 420px;
    padding: 10px 14px;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-left: 3px solid var(--ok);
    border-radius: var(--radius);
    box-shadow: var(--shadow);
    pointer-events: auto;
    animation: slide 0.18s ease-out;
    white-space: pre-wrap;
  }
  .toast.err {
    border-left-color: var(--danger);
  }
  .toast.err :global(.icon) {
    color: var(--danger);
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  @keyframes pop {
    from {
      transform: scale(0.97);
      opacity: 0;
    }
  }
  @keyframes slide {
    from {
      transform: translateY(8px);
      opacity: 0;
    }
  }
</style>
