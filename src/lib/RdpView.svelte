<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { api, errText, makeChannel } from "./api";
  import { app, type Tab } from "./state.svelte";
  import Icon from "./Icon.svelte";

  let { tab, active }: { tab: Tab; active: boolean } = $props();

  let el: HTMLDivElement;
  let ro: ResizeObserver;
  let startedGen = -1;
  let mounted = $state(false);
  let rectKey = $state("");

  function rect(): [number, number, number, number] {
    const r = el.getBoundingClientRect();
    const k = window.devicePixelRatio || 1;
    return [Math.round(r.left * k), Math.round(r.top * k), Math.round(r.width * k), Math.round(r.height * k)];
  }

  async function start() {
    startedGen = tab.gen;
    try {
      const r = await api.open({ itemId: tab.itemId, quick: tab.quick?.address, quickProtocol: "rdp",
        quickUsername: tab.quick?.username, quickPassword: tab.quick?.password, cols: 0, rows: 0, rect: rect(), rdpLog: app.settings.rdpLog },
        makeChannel<ArrayBuffer>());
      app.bindSession(tab.key, r.id);
      place();
    } catch (e) {
      tab.state = "error";
      tab.message = errText(e);
    }
  }

  const visible = $derived(
    active && !tab.detached && app.overlays === 0 && tab.state !== "closed" && tab.state !== "error",
  );

  function place() {
    if (tab.sessionId === null || !el) return;
    const [x, y, w, h] = rect();
    api.rdpPlace(tab.sessionId, x, y, w, h, visible && w > 0 && h > 0).catch(() => {});
  }

  onMount(() => {
    let raf = 0;
    ro = new ResizeObserver(() => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => (rectKey = rect().join(",")));
    });
    ro.observe(el);
    const onWin = () => (rectKey = rect().join(","));
    window.addEventListener("resize", onWin);
    mounted = true;
    return () => window.removeEventListener("resize", onWin);
  });

  onDestroy(() => ro?.disconnect());

  $effect(() => {
    if (mounted && tab.gen !== startedGen) start();
  });

  // Позиция и видимость нативного окна
  $effect(() => {
    void rectKey;
    void visible;
    void tab.sessionId;
    place();
  });
</script>

<div class="wrap" class:hidden={!active} bind:this={el}>
  {#if tab.state === "connecting"}
    <div class="center muted">
      <div class="spinner"></div>
      Подключение к {tab.title}…
    </div>
  {:else if tab.state === "closed" || tab.state === "error"}
    <div class="center">
      <Icon name={tab.state === "error" ? "alert" : "plug"} size={28} />
      <div class="msg selectable" class:err={tab.state === "error"}>
        {tab.state === "error" ? tab.message || "Ошибка подключения" : "Сеанс RDP завершён"}
      </div>
      <button class="btn primary" onclick={() => app.reconnect(tab.key)}>
        <Icon name="reconnect" size={14} /> Переподключиться
      </button>
    </div>
  {:else if tab.detached}
    <div class="center">
      <Icon name="copy" size={28} />
      <div class="msg">Сеанс открыт в отдельном окне</div>
      <button class="btn primary" onclick={() => tab.sessionId !== null && api.rdpAttach(tab.sessionId)}>
        Вернуть во вкладку
      </button>
      <small class="faint">Или просто закройте отдельное окно — сеанс вернётся сюда.</small>
    </div>
  {:else if app.overlays > 0 && active}
    <div class="center faint">Окно сеанса скрыто, пока открыт диалог</div>
  {/if}
</div>

<style>
  .wrap {
    position: absolute;
    inset: 0;
    background: #0d0f13;
    color: #c9cfdb;
  }
  .hidden {
    visibility: hidden;
  }
  .center {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 14px;
    text-align: center;
    padding: 20px;
  }
  .msg {
    max-width: 560px;
    white-space: pre-wrap;
    line-height: 1.5;
  }
  .err {
    color: #f58a8a;
  }
  .spinner {
    width: 26px;
    height: 26px;
    border: 3px solid #ffffff22;
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
