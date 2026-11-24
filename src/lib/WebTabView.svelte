<script lang="ts">
  // Веб-вкладка: встроенная страница (отдельный WebView без доступа к командам программы)
  // поверх области вкладки. Скрывается в фоне, под диалогами и при блокировке хранилища.
  import { onMount, onDestroy } from "svelte";
  import { api, errText, listen, parseConn } from "./api";
  import { app, type Tab } from "./state.svelte";
  import { copyPassword, copyUsername } from "./actions";
  import Icon from "./Icon.svelte";

  let { tab, active }: { tab: Tab; active: boolean } = $props();

  let area = $state<HTMLDivElement>();
  let label = "";
  let startedGen = -1;
  let url = $state("");
  let loading = $state(false);
  let rectKey = $state("");
  const item = $derived(tab.itemId ? app.item(tab.itemId) : undefined);
  const home = $derived(item ? (parseConn(item)?.uri ?? "") : (tab.quick?.address ?? ""));

  const visible = $derived(active && app.overlays === 0 && app.phase === "unlocked" && tab.state === "connected");

  function rect() {
    const r = area!.getBoundingClientRect();
    return [r.left, r.top, r.width, r.height] as const;
  }

  async function start() {
    const gen = tab.gen;
    startedGen = gen;
    if (label) api.webClose(label).catch(() => {});
    const myLabel = `web-${tab.key}-${gen}`;
    label = myLabel;
    url = home;
    tab.state = "connecting";
    try {
      const [x, y, w, h] = rect();
      await api.webOpen(myLabel, home, x, y, w, h);
      // Переподключение/закрытие вкладки, пока webOpen был в пути: поздний webview закрываем
      if (tab.gen !== gen || !app.tab(tab.key)) {
        api.webClose(myLabel).catch(() => {});
        return;
      }
      tab.state = "connected";
      place();
    } catch (e) {
      if (tab.gen !== gen) return;
      tab.state = "error";
      tab.message = errText(e);
    }
  }

  function place() {
    if (!label || !area || tab.state !== "connected") return;
    const [x, y, w, h] = rect();
    api.webPlace(label, x, y, w, h, visible && w > 0 && h > 0).catch(() => {});
  }

  onMount(() => {
    const ro = new ResizeObserver(() => (rectKey = rect().join(",")));
    ro.observe(area!);
    const onWin = () => (rectKey = rect().join(","));
    window.addEventListener("resize", onWin);
    const un = listen<{ label: string; url: string; loading: boolean }>("web-nav", (e) => {
      if (e.payload.label !== label) return;
      url = e.payload.url;
      loading = e.payload.loading;
    });
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", onWin);
      un.then((f) => f());
    };
  });
  onDestroy(() => {
    if (label) api.webClose(label).catch(() => {});
  });

  $effect(() => {
    if (area && tab.gen !== startedGen) start();
  });
  $effect(() => {
    void rectKey;
    void visible;
    place();
  });

  const nav = (op: "back" | "forward" | "reload" | "stop") => api.webNav(label, op).catch(() => {});
  const secure = $derived(/^https:/i.test(url));
</script>

<div class="wrap" class:hidden={!active}>
  <div class="bar">
    <button class="icon-btn" title="Назад" onclick={() => nav("back")}><Icon name="chevron" size={14} /></button>
    <button class="icon-btn fwd" title="Вперёд" onclick={() => nav("forward")}><Icon name="chevron" size={14} /></button>
    <button class="icon-btn" title={loading ? "Остановить" : "Обновить"} onclick={() => nav(loading ? "stop" : "reload")}>
      <Icon name={loading ? "x" : "refresh"} size={14} />
    </button>
    <button class="icon-btn" title="Домой: {home}" onclick={() => app.reconnect(tab.key)}><Icon name="home" size={14} /></button>
    <div class="url" class:insecure={!secure} title={secure ? "Защищённое соединение (HTTPS)" : "Соединение без шифрования (HTTP)"}>
      <Icon name={secure ? "lock" : "alert"} size={12} />
      <span class="selectable">{url}</span>
    </div>
    {#if item}
      <button class="icon-btn" title="Копировать логин" onclick={() => copyUsername(item!)} disabled={!item.username}><Icon name="user" size={14} /></button>
      <button class="icon-btn" title="Копировать пароль" onclick={() => copyPassword(item!)} disabled={!item.hasPassword}><Icon name="key" size={14} /></button>
    {/if}
  </div>
  <div class="area" bind:this={area}>
    {#if tab.state === "error"}
      <div class="center">
        <Icon name="alert" size={26} />
        <div class="err selectable">{tab.message}</div>
        <button class="btn primary" onclick={() => app.reconnect(tab.key)}><Icon name="reconnect" size={14} /> Открыть снова</button>
      </div>
    {:else if app.overlays > 0 && active}
      <div class="center faint">Страница скрыта, пока открыт диалог</div>
    {/if}
  </div>
</div>

<style>
  .wrap {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg);
  }
  .hidden {
    visibility: hidden;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-panel);
  }
  .bar :global(.icon-btn:first-child svg) {
    transform: rotate(90deg);
  }
  .fwd :global(svg) {
    transform: rotate(-90deg);
  }
  .url {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0 6px;
    padding: 4px 10px;
    font-size: 12px;
    border-radius: 14px;
    background: var(--bg-sunken);
    color: var(--text-muted);
  }
  .url span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .url.insecure {
    color: var(--warn);
  }
  .area {
    position: relative;
    flex: 1;
    min-height: 0;
    background: #fff;
  }
  .center {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    padding: 20px;
    text-align: center;
    background: var(--bg);
    color: var(--text);
  }
  .err {
    color: var(--danger);
    max-width: 520px;
  }
</style>
