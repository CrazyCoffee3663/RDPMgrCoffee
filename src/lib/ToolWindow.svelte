<script lang="ts">
  // Отдельное окно инструмента (ping, tracert, проверка порта…) в стиле приложения.
  // Параметры — в адресе окна (?tool=…&item=…&cmd=…&args=…); логин и пароль подставляет Rust.
  import { onMount, tick } from "svelte";
  import { api, errText, makeChannel, inTauri } from "./api";
  import { app, applyTheme } from "./state.svelte";
  import WindowControls from "./WindowControls.svelte";
  import Icon from "./Icon.svelte";

  const q = new URLSearchParams(location.search);
  const name = q.get("tool") ?? "Инструмент";
  const title = q.get("title") ?? "";
  const itemId = q.get("item") ?? "";
  const cmd = q.get("cmd") ?? "";
  const args = q.get("args") ?? "";

  let out = $state("");
  let running = $state(false);
  let status = $state("");
  let outEl = $state<HTMLPreElement>();
  let follow = true;
  const MAX = 400_000;

  async function append(s: string) {
    out += s.replace(/\r\n/g, "\n").replace(/\r/g, "");
    if (out.length > MAX) out = out.slice(out.length - MAX * 0.8);
    if (follow) {
      await tick();
      outEl?.scrollTo({ top: outEl.scrollHeight });
    }
  }

  async function start() {
    if (running) return;
    running = true;
    status = "Выполняется…";
    if (out) await append("\n──────── перезапуск ────────\n\n");
    const ch = makeChannel<string>();
    ch.onmessage = (s) => append(s);
    try {
      const code = await api.toolExec(itemId, cmd, args, app.settings.toolPasswordClipboard, ch);
      status = code === null ? "Остановлено" : code === 0 ? "Завершено" : `Завершено с кодом ${code}`;
    } catch (e) {
      const t = errText(e);
      status = "Ошибка";
      await append(
        `\n${t === "WINBOX_MISSING" ? "WinBox не установлен — установите его в «Настройки → Расширенные»." : t}\n`,
      );
    } finally {
      running = false;
    }
  }

  function stop() {
    api.toolStop().catch(() => {});
  }

  async function copy() {
    const sel = window.getSelection()?.toString();
    await navigator.clipboard.writeText(sel || out).catch(() => api.clipboardSet(sel || out));
    status = sel ? "Выделенное скопировано" : "Вывод скопирован";
  }

  function onScroll() {
    if (!outEl) return;
    follow = outEl.scrollTop + outEl.clientHeight >= outEl.scrollHeight - 8;
  }

  function onKey(e: KeyboardEvent) {
    if (e.ctrlKey && e.key.toLowerCase() === "c" && running && !window.getSelection()?.toString()) {
      e.preventDefault();
      stop();
    } else if (e.key === "Enter" && !running) {
      start();
    } else if (e.key === "Escape" && inTauri) {
      import("@tauri-apps/api/window").then((m) => m.getCurrentWindow().close());
    }
  }

  onMount(() => {
    applyTheme(app.settings.theme);
    document.title = `${name} — ${title}`;
    start();
  });
</script>

<svelte:window onkeydown={onKey} />

<div class="win">
  <div class="bar" data-tauri-drag-region>
    <span class="ico"><Icon name="terminal" size={14} /></span>
    <b data-tauri-drag-region>{name}</b>
    <span class="t faint" data-tauri-drag-region>{title}</span>
    <span class="sp" data-tauri-drag-region></span>
    {#if running}
      <button class="btn sm" onclick={stop} title="Остановить (Ctrl+C)"><span class="sq"></span> Стоп</button>
    {:else}
      <button class="btn sm" onclick={start} title="Запустить снова (Enter)"><Icon name="refresh" size={13} /> Ещё раз</button>
    {/if}
    <button class="icon-btn" onclick={copy} title="Копировать вывод или выделенное"><Icon name="copy" size={14} /></button>
    <button class="icon-btn" onclick={() => (out = "")} title="Очистить"><Icon name="trash" size={14} /></button>
    <WindowControls />
  </div>
  <pre class="out selectable" bind:this={outEl} onscroll={onScroll}>{out}</pre>
  <div class="foot">
    <span class="st" class:run={running}></span>
    <span>{status}</span>
    <span class="sp"></span>
    <span class="faint mono">{cmd} {args}</span>
  </div>
</div>

<style>
  .win {
    position: fixed;
    inset: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 38px;
    padding-left: 12px;
    background: var(--bg-panel);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .ico {
    display: inline-flex;
    color: var(--accent);
  }
  .t {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .sp {
    flex: 1;
    align-self: stretch;
  }
  .btn.sm {
    padding: 3px 10px;
    font-size: 12px;
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .sq {
    width: 8px;
    height: 8px;
    border-radius: 1px;
    background: var(--danger);
  }
  .out {
    flex: 1;
    margin: 0;
    padding: 10px 14px;
    overflow: auto;
    font-family: var(--mono);
    font-size: 12.5px;
    line-height: 1.45;
    white-space: pre-wrap;
    word-break: break-word;
    background: var(--term-bg);
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 12px;
    font-size: 11.5px;
    color: var(--text-muted);
    border-top: 1px solid var(--border);
    background: var(--bg-panel);
  }
  .foot .mono {
    font-family: var(--mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 60%;
  }
  .st {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--text-faint);
  }
  .st.run {
    background: var(--ok);
    animation: pulse 1.2s infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
</style>
