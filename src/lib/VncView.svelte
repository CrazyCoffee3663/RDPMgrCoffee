<script lang="ts">
  // VNC во вкладке: протокол RFB разбирает noVNC, байты ходят через Tauri (Rust держит TCP).
  // Буфер обмена: с сервера — только в активной вкладке при разблокированном хранилище;
  // на сервер — только по кнопке «Вставить», чтобы содержимое буфера не уходило само.
  import { onDestroy } from "svelte";
  import RFBModule from "@novnc/novnc/lib/rfb";
  // noVNC 1.4 собран в CommonJS: в разных сборщиках класс приходит как default или как default.default
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const RFB = ((RFBModule as any).default ?? RFBModule) as typeof RFBModule;
  type RFB = RFBModule;
  import { api, errText, makeChannel } from "./api";
  import { app, type Tab } from "./state.svelte";
  import Icon from "./Icon.svelte";

  let { tab, active }: { tab: Tab; active: boolean } = $props();

  /** Канал для noVNC вместо WebSocket: свойства, которые проверяет Websock.attach */
  class IpcChannel {
    binaryType = "arraybuffer";
    protocol = "";
    readyState = "connecting";
    onopen: (() => void) | null = null;
    onmessage: ((e: { data: ArrayBuffer }) => void) | null = null;
    onclose: ((e: { code: number; reason: string; wasClean: boolean }) => void) | null = null;
    onerror: ((e: unknown) => void) | null = null;
    sid: number | null = null;
    private pending: ArrayBuffer[] = [];
    send(d: ArrayBuffer | Uint8Array) {
      // Пока хранилище заблокировано, ввод на сервер не уходит
      if (this.sid === null || this.readyState !== "open" || app.phase !== "unlocked") return;
      const u8 = d instanceof Uint8Array ? d : new Uint8Array(d);
      api.inputBytes(this.sid, Array.from(u8)).catch(() => {});
    }
    close() {
      if (this.readyState === "closed") return;
      this.readyState = "closed";
      if (this.sid !== null) api.close(this.sid).catch(() => {});
    }
    deliver(buf: ArrayBuffer) {
      if (this.readyState === "open" && this.onmessage) this.onmessage({ data: buf });
      else this.pending.push(buf);
    }
    opened() {
      if (this.readyState !== "connecting") return;
      this.readyState = "open";
      this.onopen?.();
      for (const b of this.pending.splice(0)) this.onmessage?.({ data: b });
    }
    ended() {
      if (this.readyState === "closed") return;
      this.readyState = "closed";
      this.onclose?.({ code: 1000, reason: "", wasClean: true });
    }
  }

  let screen = $state<HTMLDivElement>();
  let rfb: RFB | null = null;
  let chan: IpcChannel | null = null;
  let startedGen = -1;
  let needPw = $state<string[] | null>(null);
  let pwUser = $state("");
  let pwValue = $state("");
  let vncError = $state("");

  async function start() {
    startedGen = tab.gen;
    stop();
    vncError = "";
    chan = new IpcChannel();
    const c = chan;
    rfb = new RFB(screen!, c, {});
    rfb.scaleViewport = true;
    rfb.resizeSession = false;
    rfb.focusOnClick = true;
    rfb.addEventListener("credentialsrequired", (e: CustomEvent<{ types: string[] }>) => credentials(e.detail.types));
    rfb.addEventListener("securityfailure", (e: CustomEvent<{ reason?: string }>) => {
      vncError = e.detail.reason || "Неверный пароль VNC";
    });
    rfb.addEventListener("clipboard", (e: CustomEvent<{ text: string }>) => {
      if (active && app.phase === "unlocked") api.clipboardSet(e.detail.text).catch(() => {});
    });
    const ch = makeChannel<ArrayBuffer>();
    ch.onmessage = (d) => c.deliver(d);
    try {
      const r = await api.open(
        { itemId: tab.itemId, quick: tab.quick?.address, quickProtocol: "vnc", quickUsername: tab.quick?.username, quickPassword: tab.quick?.password, cols: 0, rows: 0 },
        ch,
      );
      c.sid = r.id;
      app.bindSession(tab.key, r.id);
    } catch (e) {
      tab.state = "error";
      tab.message = errText(e);
    }
  }

  function stop() {
    try {
      rfb?.disconnect();
    } catch {
      /* уже закрыт */
    }
    rfb = null;
    chan?.ended();
    chan = null;
  }

  /** Пароль (и логин для VeNCrypt/ARD): из записи хранилища, иначе спросить */
  async function credentials(types: string[]) {
    let password = tab.quick?.password ?? "";
    let username = tab.quick?.username ?? "";
    if (tab.itemId) {
      try {
        password = (await api.secret(tab.itemId)).password;
        username = app.item(tab.itemId)?.username ?? "";
      } catch {
        /* спросим */
      }
    }
    if (!password || (types.includes("username") && !username)) {
      pwUser = username;
      pwValue = "";
      needPw = types;
      return;
    }
    rfb?.sendCredentials({ username, password });
  }
  function submitPw() {
    rfb?.sendCredentials({ username: pwUser, password: pwValue });
    pwValue = "";
    needPw = null;
  }

  async function paste() {
    try {
      const t = await api.clipboardGet();
      if (t) rfb?.clipboardPasteFrom(t);
      rfb?.focus();
    } catch (e) {
      app.toast(errText(e), true);
    }
  }

  $effect(() => {
    if (screen && tab.gen !== startedGen) start();
  });
  // Состояние сеанса → канал noVNC
  $effect(() => {
    if (tab.state === "connected") chan?.opened();
    else if (tab.state === "closed" || tab.state === "error") chan?.ended();
  });
  // Неактивная вкладка и заблокированное хранилище — только просмотр
  $effect(() => {
    if (rfb) rfb.viewOnly = !active || app.phase !== "unlocked";
  });
  onDestroy(stop);
