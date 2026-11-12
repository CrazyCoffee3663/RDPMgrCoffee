<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { Terminal, type ITheme } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import { WebglAddon } from "@xterm/addon-webgl";
  import { WebLinksAddon } from "@xterm/addon-web-links";
  import "@xterm/xterm/css/xterm.css";
  import { api, errText, makeChannel } from "./api";
  import { app, type Tab } from "./state.svelte";
  import Icon from "./Icon.svelte";
  import SftpPanel from "./SftpPanel.svelte";

  /** Панель файлов SFTP (только SSH) */
  let showFiles = $state(false);
  $effect(() => {
    void showFiles;
    setTimeout(() => doFit(), 30);
  });

  let { tab, active }: { tab: Tab; active: boolean } = $props();

  let el: HTMLDivElement;
  let term: Terminal;
  let fit: FitAddon;
  let ro: ResizeObserver;
  let startedGen = -1;

  const DARK: ITheme = {
    background: "#121419",
    foreground: "#dfe3ec",
    cursor: "#4f8cff",
    cursorAccent: "#121419",
    selectionBackground: "#35507f99",
    black: "#1c1f26",
    red: "#f06464",
    green: "#3ccf7e",
    yellow: "#e6b04a",
    blue: "#4f8cff",
    magenta: "#c678dd",
    cyan: "#38bdd8",
    white: "#d4d8e2",
    brightBlack: "#5c6577",
    brightRed: "#ff8080",
    brightGreen: "#6be39f",
    brightYellow: "#f5cb70",
    brightBlue: "#7aa9ff",
    brightMagenta: "#dd9ff0",
    brightCyan: "#6fd6ea",
    brightWhite: "#ffffff",
  };
  const LIGHT: ITheme = {
    background: "#ffffff",
    foreground: "#1b2130",
    cursor: "#2563eb",
    cursorAccent: "#ffffff",
    selectionBackground: "#2563eb33",
    black: "#1b2130",
    red: "#c92a2a",
    green: "#1f8a4c",
    yellow: "#a86b00",
    blue: "#2563eb",
    magenta: "#9c36b5",
    cyan: "#0b7fa1",
    white: "#c8ccd4",
    brightBlack: "#636c80",
    brightRed: "#e03131",
    brightGreen: "#2f9e44",
    brightYellow: "#c98a12",
    brightBlue: "#3b82f6",
    brightMagenta: "#ae3ec9",
    brightCyan: "#1098ad",
    brightWhite: "#ffffff",
  };

  function currentTheme(): ITheme {
    const base = document.documentElement.dataset.theme === "light" ? LIGHT : DARK;
    // Своя тема: фон, текст и курсор берём из CSS-переменных
    const cs = getComputedStyle(document.documentElement);
    const v = (n: string) => {
      const x = cs.getPropertyValue(n).trim();
      if (!x) return undefined;
      // color-mix → #rrggbb: элемент считает цвет, холст переводит в понятный xterm вид
      probe.style.color = x;
      ctx.clearRect(0, 0, 1, 1);
      ctx.fillStyle = getComputedStyle(probe).color;
      ctx.fillRect(0, 0, 1, 1);
      const [r, g, b] = ctx.getImageData(0, 0, 1, 1).data;
      return "#" + [r, g, b].map((n) => n.toString(16).padStart(2, "0")).join("");
    };
    if (!document.documentElement.style.getPropertyValue("--term-bg")) return base;
    const bg = v("--term-bg");
    const fg = v("--text");
    const ac = v("--accent");
    return { ...base, background: bg, foreground: fg, cursor: ac, cursorAccent: bg };
  }
  const probe = (() => {
    let p = document.getElementById("theme-probe");
    if (!p) {
      p = document.createElement("span");
      p.id = "theme-probe";
      p.style.display = "none";
      document.body.appendChild(p);
    }
    return p;
  })();
  const ctx = (() => {
    const c = document.createElement("canvas");
    c.width = c.height = 1;
    return c.getContext("2d", { willReadFrequently: true })!;
  })();

  function doFit() {
    if (!term || !el || el.clientWidth < 20 || el.clientHeight < 20) return;
    try {
      fit.fit();
    } catch {}
  }

  async function paste() {
    try {
      const text = await api.clipboardGet();
      if (text) term.paste(text);
    } catch (e) {
      app.toast(errText(e), true);
    }
  }

  function copySelection(): boolean {
    const s = term.getSelection();
    if (!s) return false;
    api.clipboardSet(s).catch(() => {});
    return true;
  }

  async function start() {
    startedGen = tab.gen;
    term.reset();
    doFit();
    const ch = makeChannel<ArrayBuffer>();
    ch.onmessage = (d) => term.write(new Uint8Array(d));
    try {
      const r = await api.open(
        {
          itemId: tab.itemId,
          quick: tab.quick?.address,
          quickProtocol: tab.quick?.protocol,
          quickUsername: tab.quick?.username,
          quickPassword: tab.quick?.password,
          cols: term.cols,
          rows: term.rows,
        },
        ch,
      );
      app.bindSession(tab.key, r.id);
      if (active) term.focus();
    } catch (e) {
      tab.state = "error";
      tab.message = errText(e);
      term.write(`\r\n\x1b[31m${tab.message.replace(/\n/g, "\r\n")}\x1b[0m\r\n`);
    }
  }

  onMount(() => {
    term = new Terminal({
      fontFamily: app.settings.termFontFamily,
      fontSize: app.settings.termFontSize,
      cursorBlink: true,
      scrollback: 10000,
      allowProposedApi: true,
      theme: currentTheme(),
      rightClickSelectsWord: false,
      macOptionIsMeta: true,
    });
    fit = new FitAddon();
    term.loadAddon(fit);
    term.loadAddon(new WebLinksAddon((_e, uri) => openUrl(uri).catch(() => {})));
    term.open(el);
    try {
      const gl = new WebglAddon();
      gl.onContextLoss(() => gl.dispose());
      term.loadAddon(gl);
    } catch {
      // без WebGL работает обычный DOM-рендерер
    }

    term.onData((d) => {
      if (tab.sessionId !== null && tab.state === "connected") api.input(tab.sessionId, d);
      else if ((tab.state === "closed" || tab.state === "error") && d === "\r") app.reconnect(tab.key);
    });
    term.onResize(({ cols, rows }) => {
      if (tab.sessionId !== null) api.resize(tab.sessionId, cols, rows);
    });
    term.attachCustomKeyEventHandler((e) => {
      if (e.type !== "keydown") return true;
      if (e.ctrlKey && e.shiftKey && e.code === "KeyC") {
        copySelection();
        return false;
      }
      if (e.ctrlKey && e.shiftKey && e.code === "KeyV") {
        paste();
        return false;
      }
      // Ctrl+Tab и Ctrl+Shift+W обрабатывает приложение
      if (e.ctrlKey && (e.key === "Tab" || (e.shiftKey && e.code === "KeyW"))) return false;
      // Ctrl+Shift+S — сниппеты, F11 — полный экран
      if ((e.ctrlKey && e.shiftKey && e.code === "KeyS") || e.code === "F11") return false;
      return true;
    });
    // Как в PuTTY: правая кнопка копирует выделение или вставляет
    el.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      if (!copySelection()) paste();
      else term.clearSelection();
    });

    let raf = 0;
    ro = new ResizeObserver(() => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(doFit);
    });
    ro.observe(el);

    const mo = new MutationObserver(() => (term.options.theme = currentTheme()));
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "data-theme-rev"] });
    return () => mo.disconnect();
  });

  onDestroy(() => {
    ro?.disconnect();
    term?.dispose();
  });

  // Старт и переподключение
  $effect(() => {
    if (term && tab.gen !== startedGen) start();
  });

  $effect(() => {
    if (active && term) {
      requestAnimationFrame(() => {
        doFit();
        term.focus();
      });
    }
  });

  $effect(() => {
    if (!term) return;
    term.options.fontSize = app.settings.termFontSize;
    term.options.fontFamily = app.settings.termFontFamily;
    doFit();
  });
