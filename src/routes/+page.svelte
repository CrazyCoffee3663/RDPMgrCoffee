<script lang="ts">
  import { onMount } from "svelte";
  import { app, applyTheme, setupEvents, shouldLock } from "$lib/state.svelte";
  import { api, errText, PROTOCOL_LABEL, TAB_PROTOCOLS } from "$lib/api";
  import { getCurrentWindow } from "@tauri-apps/api/window";

  const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
  import { newItem, checkWinbox, installWinbox, checkFreerdp, updateFreerdp, checkAppUpdate, updateApp } from "$lib/actions";
  import { menu } from "$lib/dialogs.svelte";
  import Unlock from "$lib/Unlock.svelte";
  import Sidebar from "$lib/Sidebar.svelte";
  import Editor from "$lib/Editor.svelte";
  import Home from "$lib/Home.svelte";
  import Settings from "$lib/Settings.svelte";
  import WindowControls from "$lib/WindowControls.svelte";
  import ImportDialog from "$lib/ImportDialog.svelte";
  import SnippetsPopup from "$lib/SnippetsPopup.svelte";
  import Overlays from "$lib/Overlays.svelte";
  import ExportDialog from "$lib/ExportDialog.svelte";
  import Tour from "$lib/Tour.svelte";
  import Icon from "$lib/Icon.svelte";

  // Вьюхи с тяжёлыми зависимостями (xterm, noVNC) грузятся при первой вкладке своего
  // протокола: статические импорты тянули их в основной чанк (~858 кБ). Кеш промиса —
  // чтобы {#await} не мигал загрузкой при каждом обновлении вкладки.
  let termP: Promise<typeof import("$lib/TermView.svelte")> | null = null;
  let rdpP: Promise<typeof import("$lib/RdpView.svelte")> | null = null;
  let vncP: Promise<typeof import("$lib/VncView.svelte")> | null = null;
  let webP: Promise<typeof import("$lib/WebTabView.svelte")> | null = null;
  const termView = () => (termP ??= import("$lib/TermView.svelte"));
  const rdpView = () => (rdpP ??= import("$lib/RdpView.svelte"));
  const vncView = () => (vncP ??= import("$lib/VncView.svelte"));
  const webView = () => (webP ??= import("$lib/WebTabView.svelte"));

  let sidebar = $state<ReturnType<typeof Sidebar>>();
  let showSidebar = $state(true);
  let quick = $state("");
  let quickProto = $state("ssh");
  let quickEl = $state<HTMLInputElement>();
  let lastActivity = Date.now();

  onMount(() => {
    applyTheme(app.settings.theme);
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onScheme = () => applyTheme(app.settings.theme);
    mq.addEventListener("change", onScheme);
    let unEvents: (() => void) | undefined;
    setupEvents().then((f) => (unEvents = f));
    app.init();

    const bump = () => (lastActivity = Date.now());
    window.addEventListener("mousemove", bump, { passive: true });
    window.addEventListener("keydown", bump, { passive: true });

    // Автоблокировка, фоновая синхронизация и проверка доступности хостов
    let lastSync = Date.now();
    let lastProbe = 0;
    const timer = setInterval(async () => {
      if (app.phase !== "unlocked") return;
      const lockMin = app.settings.autoLockMinutes;
      if (shouldLock(Date.now() - lastActivity, lockMin)) {
        // Бездействие — по всей системе: работа в RDP-окне или другой программе не видна странице.
        // Если системное время ввода недоступно — по событиям страницы.
        const idle = await api.idleMs().catch(() => Date.now() - lastActivity);
        if (shouldLock(idle, lockMin)) {
          app.lock();
          return;
        }
        lastActivity = Date.now() - idle;
      }
      if (Date.now() - lastSync > Math.max(1, app.settings.syncMinutes) * 60_000) {
        lastSync = Date.now();
        app.sync(false);
      }
      if (app.settings.probeEnabled && Date.now() - lastProbe > Math.max(0.5, app.settings.probeMinutes) * 60_000) {
        lastProbe = Date.now();
        app.probe();
      }
    }, 15_000);

    return () => {
      clearInterval(timer);
      mq.removeEventListener("change", onScheme);
      // Без снятия слушателей и подписок dev-HMR удваивал диалоги и таймеры
      window.removeEventListener("mousemove", bump);
      window.removeEventListener("keydown", bump);
      unEvents?.();
    };
  });

  // Проверка обновлений WinBox после разблокировки (раз в сутки)
  $effect(() => {
    if (app.phase === "unlocked") setTimeout(() => (checkWinbox(), checkFreerdp(), checkAppUpdate()), 5000);
  });

  // Окно настроек скрывает окна RDP
  $effect(() => {
    if (app.settingsOpen) {
      app.pushOverlay();
      return () => app.popOverlay();
    }
  });
  $effect(() => {
    if (app.snippetsOpen) {
      app.pushOverlay();
      return () => app.popOverlay();
    }
  });
  $effect(() => {
    if (app.exportReq) {
      app.pushOverlay();
      return () => app.popOverlay();
    }
  });
  $effect(() => {
    if (app.tourOpen) {
      app.pushOverlay();
      return () => app.popOverlay();
    }
  });
  $effect(() => {
    if (app.importOpen) {
      app.pushOverlay();
      return () => app.popOverlay();
    }
  });

  function onKey(e: KeyboardEvent) {
    if (app.phase !== "unlocked") return;
    const k = e.code;
    if (e.ctrlKey && !e.shiftKey && k === "KeyF") {
      e.preventDefault();
      showSidebar = true;
      setTimeout(() => sidebar?.focusSearch());
    } else if (e.ctrlKey && k === "KeyK") {
      e.preventDefault();
      quickEl?.focus();
    } else if (e.ctrlKey && !e.shiftKey && k === "KeyN") {
      e.preventDefault();
      newItem(app.mode === "conn" ? "conn" : "login");
    } else if (e.ctrlKey && e.key === "Tab") {
      e.preventDefault();
      app.cycleTab(e.shiftKey ? -1 : 1);
    } else if (e.ctrlKey && e.shiftKey && k === "KeyW") {
      e.preventDefault();
      if (app.activeTab) app.closeTab(app.activeTab);
    } else if (e.ctrlKey && !e.shiftKey && k === "KeyL") {
      e.preventDefault();
      app.lock();
    } else if (e.ctrlKey && e.shiftKey && k === "KeyS") {
      e.preventDefault();
      app.snippetsOpen = !app.snippetsOpen;
    } else if (k === "F11") {
      e.preventDefault();
      setFullscreen(!app.fullscreen);
    } else if (k === "Escape" && app.fullscreen && !app.overlays) {
      setFullscreen(false);
    } else if (e.ctrlKey && k === "KeyB") {
      e.preventDefault();
      showSidebar = !showSidebar;
    }
  }

  function doQuick() {
    const a = quick.trim();
    if (!a) return;
    app.quickConnect(a, quickProto);
    quick = "";
  }

  async function setFullscreen(on: boolean) {
    app.fullscreen = on;
    if (inTauri) await getCurrentWindow().setFullscreen(on).catch(() => {});
  }

  async function detachTab(key: string) {
    const t = app.tab(key);
    if (!t || t.sessionId === null) return;
    try {
      await api.rdpDetach(t.sessionId, t.title);
      t.detached = true;
    } catch (e) {
      app.toast(errText(e), true);
    }
  }

  function tabMenu(e: MouseEvent, key: string) {
    const others = app.tabs.filter((t) => t.key !== key).map((t) => t.key);
    const t = app.tab(key);
    const rdpLive = t?.protocol === "rdp" && t.state === "connected";
    menu.show(e, [
      { label: "Переподключиться", icon: "reconnect", action: () => app.reconnect(key) },
      t?.detached
        ? { label: "Вернуть во вкладку", icon: "sidebar", action: () => t.sessionId !== null && api.rdpAttach(t.sessionId) }
        : { label: "Открепить в отдельное окно", icon: "copy", action: () => detachTab(key), disabled: !rdpLive },
      {
        label: app.fullscreen ? "Выйти из полноэкранного режима" : "Полноэкранный режим",
        icon: "monitor",
        action: () => ((app.activeTab = key), setFullscreen(!app.fullscreen)),
        hint: "F11",
      },
      {
        label: "Изменить подключение",
        icon: "edit",
        action: () => (app.editing = app.tab(key)?.itemId ?? null),
        disabled: !app.tab(key)?.itemId,
      },
      "sep",
      { label: "Закрыть", icon: "x", action: () => app.closeTab(key) },
      { label: "Закрыть остальные", action: () => others.forEach((k) => app.closeTab(k)), disabled: !others.length },
      { label: "Закрыть все", action: () => [...app.tabs].forEach((t) => app.closeTab(t.key)) },
    ]);
  }

  // Перетаскивание вкладок
  let dragTab: string | null = null;
  function onTabDrop(target: string) {
    if (!dragTab || dragTab === target) return;
    const from = app.tabs.findIndex((t) => t.key === dragTab);
    const to = app.tabs.findIndex((t) => t.key === target);
    const [t] = app.tabs.splice(from, 1);
    app.tabs.splice(to, 0, t);
    dragTab = null;
  }

  const active = $derived(app.tab(app.activeTab));
  const connectedCount = $derived(app.tabs.filter((t) => t.state === "connected").length);
