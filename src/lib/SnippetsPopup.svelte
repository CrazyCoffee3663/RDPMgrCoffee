<script lang="ts">
  // Сниппеты команд: быстрый поиск и отправка в активный терминал (или во все SSH-вкладки).
  import { api, errText } from "./api";
  import { app, type Snippet } from "./state.svelte";
  import Icon from "./Icon.svelte";

  let { onclose }: { onclose: () => void } = $props();

  let q = $state("");
  let idx = $state(0);
  let toAll = $state(false);
  let edit = $state<Snippet | null>(null);
  let searchEl = $state<HTMLInputElement>();

  const list = $derived.by(() => {
    const s = q.trim().toLowerCase();
    const all = app.settings.snippets;
    return s ? all.filter((x) => x.name.toLowerCase().includes(s) || x.cmd.toLowerCase().includes(s)) : all;
  });
  /** Терминальные вкладки с живой сессией: текст понимают только SSH/Telnet/COM.
      VNC получил бы мусор в потоке RFB, RDP и веб-вкладки не принимают session_input. */
  const terms = $derived(
    app.tabs.filter((t) => ["ssh", "telnet", "serial"].includes(t.protocol) && t.sessionId !== null && t.state === "connected"),
  );
  const activeTerm = $derived(terms.find((t) => t.key === app.activeTab));

  $effect(() => {
    if (idx >= list.length) idx = Math.max(0, list.length - 1);
  });
  $effect(() => {
    searchEl?.focus();
  });

  async function send(sn: Snippet) {
    const targets = toAll ? terms : activeTerm ? [activeTerm] : [];
    if (!targets.length) return app.toast("Нет открытого терминала для отправки", true);
    const text = sn.cmd.replace(/\r?\n/g, "\r") + (sn.enter ? "\r" : "");
    try {
      for (const t of targets) await api.input(t.sessionId!, text);
      onclose();
    } catch (e) {
      app.toast(errText(e), true);
    }
  }

  function onKey(e: KeyboardEvent) {
    if (edit) return;
    if (e.key === "ArrowDown") (e.preventDefault(), (idx = Math.min(list.length - 1, idx + 1)));
    else if (e.key === "ArrowUp") (e.preventDefault(), (idx = Math.max(0, idx - 1)));
    else if (e.key === "Enter" && list[idx]) (e.preventDefault(), send(list[idx]));
    else if (e.key === "Escape") (e.preventDefault(), onclose());
  }

  function saveEdit() {
    if (!edit || !edit.name.trim() || !edit.cmd.trim()) return;
    const sn = { ...edit, name: edit.name.trim() };
    const all = app.settings.snippets;
    app.settings.snippets = all.some((x) => x.id === sn.id) ? all.map((x) => (x.id === sn.id ? sn : x)) : [...all, sn];
    app.saveSettings();
    edit = null;
  }
  function remove(sn: Snippet) {
    app.settings.snippets = app.settings.snippets.filter((x) => x.id !== sn.id);
    app.saveSettings();
    edit = null;
  }
</script>

