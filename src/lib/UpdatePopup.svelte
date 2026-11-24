<script lang="ts">
  import { app } from "./state.svelte";
  import { updateApp, updateFreerdp, installWinbox } from "./actions";
  import Icon from "./Icon.svelte";

  type Row = {
    key: string;
    kind: "app" | "freerdp" | "winbox";
    text: string;
    label: string;
    busy: boolean;
    run: () => void;
  };

  const rows = $derived.by(() => {
    const out: Row[] = [];
    if (app.appUpdate && !app.updateDismissed.includes("app:" + app.appUpdate.latest))
      out.push({
        key: "app:" + app.appUpdate.latest,
        kind: "app",
        text: `Вышла версия приложения ${app.appUpdate.latest} (установлена ${app.appUpdate.current})`,
        label: "Обновить",
        busy: !!app.appUpdating,
        run: () => updateApp(),
      });
    if (app.freerdpUpdate && !app.updateDismissed.includes("freerdp:" + app.freerdpUpdate.latest))
      out.push({
        key: "freerdp:" + app.freerdpUpdate.latest,
        kind: "freerdp",
        text: `Вышел FreeRDP ${app.freerdpUpdate.latest} (встроен ${app.freerdpUpdate.installed}) — обновления закрывают уязвимости RDP-клиента`,
        label: "Обновить",
        busy: !!app.freerdpUpdating,
        run: () => updateFreerdp(),
      });
    if (app.winboxUpdate && !app.updateDismissed.includes("winbox:" + app.winboxUpdate.latest))
      out.push({
        key: "winbox:" + app.winboxUpdate.latest,
        kind: "winbox",
        text: `Доступна WinBox ${app.winboxUpdate.latest} (установлена ${app.winboxUpdate.installed})`,
        label: "Скачать",
        busy: false,
        run: () => {
          const u = app.winboxUpdate;
          if (u) installWinbox(u.latest, true);
        },
      });
    return out;
  });

  function goto(kind: Row["kind"]) {
    app.settingsFocus = kind;
    app.settingsOpen = true;
  }
</script>

<!-- В layout, а не поверх: нативное окно RDP перекрывает webview, поэтому карточка
     занимает место над статус-баром — область вкладки (и окно RDP) сужается сама -->
{#if rows.length}
  <div class="updpop" role="status">
    <div class="head">
      <Icon name="download" size={14} />
      <span>Доступны обновления</span>
      <span class="sp"></span>
      <button class="x" title="Скрыть до следующей новой версии" onclick={() => app.dismissUpdates(rows.map((r) => r.key))}>
        <Icon name="x" size={12} />
      </button>
    </div>
    {#each rows as r (r.key)}
      <div class="urow">
        <span class="txt selectable">{r.text}</span>
        <button class="btn primary" disabled={r.busy} onclick={r.run}>{r.label}</button>
        <button class="btn" onclick={() => goto(r.kind)}>К настройкам</button>
      </div>
    {/each}
  </div>
{/if}

<style>
  .updpop {
    flex-shrink: 0;
    margin: 0 10px 8px;
    padding: 8px 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-left: 3px solid var(--accent);
    border-radius: var(--radius);
    box-shadow: var(--shadow);
    animation: rise 0.18s ease-out;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 6px;
    font-weight: 600;
    color: var(--accent);
  }
  .sp {
    flex: 1;
  }
  .x {
    display: flex;
    padding: 4px;
    color: var(--text-faint);
  }
  .x:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .urow {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .txt {
    flex: 1;
    min-width: 0;
    color: var(--text-muted);
    line-height: 1.4;
  }
  .urow .btn {
    padding: 4px 12px;
  }
  @keyframes rise {
    from {
      transform: translateY(8px);
      opacity: 0;
    }
  }
</style>
