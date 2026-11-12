<script lang="ts">
  import { app, applyTheme, findTheme } from "./state.svelte";
  import { PRESETS, newThemeId, type CustomTheme } from "./themes";
  import { createRootFolder, migrateToRoot, needsMigration, tidyGateways, checkWinbox, installWinbox, newerVersion } from "./actions";
  import { DEFAULT_TOOLS, type Tool } from "./state.svelte";
  import { PROTOCOLS, PROTOCOL_LABEL } from "./api";
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

  // ---- Внешние инструменты и WinBox ----
  let wb = $state<{ installed: string | null; latest: string | null; path: string } | null>(null);
  let wbBusy = $state(false);
  async function wbCheck() {
    wbBusy = true;
    wb = (await checkWinbox(true)) ?? null;
    wbBusy = false;
  }
  async function wbInstall() {
    if (!wb?.latest) return;
    wbBusy = true;
    if (await installWinbox(wb.latest, !!wb.installed)) await wbCheck();
    wbBusy = false;
  }
  function addTool() {
    s.tools = [...s.tools, { id: "t" + Date.now().toString(36), name: "Новый инструмент", command: "", args: "%HOST%", protocols: [] }];
    changed();
  }
  function delTool(t: Tool) {
    s.tools = s.tools.filter((x) => x.id !== t.id);
    changed();
  }
  function toggleProto(t: Tool, p: string) {
    t.protocols = t.protocols.includes(p) ? t.protocols.filter((x) => x !== p) : [...t.protocols, p];
    changed();
  }

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

        <label class="row">
          <span>Подпись с логином у подключений</span>
          <span><input type="checkbox" bind:checked={s.showLogin} onchange={changed} /></span>
        </label>

        <div class="sect">Индикатор доступности</div>
        <p class="hint">
          Точка у подключения: <b class="c-ok">зелёная</b> — подключено, <b class="c-warn">жёлтая</b> — хост отвечает,
          <b class="c-bad">красная</b> — нет ответа. Проверка — короткая попытка TCP-соединения на порт подключения, без входа.
        </p>
        <label class="row">
          <span>Проверять доступность хостов</span>
          <span><input type="checkbox" bind:checked={s.probeEnabled} onchange={changed} /></span>
        </label>
        {#if s.probeEnabled}
          <label class="row">
            <span>Интервал проверки, мин</span>
            <input type="number" min="0.5" max="60" step="0.5" bind:value={s.probeMinutes} onchange={changed} />
          </label>
          <label class="row">
            <span>Ожидание ответа, мс</span>
            <input type="number" min="200" max="10000" step="100" bind:value={s.probeTimeoutMs} onchange={changed} />
          </label>
          <label class="row">
            <span>Проверок одновременно</span>
            <input type="number" min="1" max="64" bind:value={s.probeParallel} onchange={changed} />
          </label>
          <label class="row">
            <span>Только подключения в раскрытых папках</span>
            <span><input type="checkbox" bind:checked={s.probeVisibleOnly} onchange={changed} /></span>
          </label>
          <label class="row">
            <span>Для подключений через RD-шлюз проверять шлюз <small>(иначе не проверять)</small></span>
            <span><input type="checkbox" bind:checked={s.probeViaGateway} onchange={changed} /></span>
          </label>
        {/if}

        <div class="sect">Внешние инструменты</div>
        <p class="hint">
          Появляются в меню подключения (правый клик). Подстановки: <code>%HOST%</code> <code>%PORT%</code>
          <code>%USER%</code> <code>%PASSWORD%</code> <code>%DOMAIN%</code> <code>%NAME%</code>. Программа:
          путь к exe, <code>winbox</code> — встроенный WinBox, <code>url</code> — открыть ссылку в браузере.
          <br />Пароль в командной строке виден другим программам вашего пользователя — подставляйте
          <code>%PASSWORD%</code> только там, где это нужно.
        </p>
        {#each s.tools as t (t.id)}
          <div class="tool">
            <input class="tn" bind:value={t.name} onchange={changed} placeholder="Название" />
            <input class="tc mono" bind:value={t.command} onchange={changed} placeholder="cmd / winbox / url / C:\путь\к.exe" />
            <button class="icon-btn" title="Удалить" onclick={() => delTool(t)}><Icon name="trash" size={14} /></button>
            <input class="ta mono" bind:value={t.args} onchange={changed} placeholder="Аргументы" />
            <div class="tp">
              {#each PROTOCOLS as p}
                <label class="chk"
                  ><input type="checkbox" checked={!t.protocols.length || t.protocols.includes(p)}
                    onchange={() => (t.protocols.length ? toggleProto(t, p) : ((t.protocols = PROTOCOLS.filter((x) => x !== p)), changed()))}
                  />{PROTOCOL_LABEL[p]}</label
                >
              {/each}
            </div>
          </div>
        {/each}
        <div class="row">
          <button class="btn" onclick={() => ((s.tools = DEFAULT_TOOLS.map((x) => ({ ...x }))), changed())}>Вернуть стандартные</button>
          <button class="btn" onclick={addTool}><Icon name="plus" size={14} /> Добавить</button>
        </div>

        <div class="sect">WinBox</div>
        <div class="row">
          <span>
            {#if wb}
              Установлен: <b>{wb.installed ?? "нет"}</b>, последняя: <b>{wb.latest ?? "не удалось узнать"}</b>
            {:else}
              Скачивается с download.mikrotik.com при первом запуске (с вашего согласия)
            {/if}
          </span>
          <span class="btns">
            <button class="btn" onclick={wbCheck} disabled={wbBusy}>Проверить</button>
            {#if wb?.latest && (!wb.installed || wb.installed === "?" || newerVersion(wb.latest, wb.installed))}
              <button class="btn primary" onclick={wbInstall} disabled={wbBusy}>{wb.installed ? "Обновить" : "Установить"}</button>
            {/if}
          </span>
        </div>
        <label class="row">
          <span>Проверять обновления WinBox <small>(раз в сутки, установка — только после подтверждения)</small></span>
          <span><input type="checkbox" bind:checked={s.winboxAutoCheck} onchange={changed} /></span>
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
  .c-ok {
    color: var(--ok);
  }
  .c-warn {
    color: var(--warn);
  }
  .c-bad {
    color: var(--danger);
  }
  .tool {
    display: grid;
    grid-template-columns: 1fr 1.4fr auto;
    gap: 6px;
    padding: 8px;
    margin-bottom: 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .ta,
  .tp {
    grid-column: 1 / 4;
  }
  .tp {
    display: flex;
    gap: 14px;
    font-size: 12px;
    color: var(--text-muted);
  }
  .btns {
    display: flex;
    gap: 6px;
    justify-content: flex-end;
  }
  code {
    font-family: var(--mono);
    font-size: 11.5px;
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
