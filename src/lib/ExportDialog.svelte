<script lang="ts">
  // Экспорт: без логинов и паролей (открытый .json) или полный, зашифрованный паролем файла (.rdpexport).
  import { errText, parseConn } from "./api";
  import { app } from "./state.svelte";
  import { exportPlain, exportEncrypted } from "./exporter";
  import Icon from "./Icon.svelte";

  let { onclose }: { onclose: () => void } = $props();
  const req = app.exportReq!;
  const count = req.items.filter((i) => parseConn(i) && !app.gatewayIds.has(i.id)).length;

  let mode = $state<"plain" | "encrypted">("plain");
  let pw = $state("");
  let pw2 = $state("");
  let busy = $state(false);
  let error = $state("");

  async function run() {
    error = "";
    busy = true;
    try {
      if (mode === "plain") await exportPlain(req.items, req.label, req.base);
      else {
        if (pw !== pw2) throw "Пароли не совпадают";
        await exportEncrypted(req.items, req.label, req.base, pw);
      }
      onclose();
    } catch (e) {
      error = errText(e);
    } finally {
      pw = pw2 = "";
      busy = false;
    }
  }
</script>

<div class="backdrop" role="presentation" onmousedown={(e) => e.target === e.currentTarget && !busy && onclose()}>
  <div class="dialog" role="dialog" aria-label="Экспорт подключений">
    <header>
      <span>Экспорт: {req.label}</span>
      <button class="icon-btn" onclick={onclose} disabled={busy}><Icon name="x" /></button>
    </header>
    <div class="body">
      <p class="muted">Подключений: <b>{count}</b>. Файл сохранится в «Загрузки».</p>

      <label class="opt" class:on={mode === "plain"}>
        <input type="radio" bind:group={mode} value="plain" />
        <span>
          <b>Без логинов и паролей</b> (.json)
          <small>Адреса, порты, домены, параметры, RD-шлюзы и папки. Логины, пароли и заметки не выгружаются — файл можно
            передавать коллегам.</small>
        </span>
      </label>
      <label class="opt" class:on={mode === "encrypted"}>
        <input type="radio" bind:group={mode} value="encrypted" />
        <span>
          <b>Полный, зашифрованный</b> (.rdpexport)
          <small>Всё, включая логины, пароли и заметки, — для переноса в другое хранилище. Файл зашифрован паролем,
            который вы зададите (AES-256, ключ через Argon2id). Без этого пароля файл не прочитать.</small>
        </span>
      </label>

      {#if mode === "encrypted"}
        <div class="pw">
          <input type="password" bind:value={pw} placeholder="Пароль файла (не короче 12 символов)" autocomplete="new-password" />
          <input type="password" bind:value={pw2} placeholder="Повторите пароль" autocomplete="new-password"
            onkeydown={(e) => e.key === "Enter" && pw.length >= 12 && pw === pw2 && run()} />
          <small class="faint">Не храните пароль рядом с файлом. Удалите файл после переноса.</small>
        </div>
      {/if}

      {#if error}<p class="err selectable">{error}</p>{/if}
      <div class="actions">
        <button class="btn" onclick={onclose} disabled={busy}>Отмена</button>
        <button class="btn primary" onclick={run}
          disabled={busy || (mode === "encrypted" && (pw.length < 12 || pw !== pw2))}>
          <Icon name="download" size={14} />
          {busy ? "Экспорт…" : "Экспортировать"}
        </button>
      </div>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 120;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.45);
    backdrop-filter: blur(6px);
  }
  .dialog {
    width: min(520px, calc(100vw - 32px));
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    box-shadow: var(--shadow);
    color: var(--text);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 14px;
    border-bottom: 1px solid var(--border);
    font-weight: 600;
  }
  .body {
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .opt {
    display: flex;
    gap: 10px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    cursor: pointer;
  }
  .opt.on {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .opt span {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .opt small {
    color: var(--text-muted);
    line-height: 1.4;
  }
  .opt input {
    margin-top: 3px;
  }
  .pw {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .err {
    color: var(--danger);
    margin: 0;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
