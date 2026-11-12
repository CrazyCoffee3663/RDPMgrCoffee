<script lang="ts">
  // Обучающий тур: показывается при первом запуске, повторно — из настроек.
  import { app, applyTheme } from "./state.svelte";
  import { PRESETS } from "./themes";
  import { createRootFolder, migrateToRoot, needsMigration } from "./actions";
  import Icon from "./Icon.svelte";

  let { onclose }: { onclose: () => void } = $props();

  /** kind — интерактивный шаг: выбор темы или рабочей папки Bitwarden */
  type Step = { icon: string; title: string; lead: string; points: [string, string][]; kind?: "theme" | "workspace" };
  const ALL: Step[] = [
    {
      icon: "plug",
      title: "Добро пожаловать в RDPMgrCoffee",
      lead: "Менеджер подключений RDP, SSH, Telnet, COM и MikroTik с зашифрованным хранилищем. Тур займёт пару минут — его можно пропустить и пройти позже в настройках.",
      points: [
        ["←  →", "листать шаги"],
        ["Esc", "закрыть тур"],
      ],
    },
    {
      icon: "edit",
      title: "Выберите тему",
      lead: "Оформление можно поменять в любой момент в настройках, там же — редактор своей темы.",
      points: [],
      kind: "theme",
    },
    {
      icon: "lock",
      title: "Хранилище и безопасность",
      lead: "Подключения и пароли хранятся либо в Bitwarden / Vaultwarden, либо локально — в файле на этом компьютере. В обоих случаях всё зашифровано, расшифровка — только у вас.",
      points: [
        ["Ctrl+L", "заблокировать хранилище; при простое оно блокируется само"],
        ["Пароли", "никогда не попадают в командную строку программ и в журнал буфера обмена Windows"],
        ["Буфер", "скопированный пароль стирается автоматически"],
      ],
    },
    {
      icon: "folderOpen",
      title: "Рабочая папка Bitwarden",
      lead: "Менеджер может работать только с одной папкой хранилища: в дереве, поиске и списках будут лишь её записи, а личные логины, карты и прочее останутся в стороне. Новые записи создаются внутри неё.",
      points: [],
      kind: "workspace",
    },
    {
      icon: "folder",
      title: "Дерево подключений",
      lead: "Слева — папки и подключения. Двойной клик или Enter — подключиться.",
      points: [
        ["Правый клик", "меню записи, папки или пустого места: создать, переместить, экспорт, инструменты"],
        ["Ctrl / Shift", "выделить несколько записей; Ctrl+A — все, Del — удалить"],
        ["Перетаскивание", "перенос в папку и свой порядок"],
        ["Точка справа", "красная — не отвечает, жёлтая — доступен, зелёная — подключено"],
      ],
    },
    {
      icon: "plus",
      title: "Новое подключение",
      lead: "Ctrl+N или «+» над деревом. Выберите тип: RDP, SSH, Telnet, COM-порт или MikroTik (откроется в WinBox).",
      points: [
        ["Ctrl+K", "быстрое подключение без сохранения: адрес в строке сверху"],
        ["F2", "изменить выбранную запись"],
        ["Ctrl+F", "поиск по имени, адресу, логину"],
      ],
    },
    {
      icon: "key",
      title: "Учётные данные",
      lead: "Вкладка «Логины» — общие учётки, RD-шлюзы и SSH-ключи. Одна учётка на много серверов: сменили пароль в одном месте — он поменялся везде.",
      points: [
        ["Логины → «+»", "создать учётку (логин, пароль, домен)"],
        ["Подключение → «Из другой записи»", "привязать её к подключению"],
        ["Подпись в дереве", "показывает, под каким логином вы войдёте"],
      ],
    },
    {
      icon: "monitor",
      title: "RDP",
      lead: "Сеанс открывается во вкладке и подстраивает разрешение под размер окна. Поддерживаются RD-шлюзы, буфер обмена, диски, смарт-карты.",
      points: [
        ["F11", "полный экран; Esc — выйти"],
        ["Меню вкладки → «Открепить»", "сеанс в отдельном окне, можно унести на другой монитор; закрыли окно — сеанс вернулся во вкладку"],
      ],
    },
    {
      icon: "terminal",
      title: "SSH и терминал",
      lead: "Терминал с вкладками, вход по паролю, ключу или коду.",
      points: [
        ["Кнопка «Файлы»", "панель SFTP: скачать, загрузить (можно перетащить из проводника), папки"],
        ["Ctrl+Shift+S", "сниппеты — готовые команды, можно отправить во все терминалы сразу"],
        ["Редактор → туннели", "L 8080 host:80 — проброс порта, D 1080 — SOCKS-прокси"],
        ["Промежуточный хост", "подключение через другой SSH-сервер"],
      ],
    },
    {
      icon: "zap",
      title: "Инструменты",
      lead: "В меню подключения (правый клик) — внешние инструменты.",
      points: [
        ["Ping, Tracert, порт", "открываются в отдельном окне в стиле приложения"],
        ["WinBox", "только у подключений MikroTik; пароль кладётся в буфер — вставьте Ctrl+V"],
        ["Свои инструменты", "Настройки → Расширенные → Внешние инструменты"],
      ],
    },
    {
      icon: "download",
      title: "Импорт и экспорт",
      lead: "Перенос подключений из других программ и между хранилищами.",
      points: [
        ["Импорт", "mRemoteNG, Terminals и файлы RDPMgrCoffee — вместе с паролями"],
        ["Экспорт .json", "без логинов и паролей — можно отдавать коллегам"],
        ["Экспорт .rdpexport", "всё, включая пароли, — зашифровано вашим паролем, для переноса"],
      ],
    },
    {
      icon: "settings",
      title: "Настройки",
      lead: "Шестерёнка на верхней панели.",
      points: [
        ["Внешний вид", "темы и редактор своей темы"],
        ["Расширенные", "терминал, индикатор доступности, инструменты, WinBox, безопасность, синхронизация"],
        ["Хранилище", "резервная копия и смена мастер-пароля (локальный режим)"],
        ["Обучение", "этот тур можно пройти заново"],
      ],
    },
  ];

  // Рабочая папка — только для Bitwarden (в локальном хранилище и так только подключения)
  const STEPS = ALL.filter((st) => st.kind !== "workspace" || !app.status.local);

  // ---- Тема ----
  const STOCK: [string, string, string][] = [
    ["system", "Как в системе", "linear-gradient(135deg,#f5f6f8 50%,#15171c 50%)"],
    ["dark", "Тёмная", "#15171c"],
    ["light", "Светлая", "#f5f6f8"],
  ];
  function pickTheme(id: string) {
    app.settings.theme = id;
    app.saveSettings();
    applyTheme(id);
  }

  // ---- Рабочая папка ----
  const topFolders = $derived(
    app.view.folders.map((f) => f.name).filter((n) => !n.includes("/")).sort((a, b) => a.localeCompare(b, "ru")),
  );
  let rootChoice = $state(app.settings.rootFolder || "");
  let newRoot = $state("Подключения");
  let rootBusy = $state(false);
  const rootTarget = $derived(rootChoice === "__new" ? newRoot.trim() : rootChoice);
  async function applyRoot() {
    rootBusy = true;
    app.settings.rootFolder = rootTarget;
    app.saveSettings();
    if (rootTarget) await createRootFolder();
    rootBusy = false;
  }

  let i = $state(0);
  const step = $derived(STEPS[i]);
  const last = $derived(i === STEPS.length - 1);

  function finish() {
    app.settings.tourDone = true;
    app.saveSettings();
    onclose();
  }
  function onKey(e: KeyboardEvent) {
    // Enter на кнопке — это её нажатие, не листаем второй раз
    if (e.key === "Enter" && (e.target as HTMLElement)?.tagName === "BUTTON") return;
    // Стрелки и Enter в списке выбора/поле ввода — для них, а не для листания
    if (e.key !== "Escape" && ["INPUT", "SELECT", "TEXTAREA"].includes((e.target as HTMLElement)?.tagName)) return;
    if (e.key === "Escape") finish();
    else if (e.key === "ArrowRight" || e.key === "Enter") last ? finish() : i++;
    else if (e.key === "ArrowLeft" && i > 0) i--;
    else return;
    e.preventDefault();
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="backdrop" role="presentation">
  <div class="tour" role="dialog" aria-label="Обучение">
    <div class="top">
      <span class="ico"><Icon name={step.icon} size={22} /></span>
      <span class="count faint">{i + 1} / {STEPS.length}</span>
      <button class="icon-btn" title="Закрыть (Esc)" onclick={finish}><Icon name="x" /></button>
    </div>
    {#key i}
      <div class="content">
        <h2>{step.title}</h2>
        <p class="lead">{step.lead}</p>
        {#if step.kind === "theme"}
          <div class="themes">
            {#each STOCK as [id, label, bg]}
              <button class="sw" class:on={app.settings.theme === id} onclick={() => pickTheme(id)}>
                <span class="pv" style="background:{bg}"></span>{label}
              </button>
            {/each}
            {#each PRESETS as t (t.id)}
              <button class="sw" class:on={app.settings.theme === t.id} onclick={() => pickTheme(t.id)}>
                <span class="pv" style="background:{t.gradient ? `linear-gradient(${t.angle}deg, ${t.g1}, ${t.g2})` : t.bg}"
                  ><i style="background:{t.panel}"></i><b style="background:{t.accent}"></b></span
                >{t.name}
              </button>
            {/each}
          </div>
        {:else if step.kind === "workspace"}
          <div class="ws">
            <select bind:value={rootChoice} disabled={rootBusy}>
              <option value="">Всё хранилище (без изоляции)</option>
              {#each topFolders as n}<option value={n}>{n}</option>{/each}
              <option value="__new">Создать новую папку…</option>
            </select>
            {#if rootChoice === "__new"}
              <input bind:value={newRoot} placeholder="Имя папки" disabled={rootBusy} />
            {/if}
            <div class="wsrow">
              <span class="faint">Сейчас: {app.settings.rootFolder || "всё хранилище"}</span>
              <button class="btn primary" onclick={applyRoot}
                disabled={rootBusy || rootTarget === app.settings.rootFolder || (rootChoice === "__new" && !newRoot.trim())}
                >{rootBusy ? "Применение…" : "Применить"}</button
              >
            </div>
            {#if needsMigration()}
              <div class="wsrow">
                <span>Вне рабочей папки подключений: <b>{needsMigration()}</b></span>
                <button class="btn" onclick={() => migrateToRoot()}>Перенести в неё</button>
              </div>
            {/if}
            <small class="faint">Можно пропустить и выбрать позже: Настройки → Рабочая папка.</small>
          </div>
        {/if}
        {#if step.points.length}
          <dl>
            {#each step.points as [k, v]}
              <dt>{k}</dt>
              <dd>{v}</dd>
            {/each}
          </dl>
        {/if}
      </div>
    {/key}
    <div class="foot">
      <div class="dots">
        {#each STEPS as _, n}
          <button class="dot" class:on={n === i} aria-label="Шаг {n + 1}" onclick={() => (i = n)}></button>
        {/each}
      </div>
      <span class="sp"></span>
      {#if i === 0}
        <button class="btn" onclick={finish}>Пропустить</button>
      {:else}
        <button class="btn" onclick={() => i--}>Назад</button>
      {/if}
      <button class="btn primary" onclick={() => (last ? finish() : i++)}>{last ? "Готово" : "Далее"}</button>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 130;
    display: grid;
    place-items: center;
    background: rgba(10, 12, 18, 0.5);
    backdrop-filter: blur(6px);
  }
  .tour {
    width: min(560px, calc(100vw - 32px));
    min-height: 380px;
    display: flex;
    flex-direction: column;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 14px;
    box-shadow: var(--shadow);
    color: var(--text);
    overflow: hidden;
  }
  .top {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 18px 18px 0 22px;
  }
  .ico {
    display: grid;
    place-items: center;
    width: 42px;
    height: 42px;
    border-radius: 11px;
    color: var(--accent);
    background: var(--accent-soft);
  }
  .count {
    margin-left: auto;
    font-size: 12px;
  }
  .content {
    flex: 1;
    padding: 12px 22px 8px;
    animation: in 0.18s ease-out;
  }
  @keyframes in {
    from {
      opacity: 0;
      transform: translateX(8px);
    }
  }
  h2 {
    margin: 4px 0 8px;
    font-size: 18px;
    font-weight: 600;
  }
  .lead {
    margin: 0 0 14px;
    line-height: 1.5;
    color: var(--text-muted);
  }
  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 8px 14px;
    margin: 0;
  }
  dt {
    font-family: var(--mono);
    font-size: 12px;
    padding: 2px 7px;
    border-radius: 5px;
    background: var(--bg-sunken);
    border: 1px solid var(--border);
    align-self: start;
    white-space: nowrap;
  }
  dd {
    margin: 0;
    line-height: 1.45;
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 14px 18px 16px 22px;
    border-top: 1px solid var(--border);
  }
  .sp {
    flex: 1;
  }
  .themes {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(92px, 1fr));
    gap: 8px;
  }
  .sw {
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 6px;
    font-size: 11.5px;
    border-radius: 9px;
    border: 1px solid var(--border);
    color: var(--text-muted);
  }
  .sw.on {
    border-color: var(--accent);
    color: var(--text);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }
  .pv {
    position: relative;
    display: block;
    height: 38px;
    border-radius: 6px;
    overflow: hidden;
  }
  .pv i {
    position: absolute;
    left: 6px;
    top: 6px;
    width: 45%;
    height: 26px;
    border-radius: 4px;
  }
  .pv b {
    position: absolute;
    right: 6px;
    bottom: 6px;
    width: 14px;
    height: 6px;
    border-radius: 3px;
  }
  .ws {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .wsrow {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }
  .dots {
    display: flex;
    gap: 5px;
  }
  .dot {
    width: 7px;
    height: 7px;
    padding: 0;
    border-radius: 50%;
    background: var(--border-strong);
  }
  .dot.on {
    width: 18px;
    border-radius: 4px;
    background: var(--accent);
  }
</style>