</script>

<div class="wrap" class:hidden={!active}>
  <div class="screen" bind:this={screen}></div>
  {#if tab.state === "connected" && !needPw}
    <div class="bar">
      <span class="warn" title="VNC не шифрует изображение и ввод — используйте только в доверенной сети или через SSH-туннель"
        >Без шифрования</span
      >
      <button class="icon-btn" title="Вставить текст из буфера обмена на сервер" onclick={paste}><Icon name="copy" size={13} /></button>
    </div>
  {/if}
  {#if needPw}
    <form class="center" onsubmit={(e) => (e.preventDefault(), submitPw())}>
      <Icon name="lock" size={24} />
      <div>Сервер VNC запрашивает {needPw.includes("username") ? "логин и пароль" : "пароль"}</div>
      {#if needPw.includes("username")}<input bind:value={pwUser} placeholder="Логин" />{/if}
      <!-- svelte-ignore a11y_autofocus -->
      <input type="password" bind:value={pwValue} placeholder="Пароль" autofocus />
      <button class="btn primary" disabled={!pwValue}>Войти</button>
    </form>
  {:else if tab.state === "connecting"}
    <div class="center muted">
      <div class="spinner"></div>
      Подключение к {tab.title}…
    </div>
  {:else if tab.state === "closed" || tab.state === "error"}
    <div class="center">
      <Icon name={tab.state === "error" || vncError ? "alert" : "plug"} size={28} />
      <div class="msg selectable" class:err={tab.state === "error" || !!vncError}>
        {vncError || (tab.state === "error" ? tab.message || "Ошибка подключения" : "Сеанс VNC завершён")}
      </div>
      <button class="btn primary" onclick={() => app.reconnect(tab.key)}>
        <Icon name="reconnect" size={14} /> Переподключиться
      </button>
    </div>
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
  .screen {
    position: absolute;
    inset: 0;
  }
  .bar {
    position: absolute;
    top: 8px;
    right: 18px;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 2px 4px 2px 8px;
    font-size: 11px;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 12px;
    opacity: 0.6;
    transition: opacity 0.15s;
  }
  .bar:hover {
    opacity: 1;
  }
  .warn {
    color: var(--warn);
  }
  .center {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    text-align: center;
    padding: 20px;
    background: #0d0f13;
  }
  .center input {
    max-width: 260px;
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
