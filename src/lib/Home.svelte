<script lang="ts">
  import { parseConn, PROTOCOL_LABEL, type Item } from "./api";
  import { app } from "./state.svelte";
  import { menu } from "./dialogs.svelte";
  import { itemMenu, newItem, migrateToRoot, needsMigration } from "./actions";
  import Icon from "./Icon.svelte";

  const favorites = $derived(app.connections.filter((i) => i.favorite).sort((a, b) => a.name.localeCompare(b.name, "ru")));
  const recent = $derived(app.recent.map((id) => app.item(id)).filter((i): i is Item => !!i && !!parseConn(i) && app.inScope(i)).slice(0, 8));
  const stats = $derived.by(() => {
    const s: Record<string, number> = {};
    for (const i of app.connections) {
      const p = parseConn(i)!.protocol;
      s[p] = (s[p] ?? 0) + 1;
    }
    return s;
  });
</script>

{#snippet card(it: Item)}
  {@const c = parseConn(it)!}
  <button
    class="card proto-{c.protocol}"
    onclick={() => app.connect(it)}
    oncontextmenu={(e) => menu.show(e, itemMenu(it))}
    title="Подключиться"
  >
    <span class="badge">{PROTOCOL_LABEL[c.protocol]}</span>
    <span class="cn">{it.name}</span>
    <span class="ch">{c.host}{c.port ? ":" + c.port : ""}</span>
  </button>
{/snippet}

<div class="home">
  <div class="inner">
    <h1>Добро пожаловать</h1>
    <p class="muted">
      Двойной клик по подключению в дереве — открыть его во вкладке. Быстрое подключение без сохранения — в строке
      сверху.
    </p>

    {#if !app.root}
      <div class="note">
        <Icon name="folder" size={16} />
        <span>
          Сейчас показано всё хранилище Bitwarden. Задайте рабочую папку, чтобы видеть только подключения и связанные с
          ними учётки и шлюзы.
        </span>
        <button class="btn" onclick={() => (app.settingsOpen = true)}>Настроить</button>
      </div>
    {:else if needsMigration()}
      <div class="note">
        <Icon name="alert" size={16} />
        <span>Вне рабочей папки «{app.root}» есть записи для переноса: {needsMigration()}.</span>
        <button class="btn" onclick={migrateToRoot}>Перенести</button>
      </div>
    {/if}

    <div class="stats">
      {#each Object.entries(stats) as [p, n]}
        <div class="stat proto-{p}"><b>{n}</b><span>{PROTOCOL_LABEL[p as keyof typeof PROTOCOL_LABEL]}</span></div>
      {/each}
      <div class="stat"><b>{app.logins.length}</b><span>логинов</span></div>
    </div>

    {#if favorites.length}
      <h2><Icon name="star" size={14} /> Избранное</h2>
      <div class="grid">{#each favorites as it (it.id)}{@render card(it)}{/each}</div>
    {/if}

    {#if recent.length}
      <h2><Icon name="refresh" size={14} /> Недавние</h2>
      <div class="grid">{#each recent as it (it.id)}{@render card(it)}{/each}</div>
    {/if}

    {#if !app.connections.length}
      <div class="first">
        <button class="btn primary" onclick={() => newItem("conn")}><Icon name="plus" size={14} /> Создать первое подключение</button>
        <button class="btn" onclick={() => (app.importOpen = true)}><Icon name="download" size={14} /> Импорт из mRemoteNG</button>
      </div>
    {/if}

    <h2><Icon name="zap" size={14} /> Горячие клавиши</h2>
    <div class="keys">
      <span><kbd>Ctrl</kbd>+<kbd>F</kbd></span><span>поиск</span>
      <span><kbd>Ctrl</kbd>+<kbd>K</kbd></span><span>быстрое подключение</span>
      <span><kbd>Ctrl</kbd>+<kbd>N</kbd></span><span>новое подключение</span>
      <span><kbd>Ctrl</kbd>+<kbd>Tab</kbd></span><span>следующая вкладка</span>
      <span><kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>W</kbd></span><span>закрыть вкладку</span>
      <span><kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>C</kbd> / <kbd>V</kbd></span><span>копировать / вставить в терминале</span>
      <span><kbd>Ctrl</kbd>+<kbd>L</kbd></span><span>заблокировать хранилище</span>
    </div>
  </div>
</div>

<style>
  .home {
    position: absolute;
    inset: 0;
    overflow: auto;
  }
  .inner {
    max-width: 860px;
    margin: 0 auto;
    padding: 36px 28px;
  }
  h1 {
    margin: 0 0 6px;
    font-size: 22px;
    font-weight: 600;
  }
  h2 {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 28px 0 10px;
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-faint);
  }
  p {
    margin: 0;
    line-height: 1.5;
  }
  .note {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 18px;
    padding: 10px 14px;
    border: 1px solid var(--border);
    border-left: 3px solid var(--accent);
    border-radius: var(--radius);
    background: var(--bg-panel);
    line-height: 1.45;
  }
  .note span {
    flex: 1;
  }
  .stats {
    display: flex;
    gap: 10px;
    margin-top: 20px;
    flex-wrap: wrap;
  }
  .stat {
    display: flex;
    align-items: baseline;
    gap: 6px;
    padding: 10px 16px;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .stat b {
    font-size: 20px;
    color: var(--pc, var(--text));
  }
  .stat span {
    color: var(--text-muted);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 10px;
  }
  .card {
    display: grid;
    grid-template-columns: auto 1fr;
    grid-template-rows: auto auto;
    gap: 2px 10px;
    align-items: center;
    padding: 12px;
    text-align: left;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    transition: border-color 0.12s, transform 0.12s;
  }
  .card:hover {
    border-color: var(--pc);
    transform: translateY(-1px);
  }
  .badge {
    grid-row: 1 / 3;
    font-size: 10px;
    font-weight: 700;
    width: 40px;
    padding: 8px 0;
    text-align: center;
    border-radius: 6px;
    color: var(--pc);
    background: color-mix(in srgb, var(--pc) 15%, transparent);
  }
  .cn {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ch {
    font-size: 12px;
    color: var(--text-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .first {
    margin-top: 24px;
  }
  .keys {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 8px 18px;
    color: var(--text-muted);
    align-items: center;
  }
</style>