</script>

<svelte:window onkeydown={onKey} />

{#if app.phase === "loading"}
  <div class="loading"></div>
{:else if app.phase !== "unlocked" && !app.tabs.length}
  <Unlock />
{:else}
  <div class="app" class:fs={app.fullscreen}>
    <div class="toolbar" data-tauri-drag-region>
      <button class="icon-btn" class:active={showSidebar} title="Панель подключений (Ctrl+B)" onclick={() => (showSidebar = !showSidebar)}>
        <Icon name="sidebar" />
      </button>
      <form class="quick" onsubmit={(e) => (e.preventDefault(), doQuick())}>
        <Icon name="zap" size={14} />
        <select bind:value={quickProto} title="Протокол">
          {#each TAB_PROTOCOLS as p}<option value={p}>{PROTOCOL_LABEL[p]}</option>{/each}
        </select>
        <input bind:this={quickEl} bind:value={quick} placeholder="Быстрое подключение: user@host:port  (Ctrl+K)" />
        <button class="go" type="submit" disabled={!quick.trim()} title="Подключиться"><Icon name="play" size={13} /></button>
      </form>
      <span class="sp" data-tauri-drag-region></span>
      <button class="icon-btn" title="Сниппеты команд (Ctrl+Shift+S)" onclick={() => (app.snippetsOpen = true)}><Icon name="terminal" /></button>
      <button class="icon-btn" title="Синхронизировать" onclick={() => app.sync(true)} disabled={app.syncing}>
        <span class:spin={app.syncing}><Icon name="refresh" /></span>
      </button>
      <button class="icon-btn" title="Заблокировать (Ctrl+L)" onclick={() => app.lock()}><Icon name="lock" /></button>
      <button class="icon-btn" title="Настройки" onclick={() => (app.settingsOpen = true)}><Icon name="settings" /></button>
      <WindowControls />
    </div>

    <div class="main">
      {#if showSidebar && !app.fullscreen}<Sidebar bind:this={sidebar} />{/if}

      <div class="center">
        {#if app.tabs.length}
          <div class="tabs" role="tablist">
            <button class="tab home-tab" class:on={!app.activeTab} title="Главная" onclick={() => (app.activeTab = null)}>
              <Icon name="home" size={14} />
            </button>
            {#each app.tabs as t (t.key)}
              <div
                class="tab proto-{t.protocol}"
                class:on={app.activeTab === t.key}
                role="tab"
                tabindex="0"
                aria-selected={app.activeTab === t.key}
                draggable="true"
                ondragstart={() => (dragTab = t.key)}
                ondragover={(e) => e.preventDefault()}
                ondrop={() => onTabDrop(t.key)}
                onclick={() => (app.activeTab = t.key)}
                onkeydown={(e) => e.key === "Enter" && (app.activeTab = t.key)}
                onauxclick={(e) => e.button === 1 && app.closeTab(t.key)}
                oncontextmenu={(e) => tabMenu(e, t.key)}
                title={t.message || t.title}
              >
                <span class="st {t.state}"></span>
                <span class="tt">{t.title}</span>
                {#if t.detached}<span class="tdet" title="Открыта в отдельном окне"><Icon name="copy" size={11} /></span>{/if}
                <button class="tx" title="Закрыть (Ctrl+Shift+W)" onclick={(e) => (e.stopPropagation(), app.closeTab(t.key))}>
                  <Icon name="x" size={12} />
                </button>
              </div>
            {/each}
            <span class="tsp"></span>
            <button
              class="tab fs-btn"
              title={app.fullscreen ? "Выйти из полноэкранного режима (F11)" : "Полноэкранный режим (F11)"}
              onclick={() => setFullscreen(!app.fullscreen)}
            >
              <Icon name={app.fullscreen ? "x" : "monitor"} size={14} />
            </button>
          </div>
        {/if}
        <div class="views">
          {#each app.tabs as t (t.key)}
            {#if t.protocol === "rdp"}
              {#await rdpView()}
                <div class="viewloading"></div>
              {:then m}
                <m.default tab={t} active={app.activeTab === t.key} />
              {/await}
            {:else if t.protocol === "vnc"}
              {#await vncView()}
                <div class="viewloading"></div>
              {:then m}
                <m.default tab={t} active={app.activeTab === t.key} />
              {/await}
            {:else if t.protocol === "web"}
              {#await webView()}
                <div class="viewloading"></div>
              {:then m}
                <m.default tab={t} active={app.activeTab === t.key} />
              {/await}
            {:else}
              {#await termView()}
                <div class="viewloading"></div>
              {:then m}
                <m.default tab={t} active={app.activeTab === t.key} />
              {/await}
            {/if}
          {/each}
          {#if !app.activeTab}<Home />{/if}
        </div>
      </div>

      {#if app.editing}<Editor />{/if}
    </div>

    <div class="status">
      {#if app.status.local}
      <span class="si" title="Зашифрованное хранилище на этом компьютере, без сервера">
        <Icon name="lock" size={12} /> Локальное хранилище
      </span>
      {:else}
      <span class="si" class:err={!!app.syncError} title={app.syncError || "Синхронизировано"}>
        <Icon name="cloud" size={13} />
        {app.syncing ? "Синхронизация…" : app.syncError ? "Нет связи с сервером" : "Синхронизировано"}
      </span>
      {/if}
      <span class="faint">{app.status.email}</span>
      <span class="sp"></span>
      {#if app.appUpdate}
        <button
          class="upd"
          onclick={() => updateApp()}
          disabled={!!app.appUpdating}
          title="Установлена {app.appUpdate.current} — установщик с GitHub, проверка SHA-256"
        >
          <Icon name="alert" size={12} /> Вышла версия {app.appUpdate.latest} — обновить
        </button>
      {/if}
      {#if app.freerdpUpdate}
        <button
          class="upd"
          onclick={() => updateFreerdp()}
          disabled={!!app.freerdpUpdating}
          title="Встроен FreeRDP {app.freerdpUpdate.installed} — обновление закрывает уязвимости RDP-клиента"
        >
          <Icon name="alert" size={12} /> Вышел FreeRDP {app.freerdpUpdate.latest} — обновить
        </button>
      {/if}
      {#if app.winboxUpdate}
        {@const u = app.winboxUpdate}
        <button class="upd" onclick={() => installWinbox(u.latest, true)} title="Установлена {u.installed}">
          <Icon name="download" size={12} /> Доступна WinBox {u.latest}
        </button>
      {/if}
      {#if active}<span class="faint">{active.title}</span>{/if}
      <span>Сеансов: {connectedCount}</span>
    </div>
  </div>

  {#if app.settingsOpen}<Settings onclose={() => (app.settingsOpen = false)} />{/if}
  {#if app.importOpen}<ImportDialog onclose={() => (app.importOpen = false)} />{/if}
  {#if app.exportReq}<ExportDialog onclose={() => (app.exportReq = null)} />{/if}
  {#if app.tourOpen}<Tour onclose={() => (app.tourOpen = false)} />{/if}
  {#if app.snippetsOpen}<SnippetsPopup onclose={() => (app.snippetsOpen = false)} />{/if}
  {#if app.phase !== "unlocked"}
    <!-- Есть открытые сеансы: экран блокировки закрывает их, но не разрывает -->
    <div class="lockcover"><Unlock /></div>
  {/if}
{/if}

<Overlays />

<style>
  .loading {
    height: 100vh;
    background: var(--bg);
  }
  .app {
    height: 100vh;
    display: flex;
    flex-direction: column;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
    background: var(--bg-panel);
    border-bottom: 1px solid var(--border);
  }
  .sp {
    flex: 1;
  }
  .quick {
    display: flex;
    align-items: center;
    gap: 6px;
    width: min(560px, 50vw);
    padding: 0 3px 0 10px;
    background: var(--bg-sunken);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text-faint);
  }
  .quick:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  .quick select {
    width: auto;
    border: 0;
    background-color: transparent;
    padding: 4px 22px 4px 2px;
    font-weight: 600;
    color: var(--text-muted);
    box-shadow: none !important;
  }
  .quick input {
    border: 0;
    background: none;
    padding: 6px 0;
    box-shadow: none !important;
  }
  .go {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 24px;
    color: var(--accent);
  }
  .go:hover:not(:disabled) {
    background: var(--accent-soft);
  }
  .spin {
    display: inline-flex;
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .main {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .center {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .upd {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 1px 8px;
    font-size: 11.5px;
    color: var(--accent);
    background: var(--accent-soft);
    border-radius: 10px;
  }
  .fs .toolbar,
  .fs .status {
    display: none;
  }
  .tsp {
    flex: 1;
  }
  .fs-btn {
    flex-shrink: 0;
  }
  .tdet {
    display: inline-flex;
    color: var(--text-faint);
  }
  .tabs {
    display: flex;
    align-items: flex-end;
    gap: 2px;
    padding: 6px 6px 0;
    background: var(--bg);
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    flex-shrink: 0;
  }
  .tabs::-webkit-scrollbar {
    height: 0;
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 7px;
    height: 32px;
    padding: 0 6px 0 10px;
    max-width: 220px;
    min-width: 0;
    border: 1px solid transparent;
    border-bottom: 0;
    border-radius: var(--radius) var(--radius) 0 0;
    color: var(--text-muted);
    cursor: default;
    position: relative;
    flex-shrink: 0;
  }
  .tab:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .tab.on {
    background: var(--bg-panel);
    border-color: var(--border);
    color: var(--text);
    margin-bottom: -1px;
    height: 33px;
  }
  .tab.on::before {
    content: "";
    position: absolute;
    left: -1px;
    right: -1px;
    top: -1px;
    height: 2px;
    background: var(--pc, var(--accent));
    border-radius: var(--radius) var(--radius) 0 0;
  }
  .home-tab {
    padding: 0 10px;
  }
  .tt {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .st {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--text-faint);
  }
  .st.connected {
    background: var(--ok);
  }
  .st.connecting {
    background: var(--warn);
    animation: pulse 1s infinite;
  }
  .st.error {
    background: var(--danger);
  }
  @keyframes pulse {
    50% {
      opacity: 0.3;
    }
  }
  .tx {
    display: flex;
    padding: 3px;
    border-radius: 4px;
    color: var(--text-faint);
  }
  .tx:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .views {
    flex: 1;
    position: relative;
    min-height: 0;
    background: var(--bg);
  }
  .viewloading {
    position: absolute;
    inset: 0;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 14px;
    height: 24px;
    padding: 0 10px;
    font-size: 11.5px;
    color: var(--text-muted);
    background: var(--bg-panel);
    border-top: 1px solid var(--border);
  }
  .si {
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .si.err {
    color: var(--warn);
  }
  .lockcover {
    position: fixed;
    inset: 0;
    z-index: 1000;
    /* Непрозрачная подложка: в некоторых темах фон полупрозрачный */
    background: linear-gradient(var(--bg), var(--bg)), #0e1014;
    backdrop-filter: blur(40px);
  }
  :global(:root[data-theme="light"]) .lockcover {
    background: linear-gradient(var(--bg), var(--bg)), #f4f5f7;
  }
</style>
