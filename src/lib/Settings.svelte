<script lang="ts">
  import { app, applyTheme, findTheme } from "./state.svelte";
  import { PRESETS, newThemeId, type CustomTheme } from "./themes";
  import { createRootFolder, migrateToRoot, needsMigration, tidyGateways } from "./actions";
  import Icon from "./Icon.svelte";

  let { onclose }: { onclose: () => void } = $props();

  const s = app.settings;
  function changed() {
    app.saveSettings();
  }

  // Рабочая папка: выбор из существующих папок верхнего уровня или новое имя
  const topFolders = $derived(
    app.view.folders.map((f) => f.name).filter((n) => !n.includes("/")).sort((a, b) => a.localeCompare(b, "ru")),
  );
  let rootChoice = $state(s.rootFolder ? (app.view.folders.some((f) => f.name === s.rootFolder) ? s.rootFolder : "__new") : "");
  let newRoot = $state(s.rootFolder && !app.view.folders.some((f) => f.name === s.rootFolder) ? s.rootFolder : "Подключения");

  // ---- Темы ----
  const STOCK: [string, string, string, string, string][] = [
    ["system", "Как в системе", "linear-gradient(135deg,#f5f6f8 50%,#15171c 50%)", "#ffffff", "#4f8cff"],
    ["dark", "Тёмная", "#15171c", "#1c1f26", "#4f8cff"],
    ["light", "Светлая", "#f5f6f8", "#ffffff", "#2563eb"],
  ];
  const cur = $derived(findTheme(s.theme));
  let draft = $state<CustomTheme | null>(null);
  const isCustom = (id: string) => s.customThemes.some((t) => t.id === id);

  function pick(id: string) {
    draft = null;
    s.theme = id;
    changed();
  }
  /** Открыть редактор: своя тема — правим её, готовая — правим копию */
  function edit(t: CustomTheme | null | undefined) {
    const base: CustomTheme = t ?? findTheme("midnight")!;
    draft =
      t && isCustom(t.id)
        ? { ...t }
        : { ...base, id: newThemeId(), name: t ? `${t.name} (моя)` : "Моя тема" };
  }
  // Предпросмотр на лету
  $effect(() => {
    if (draft) applyTheme(s.theme, { ...draft });
  });
  function cancelEdit() {
    draft = null;
    applyTheme(s.theme);
  }
  function saveDraft() {
    if (!draft) return;
    const t = { ...draft, name: draft.name.trim() };
    s.customThemes = isCustom(t.id) ? s.customThemes.map((x) => (x.id === t.id ? t : x)) : [...s.customThemes, t];
    draft = null;
    pick(t.id);
  }
  function removeDraft() {
    if (!draft) return;
    const id = draft.id;
    s.customThemes = s.customThemes.filter((x) => x.id !== id);
    draft = null;
    pick(s.theme === id ? "dark" : s.theme);
  }

  async function applyRoot() {
    s.rootFolder = rootChoice === "__new" ? newRoot.trim() : rootChoice;
    changed();
    if (s.rootFolder) await createRootFolder();
  }
</script>