<div class="backdrop" role="presentation" onmousedown={(e) => e.target === e.currentTarget && onclose()}>
  <div class="pop" role="dialog" aria-label="Сниппеты" tabindex="-1" onkeydown={onKey}>
    {#if edit}
      <div class="head"><b>{app.settings.snippets.some((x) => x.id === edit?.id) ? "Сниппет" : "Новый сниппет"}</b></div>
      <div class="form">
        <label><span>Название</span><input bind:value={edit.name} placeholder="Перезапуск nginx" /></label>
        <label>
          <span>Команда <small>(несколько строк — несколько команд)</small></span>
          <textarea class="mono" rows="4" bind:value={edit.cmd} placeholder="sudo systemctl restart nginx"></textarea>
        </label>
        <label class="chk"><input type="checkbox" bind:checked={edit.enter} /> Нажать Enter после вставки</label>
        <div class="acts">
          {#if app.settings.snippets.some((x) => x.id === edit?.id)}
            <button class="btn danger" onclick={() => edit && remove(edit)}><Icon name="trash" size={14} /></button>
          {/if}
          <span class="sp"></span>
          <button class="btn" onclick={() => (edit = null)}>Отмена</button>
          <button class="btn primary" onclick={saveEdit} disabled={!edit.name.trim() || !edit.cmd.trim()}>Сохранить</button>
        </div>
      </div>
    {:else}
      <div class="head">
        <Icon name="search" size={14} />
        <input bind:this={searchEl} bind:value={q} placeholder="Найти сниппет…  (↑↓ выбрать, Enter — отправить)" />
        <button class="icon-btn" title="Новый сниппет" onclick={() => (edit = { id: "s" + Date.now().toString(36), name: "", cmd: "", enter: true })}>
          <Icon name="plus" size={15} />
        </button>
      </div>
      <div class="list">
        {#each list as sn, i (sn.id)}
          <div
            class="it"
            class:on={i === idx}
            role="option"
            aria-selected={i === idx}
            tabindex="-1"
            onmouseenter={() => (idx = i)}
            onclick={() => send(sn)}
            onkeydown={() => {}}
          >
            <div class="nm">{sn.name}{#if !sn.enter}<small> · без Enter</small>{/if}</div>
            <div class="cmd mono">{sn.cmd}</div>
            <button class="icon-btn ed" title="Изменить" onclick={(e) => (e.stopPropagation(), (edit = { ...sn }))}>
              <Icon name="edit" size={13} />
            </button>
          </div>
        {:else}
          <div class="empty faint">
            {app.settings.snippets.length ? "Ничего не найдено" : "Сниппетов пока нет — добавьте кнопкой «+»"}
          </div>
        {/each}
      </div>
      <div class="foot">
        <label class="chk"><input type="checkbox" bind:checked={toAll} /> Во все SSH/Telnet-вкладки ({terms.length})</label>
        <span class="faint">{toAll ? "" : activeTerm ? `→ ${activeTerm.title}` : "нет активного терминала"}</span>
      </div>
    {/if}
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 110;
    background: rgba(10, 12, 18, 0.35);
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 12vh;
  }
  .pop {
    width: min(620px, 92vw);
    max-height: 70vh;
    display: flex;
    flex-direction: column;
    background: var(--bg-solid, var(--bg-panel));
    border: 1px solid var(--border);
    border-radius: 12px;
    box-shadow: var(--shadow);
    outline: none;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
    color: var(--text-faint);
  }
  .head input {
    border: 0;
    background: none;
    box-shadow: none !important;
    color: var(--text);
  }
  .list {
    overflow: auto;
    padding: 6px;
  }
  .it {
    position: relative;
    padding: 7px 36px 7px 10px;
    border-radius: var(--radius-sm);
    cursor: pointer;
  }
  .it.on {
    background: var(--bg-active);
  }
  .nm {
    font-weight: 600;
  }
  .nm small {
    font-weight: 400;
    color: var(--text-faint);
  }
  .cmd {
    margin-top: 2px;
    font-size: 12px;
    color: var(--text-muted);
    white-space: pre;
    overflow: hidden;
    text-overflow: ellipsis;
    max-height: 3.2em;
  }
  .ed {
    position: absolute;
    right: 6px;
    top: 6px;
    opacity: 0;
  }
  .it:hover .ed,
  .it.on .ed {
    opacity: 1;
  }
  .empty {
    padding: 20px;
    text-align: center;
  }
  .foot {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 12px;
    border-top: 1px solid var(--border);
    font-size: 12px;
  }
  .chk {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
  }
  .form label:not(.chk) {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .form small {
    color: var(--text-faint);
  }
  .acts {
    display: flex;
    gap: 8px;
  }
  .sp {
    flex: 1;
  }
  .mono {
    font-family: var(--mono);
  }
</style>