</script>

<div class="wrap" class:hidden={!active}>
  <div class="body">
    <div class="term" bind:this={el}></div>
    {#if showFiles && tab.sessionId !== null}
      <SftpPanel sessionId={tab.sessionId} onclose={() => (showFiles = false)} />
    {/if}
    {#if tab.protocol === "ssh" && tab.state === "connected" && !showFiles}
      <button class="files-btn" title="Файлы (SFTP)" onclick={() => (showFiles = true)}><Icon name="folder" size={14} /> Файлы</button>
    {/if}
  </div>
  {#if tab.state === "closed" || tab.state === "error"}
    <div class="bar" class:err={tab.state === "error"}>
      <Icon name={tab.state === "error" ? "alert" : "plug"} size={15} />
      <span class="msg selectable">{tab.state === "error" ? tab.message || "Ошибка" : "Соединение закрыто"}</span>
      <button class="btn primary" onclick={() => app.reconnect(tab.key)}>
        <Icon name="reconnect" size={14} /> Переподключиться <kbd>Enter</kbd>
      </button>
    </div>
  {/if}
</div>

<style>
  .wrap {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    background: var(--term-bg);
  }
  .hidden {
    visibility: hidden;
    pointer-events: none;
  }
  .body {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .files-btn {
    position: absolute;
    top: 8px;
    right: 18px;
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 3px 9px;
    font-size: 12px;
    color: var(--text-muted);
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 12px;
    opacity: 0.55;
    transition: opacity 0.15s;
  }
  .files-btn:hover {
    opacity: 1;
    color: var(--text);
  }
  .term {
    flex: 1;
    min-width: 0;
    min-height: 0;
    padding: 6px 0 0 8px;
  }
  .term :global(.xterm) {
    height: 100%;
  }
  .term :global(.xterm-viewport) {
    background: transparent !important;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    background: var(--bg-panel);
    border-top: 1px solid var(--border);
    color: var(--text-muted);
  }
  .bar.err {
    color: var(--danger);
  }
  .msg {
    flex: 1;
    white-space: pre-wrap;
  }
  .bar kbd {
    background: transparent;
    color: inherit;
    border-color: rgba(255, 255, 255, 0.4);
  }
</style>