<div class="backdrop" role="presentation" onmousedown={(e) => e.target === e.currentTarget && (cancelEdit(), onclose())}>
  <div class="dialog" role="dialog" aria-label="Настройки">
    <header>
      <span>Настройки</span>
      <button class="icon-btn" onclick={() => (cancelEdit(), onclose())}><Icon name="x" /></button>
    </header>
    <div class="body">
      <div class="sect">Рабочая папка Bitwarden</div>
      <p class="hint">
        Менеджер показывает только записи из этой папки и вложенных в неё. Остальное хранилище (личные логины, карты
        и т.п.) не попадает в дерево, поиск и списки выбора. Новые записи создаются внутри неё.
      </p>
      <div class="row">
        <span>Папка</span>
        <select bind:value={rootChoice}>
          <option value="">Всё хранилище (без изоляции)</option>
          {#each topFolders as n}<option value={n}>{n}</option>{/each}
          <option value="__new">Создать новую…</option>
        </select>
      </div>
      {#if rootChoice === "__new"}
        <label class="row"><span>Имя новой папки</span><input bind:value={newRoot} /></label>
      {/if}
      <div class="row">
        <span class="faint">Сейчас: {s.rootFolder || "всё хранилище"}</span>
        <button
          class="btn primary"
          onclick={applyRoot}
          disabled={(rootChoice === "__new" ? newRoot.trim() : rootChoice) === s.rootFolder || (rootChoice === "__new" && !newRoot.trim())}
          >Применить</button
        >
      </div>
      {#if needsMigration()}
        <div class="migrate">
          <Icon name="alert" size={16} />
          <span>
            Вне рабочей папки найдено записей: <b>{needsMigration()}</b>. Их можно перенести вместе с
            учётками и шлюзами, на которые они ссылаются.
          </span>
          <button class="btn" onclick={() => (onclose(), migrateToRoot())}>Перенести</button>
        </div>
      {/if}

      <div class="sect">RD-шлюзы</div>
      <div class="acc">
        <span class="hint">Объединить одинаковые шлюзы и сложить все в папку «RDG» (скрыта в дереве).</span>
        <button class="btn" onclick={() => (onclose(), tidyGateways())}>Навести порядок</button>
      </div>

      <div class="sect">Импорт</div>
      <div class="acc">
        <span class="hint">Подключения, папки, пароли и RD-шлюзы из файла confCons.xml.</span>
        <button class="btn" onclick={() => (onclose(), (app.importOpen = true))}>
          <Icon name="download" size={14} /> Из mRemoteNG…
        </button>
      </div>

      <div class="sect">Внешний вид</div>
      <div class="themes">
        {#each STOCK as [id, label, bg, panel, accent]}
          <button class="sw" class:on={s.theme === id} onclick={() => pick(id)}>
            <span class="pv" style="background:{bg}"><i style="background:{panel}"></i><b style="background:{accent}"></b></span>
            {label}
          </button>
        {/each}
        {#each [...PRESETS, ...s.customThemes] as t (t.id)}
          <button class="sw" class:on={s.theme === t.id} onclick={() => pick(t.id)} ondblclick={() => edit(t)} title="Двойной клик — изменить">
            <span class="pv" style="background:{t.gradient ? `linear-gradient(${t.angle}deg, ${t.g1}, ${t.g2})` : t.bg}"
              ><i style="background:{t.panel}"></i><b style="background:{t.accent}"></b></span>
            {t.name}
          </button>
        {/each}
        <button class="sw add" onclick={() => edit(null)} title="Новая тема"><span class="pv"><Icon name="plus" size={16} /></span>Своя</button>
      </div>
      {#if !draft && cur}
        <div class="row">
          <span class="faint">{isCustom(cur.id) ? "Своя тема" : "Готовая тема — изменения сохранятся как новая"}</span>
          <button class="btn" onclick={() => edit(cur)}><Icon name="edit" size={14} /> Настроить</button>
        </div>
      {/if}

      {#if draft}
        <div class="editor">
          <label class="row"><span>Название</span><input bind:value={draft.name} /></label>
          <label class="row">
            <span>Основа</span>
            <select bind:value={draft.base}>
              <option value="dark">Тёмная</option>
              <option value="light">Светлая</option>
            </select>
          </label>
          <div class="colors">
            <label><input type="color" bind:value={draft.bg} /> Фон</label>
            <label><input type="color" bind:value={draft.panel} /> Панели</label>
            <label><input type="color" bind:value={draft.text} /> Текст</label>
            <label><input type="color" bind:value={draft.accent} /> Акцент</label>
          </div>
          <label class="chk"><input type="checkbox" bind:checked={draft.gradient} /> Градиент фона</label>
          {#if draft.gradient}
            <div class="colors">
              <label><input type="color" bind:value={draft.g1} /> Начало</label>
              <label><input type="color" bind:value={draft.g2} /> Конец</label>
              <label class="angle">
                <input type="range" min="0" max="360" step="5" bind:value={draft.angle} />
                {draft.angle}°
              </label>
            </div>
          {/if}
          <div class="eactions">
            {#if isCustom(draft.id)}
              <button class="btn danger" onclick={removeDraft}><Icon name="trash" size={14} /> Удалить</button>
            {/if}
            <span class="sp"></span>
            <button class="btn" onclick={cancelEdit}>Отмена</button>
            <button class="btn primary" onclick={saveDraft} disabled={!draft.name.trim()}>Сохранить</button>
          </div>
        </div>
      {/if}

      <details class="adv">
        <summary>Расширенные</summary>
        <div class="sect">Терминал и панели</div>
        <label class="row">
          <span>Размер шрифта терминала</span>
          <input type="number" min="8" max="32" bind:value={s.termFontSize} onchange={changed} />
        </label>
        <label class="row">
          <span>Шрифт терминала</span>
          <input bind:value={s.termFontFamily} onchange={changed} />
        </label>
        <label class="row">
          <span>Ширина панели подключений</span>
          <input type="number" min="220" max="600" step="10" bind:value={s.sidebarWidth} onchange={changed} />
        </label>

        <div class="sect">Безопасность</div>
        <label class="row">
          <span>Блокировать хранилище при бездействии, мин <small>(0 — никогда)</small></span>
          <input type="number" min="0" max="1440" bind:value={s.autoLockMinutes} onchange={changed} />
        </label>
        <label class="row">
          <span>Очищать скопированный пароль через, сек <small>(0 — не очищать)</small></span>
          <input type="number" min="0" max="600" bind:value={s.clipboardClearSeconds} onchange={changed} />
        </label>

        <div class="sect">Синхронизация</div>
        <label class="row">
          <span>Проверять изменения на сервере каждые, мин</span>
          <input type="number" min="1" max="120" bind:value={s.syncMinutes} onchange={changed} />
        </label>
      </details>

      <div class="sect">Учётная запись</div>
      <div class="acc">
        <div>
          <div class="selectable">{app.status.email}</div>
          <div class="faint selectable">{app.status.server}</div>
        </div>
        <button class="btn danger" onclick={() => (onclose(), app.logout())}>
          <Icon name="logout" size={14} /> Выйти
        </button>
      </div>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 100;
    background: rgba(10, 12, 18, 0.45);
    display: grid;
    place-items: center;
  }
  .dialog {
    width: min(560px, 94vw);
    max-height: 86vh;
    display: flex;
    flex-direction: column;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 12px;
    box-shadow: var(--shadow);
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 12px 12px 12px 20px;
    font-size: 15px;
    font-weight: 600;
    border-bottom: 1px solid var(--border);
  }
  .body {
    overflow: auto;
    padding: 4px 20px 20px;
  }
  .sect {
    margin: 18px 0 8px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-faint);
  }
  .row {
    display: grid;
    grid-template-columns: 1fr 200px;
    gap: 12px;
    align-items: center;
    padding: 5px 0;
  }
  small {
    color: var(--text-faint);
  }
  .hint {
    margin: 0 0 8px;
    color: var(--text-muted);
    line-height: 1.5;
  }
  .migrate {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 8px;
    padding: 10px 12px;
    border-radius: var(--radius);
    background: var(--accent-soft);
    line-height: 1.4;
  }
  .themes {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(96px, 1fr));
    gap: 8px;
    margin-bottom: 8px;
  }
  .sw {
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 6px;
    font-size: 12px;
    color: var(--text-muted);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    text-align: center;
  }
  .sw:hover {
    border-color: var(--border-strong);
  }
  .sw.on {
    border-color: var(--accent);
    color: var(--text);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }
  .pv {
    position: relative;
    display: grid;
    place-items: center;
    height: 42px;
    border-radius: 5px;
    overflow: hidden;
    border: 1px solid var(--border);
    color: var(--text-faint);
  }
  .pv i {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: 30%;
    opacity: 0.85;
  }
  .pv b {
    position: absolute;
    right: 8px;
    bottom: 8px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
  }
  .editor {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    margin-bottom: 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--bg-sunken);
  }
  .colors {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 16px;
    padding: 4px 0;
  }
  .colors label {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  input[type="color"] {
    width: 34px;
    height: 24px;
    padding: 0;
    border: 1px solid var(--border-strong);
    border-radius: 4px;
    background: none;
    cursor: pointer;
  }
  .angle input {
    width: 120px;
    accent-color: var(--accent);
  }
  .chk {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .eactions {
    display: flex;
    gap: 8px;
    margin-top: 4px;
  }
  .sp {
    flex: 1;
  }
  .adv {
    margin-top: 14px;
    border-top: 1px solid var(--border);
  }
  .adv summary {
    padding: 12px 0 4px;
    font-weight: 600;
    cursor: pointer;
    color: var(--text-muted);
  }
  .adv summary:hover {
    color: var(--text);
  }
  .acc {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
</style>
