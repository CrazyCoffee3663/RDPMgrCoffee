import { untrack } from "svelte";
import { PRESETS, THEME_VARS, themeVars, type CustomTheme } from "./themes";
import { api, errText, listen, parseConn, fieldValue, DEFAULT_PORTS, type Item, type Protocol, type Status, type VaultView } from "./api";

export type TabState = "connecting" | "connected" | "closed" | "error";
export type Tab = {
  key: string;
  sessionId: number | null;
  protocol: Protocol;
  title: string;
  itemId?: string;
  quick?: { address: string; protocol: string; username?: string; password?: string };
  state: TabState;
  message: string;
  /** Увеличивается при переподключении — вид пересоздаёт сессию */
  gen: number;
  /** RDP-сеанс открыт в отдельном окне */
  detached?: boolean;
};

export type PromptReq = {
  id: number;
  session: number;
  kind: "confirm" | "secret" | "text";
  title: string;
  message: string;
  danger: boolean;
};

export type Toast = { id: number; text: string; error: boolean };

/** Папка для записей RD-шлюзов (по умолчанию скрыта в дереве) */
export const GATEWAY_FOLDER = "RDG";

/** Внешний инструмент. command: путь к программе, "winbox" или "url" */
export type Tool = {
  id: string;
  name: string;
  command: string;
  args: string;
  protocols: string[];
  /** Показывать вывод во всплывающем окне приложения (для консольных программ) */
  popup?: boolean;
};

/** Как запускается подключение типа WinBox: адрес[:порт] логин пароль */
export const WINBOX_CONNECT: Tool = { id: "winbox-connect", name: "WinBox", command: "winbox", args: "%ADDR% %USER% %PASSWORD%", protocols: [] };

export const DEFAULT_TOOLS: Tool[] = [
  { id: "t-ping", name: "Ping", command: "ping", args: "-t %HOST%", protocols: [], popup: true },
  { id: "t-tracert", name: "Tracert", command: "tracert", args: "%HOST%", protocols: [], popup: true },
  {
    id: "t-port", name: "Проверить порт", command: "powershell",
    args: `-NoProfile -Command "$ProgressPreference='SilentlyContinue'; Test-NetConnection %HOST% -Port %PORT% | Format-List"`,
    protocols: [], popup: true,
  },
  { id: "t-winbox", name: "WinBox", command: "winbox", args: "%ADDR% %USER% %PASSWORD%", protocols: [] },
  { id: "t-web", name: "Веб-интерфейс", command: "url", args: "https://%HOST%", protocols: [] },
];

export type Snippet = { id: string; name: string; cmd: string; enter: boolean };

export type Settings = {
  /** "system" | "dark" | "light" | id готовой или своей темы */
  theme: string;
  /** Свои темы пользователя */
  customThemes: CustomTheme[];
  autoLockMinutes: number;
  clipboardClearSeconds: number;
  termFontSize: number;
  termFontFamily: string;
  sidebarWidth: number;
  syncMinutes: number;
  /** Рабочая папка Bitwarden: менеджер видит только её содержимое. "" — всё хранилище */
  rootFolder: string;
  /** Скрытые в дереве папки (пути относительно рабочей) */
  hiddenFolders: string[];
  /** Ручной порядок в дереве: ключ родителя ("root" | "f:путь") → ключи детей ("f:путь" | id записи) */
  order: Record<string, string[]>;
  /** Подпись с логином у подключений в дереве */
  showLogin: boolean;
  /** Индикатор доступности хостов */
  probeEnabled: boolean;
  probeMinutes: number;
  probeTimeoutMs: number;
  probeParallel: number;
  /** Проверять только подключения в раскрытых папках */
  probeVisibleOnly: boolean;
  /** Для подключений через RD-шлюз проверять сам шлюз */
  probeViaGateway: boolean;
  /** Сниппеты команд для терминала */
  snippets: Snippet[];
  /** Внешние инструменты в меню подключения */
  tools: Tool[];
  /** Проверять обновления WinBox (раз в сутки) */
  winboxAutoCheck: boolean;
};

/** Старые стандартные инструменты (cmd /k …) → новые, с выводом в окне приложения */
const OLD_TOOL_ARGS: Record<string, string[]> = {
  "t-ping": ["/k ping -t %HOST%"],
  "t-tracert": ["/k tracert %HOST%"],
  "t-port": ["-NoExit -Command Test-NetConnection %HOST% -Port %PORT%"],
  "t-winbox": ["%HOST% %USER% %PASSWORD%"],
};
function migrateSettings(s: Settings): Settings {
  delete (s as Record<string, unknown>).toolPasswordClipboard;
  s.tools = s.tools.map((t) => {
    const def = DEFAULT_TOOLS.find((d) => d.id === t.id);
    return def && OLD_TOOL_ARGS[t.id]?.includes(t.args) ? { ...def, protocols: t.protocols } : t;
  });
  return s;
}

const DEFAULT_SETTINGS: Settings = {
  theme: "system",
  customThemes: [],
  autoLockMinutes: 15,
  clipboardClearSeconds: 30,
  termFontSize: 14,
  termFontFamily: '"Cascadia Mono", "Cascadia Code", Consolas, "Courier New", monospace',
  sidebarWidth: 300,
  syncMinutes: 5,
  rootFolder: "",
  hiddenFolders: [GATEWAY_FOLDER],
  order: {},
  showLogin: true,
  probeEnabled: true,
  probeMinutes: 2,
  probeTimeoutMs: 1500,
  probeParallel: 16,
  probeVisibleOnly: true,
  probeViaGateway: true,
  snippets: [
    { id: "s-df", name: "Место на дисках", cmd: "df -h", enter: true },
    { id: "s-top", name: "Нагрузка", cmd: "top -o %CPU", enter: true },
    { id: "s-log", name: "Системный журнал", cmd: "journalctl -n 100 -f", enter: true },
  ],
  tools: DEFAULT_TOOLS,
  winboxAutoCheck: true,
};

function loadJson<T>(key: string, def: T): T {
  try {
    const raw = localStorage.getItem(key);
    if (raw) return { ...def, ...JSON.parse(raw) } as T;
  } catch {}
  return def;
}

function loadArr(key: string): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(key) || "[]");
    return Array.isArray(v) ? v : [];
  } catch {
    return [];
  }
}

function save(key: string, v: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(v));
  } catch {}
}

let toastId = 0;
/** Статусы сессий, пришедшие раньше, чем вкладка узнала свой id */
const earlyStatus = new Map<number, { state: TabState; message: string }>();
let tabSeq = 0;

class AppState {
  phase = $state<"loading" | "setup" | "locked" | "unlocked">("loading");
  status = $state<Status>({ state: "setup", server: "", email: "", local: false });
  view = $state<VaultView>({ folders: [], collections: [], orgs: [], items: [] });
  syncing = $state(false);
  syncError = $state("");

  tabs = $state<Tab[]>([]);
  activeTab = $state<string | null>(null);
  mode = $state<"conn" | "logins">("conn");
  selected = $state<string | null>(null);
  /** Полноэкранный режим: окно на весь экран, видны только вкладки */
  fullscreen = $state(false);
  /** Множественный выбор в дереве (Ctrl/Shift); selected — последняя запись */
  selection = $state<string[]>([]);
  /** id записи в редакторе, "new:conn"/"new:login" для новой, null — закрыт */
  editing = $state<string | null>(null);
  editingFolder = $state<string | null>(null);
  overlays = $state(0);
  prompts = $state<PromptReq[]>([]);
  toasts = $state<Toast[]>([]);
  settingsOpen = $state(false);
  importOpen = $state(false);
  snippetsOpen = $state(false);
  /** Доступная новая версия WinBox (если установленная устарела) */
  winboxUpdate = $state<{ installed: string; latest: string } | null>(null);

  settings = $state<Settings>(migrateSettings(loadJson("settings", DEFAULT_SETTINGS)));
  recent = $state<string[]>(loadArr("recent"));
  expanded = $state<string[]>(loadArr("expanded"));

  get items(): Item[] {
    return this.view.items;
  }

  // ---------- Рабочая папка (изоляция от остального хранилища) ----------

  /** Полное имя рабочей папки; "" — показываем всё хранилище */
  root = $derived(this.settings.rootFolder.trim().replace(/^\/+|\/+$/g, ""));
  rootFolder = $derived(this.root ? this.view.folders.find((f) => f.name === this.root) : undefined);
  folderName = $derived(new Map(this.view.folders.map((f) => [f.id, f.name] as [string, string])));

  inScopeName(name: string | undefined): boolean {
    if (!this.root) return true;
    return name !== undefined && (name === this.root || name.startsWith(this.root + "/"));
  }
  inScope(item: Item): boolean {
    return !this.root || (!!item.folderId && this.inScopeName(this.folderName.get(item.folderId)));
  }
  /** Имя папки относительно рабочей ("" — сама рабочая папка) */
  relName(full: string): string {
    if (!this.root) return full;
    return full === this.root ? "" : full.slice(this.root.length + 1);
  }
  /** Полное имя папки по относительному */
  fullName(rel: string): string {
    return [this.root, rel].filter(Boolean).join("/");
  }
  /** id папки по умолчанию для новых записей */
  defaultFolderId(): string | null {
    return this.rootFolder?.id ?? null;
  }

  /** Папки внутри рабочей (без неё самой), с относительными именами */
  scopedFolders = $derived(
    this.view.folders
      .filter((f) => this.inScopeName(f.name) && f.name !== this.root)
      .map((f) => ({ folder: f, rel: this.relName(f.name) }))
      .sort((a, b) => a.rel.localeCompare(b.rel, "ru")),
  );
  scopedItems = $derived(this.root ? this.view.items.filter((i) => this.inScope(i)) : this.view.items);

  /** Шлюзы: адрес rdgw:// или запись, на которую ссылается поле gateway какого-либо подключения */
  gatewayIds = $derived.by(() => {
    const ids = new Set<string>();
    const byName = new Map<string, string[]>();
    for (const i of this.view.items) {
      if (i.uris.some((u) => /^rdgw:\/\//i.test(u.trim()))) ids.add(i.id);
      byName.set(i.name, [...(byName.get(i.name) ?? []), i.id]);
    }
    for (const c of this.view.items) {
      const g = fieldValue(c, "gateway");
      if (!g) continue;
      if (this.view.items.some((i) => i.id === g)) ids.add(g);
      else for (const id of byName.get(g) ?? []) ids.add(id);
    }
    return ids;
  });

  connections = $derived(this.scopedItems.filter((i) => parseConn(i)));
  logins = $derived(this.scopedItems.filter((i) => (i.kind === 1 && !parseConn(i)) || i.kind === 5));
  gateways = $derived(this.logins.filter((i) => this.gatewayIds.has(i.id)));
  credentials = $derived(this.logins.filter((i) => i.kind === 1 && !this.gatewayIds.has(i.id)));
  sshKeys = $derived(this.logins.filter((i) => i.kind === 5 || i.notes.includes("PRIVATE KEY")));
  /** Записи подключений вне рабочей папки — кандидаты на перенос */
  outsideConnections = $derived(this.root ? this.view.items.filter((i) => parseConn(i) && !this.inScope(i)) : []);

  /** Путь папки записи относительно рабочей — для группировки в списках */
  itemFolderRel(item: Item): string {
    const n = item.folderId ? this.folderName.get(item.folderId) : undefined;
    return n === undefined ? "" : this.relName(n);
  }

  item(id: string | null | undefined): Item | undefined {
    return id ? this.view.items.find((i) => i.id === id) : undefined;
  }

  saveSettings() {
    save("settings", this.settings);
    applyTheme(this.settings.theme);
  }

  // ---------- Доступность хостов ----------

  /** id подключения → отвечает ли хост (нет ключа — ещё не проверяли) */
  reach = $state<Record<string, boolean>>({});
  /** Подключения, видимые сейчас в дереве (заполняет Sidebar) */
  visibleIds = $state<string[]>([]);
  private probing = false;

  /** Логин подключения: свой или из записи учётных данных */
  loginOf(it: Item): string {
    const ref = fieldValue(it, "credential");
    if (ref) {
      const c = this.item(ref) ?? this.view.items.find((i) => i.name === ref);
      if (c?.username) return c.username;
    }
    if (it.username) return it.username;
    const m = /^[a-z]+:\/\/([^@/]+)@/i.exec(it.uris[0] ?? "");
    return m ? decodeURIComponent(m[1]) : "";
  }

  /** Куда стучаться для проверки: сам хост или RD-шлюз */
  probeTarget(it: Item): { host: string; port: number; kind: string } | null {
    const c = parseConn(it);
    if (!c || c.protocol === "serial") return null;
    const gw = fieldValue(it, "gateway");
    if (gw && c.protocol === "rdp") {
      if (!this.settings.probeViaGateway) return null;
      const g = this.item(gw) ?? this.view.items.find((i) => i.name === gw);
      const m = /^(?:[a-z]+:\/\/)?(\[[^\]]+\]|[^:/?#]+)(?::(\d+))?/i.exec((g?.uris[0] ?? "").trim());
      return m ? { host: m[1], port: Number(m[2] ?? 443), kind: "tls" } : null;
    }
    const kind = c.protocol === "rdp" || c.protocol === "ssh" ? c.protocol : "tcp";
    return { host: c.host, port: c.port ?? DEFAULT_PORTS[c.protocol] ?? 0, kind };
  }

  async probe() {
    const s = this.settings;
    if (!s.probeEnabled || this.probing || this.phase !== "unlocked") return;
    const ids = s.probeVisibleOnly ? new Set(this.visibleIds) : null;
    const targets: { id: string; host: string; port: number; kind: string }[] = [];
    for (const it of this.connections) {
      if (ids && !ids.has(it.id)) continue;
      const t = this.probeTarget(it);
      if (t) targets.push({ id: it.id, ...t });
    }
    if (!targets.length) return;
    this.probing = true;
    try {
      const res = await api.probeHosts(targets, s.probeTimeoutMs, s.probeParallel);
      const next = { ...this.reach };
      for (const r of res) next[r.id] = r.up;
      this.reach = next;
    } catch {
      /* сеть недоступна — оставляем прежние значения */
    } finally {
      this.probing = false;
    }
  }

  /** Показывать скрытые папки (временно, не сохраняется) */
  showHidden = $state(false);
  isHidden(rel: string): boolean {
    return this.settings.hiddenFolders.some((h) => rel === h || rel.startsWith(h + "/"));
  }
  setHidden(rel: string, hidden: boolean) {
    const list = this.settings.hiddenFolders.filter((h) => h !== rel);
    if (hidden) list.push(rel);
    this.settings.hiddenFolders = list;
    this.saveSettings();
  }

  toggleExpanded(key: string, open?: boolean) {
    const has = this.expanded.includes(key);
    const want = open ?? !has;
    if (want && !has) this.expanded = [...this.expanded, key];
    if (!want && has) this.expanded = this.expanded.filter((k) => k !== key);
    save("expanded", this.expanded);
  }

  toast(text: string, error = false) {
    const id = ++toastId;
    this.toasts = [...this.toasts, { id, text, error }];
    setTimeout(() => (this.toasts = this.toasts.filter((t) => t.id !== id)), error ? 7000 : 3000);
  }

  // ---------- Хранилище ----------

  async init() {
    try {
      this.status = await api.status();
      this.phase = this.status.state === "unlocked" ? "unlocked" : this.status.state;
      if (this.phase === "unlocked") await this.reload();
    } catch (e) {
      this.toast(errText(e), true);
      this.phase = "setup";
    }
  }

  async afterUnlock(hasCache: boolean) {
    this.status = await api.status();
    this.phase = "unlocked";
    if (hasCache) await this.reload();
    await this.sync(!hasCache);
  }

  async reload() {
    this.view = await api.view();
    if (this.selected && !this.item(this.selected)) this.selected = null;
  }

  async sync(force = false) {
    if (this.syncing || this.phase !== "unlocked") return;
    this.syncing = true;
    try {
      const changed = await api.sync(force);
      this.syncError = "";
      if (changed || force) await this.reload();
    } catch (e) {
      this.syncError = errText(e);
      if (force) this.toast(this.syncError, true);
    } finally {
      this.syncing = false;
    }
  }

  async lock() {
    await api.lock();
    this.view = { folders: [], collections: [], orgs: [], items: [] };
    this.editing = null;
    this.selected = null;
    this.phase = "locked";
  }

  async logout() {
    await api.logout();
    this.view = { folders: [], collections: [], orgs: [], items: [] };
    this.editing = null;
    this.status = await api.status();
    this.phase = "setup";
  }

  upsert(item: Item) {
    const i = this.view.items.findIndex((x) => x.id === item.id);
    if (i >= 0) this.view.items[i] = item;
    else this.view.items.push(item);
  }

  // ---------- Вкладки ----------

  connect(item: Item) {
    const c = parseConn(item);
    if (!c) {
      this.toast("У записи нет адреса подключения", true);
      return;
    }
    this.recent = [item.id, ...this.recent.filter((r) => r !== item.id)].slice(0, 12);
    save("recent", this.recent);
    if (c.protocol === "winbox") {
      // WinBox — отдельная программа, не вкладка
      import("./actions").then((m) => m.runTool(item, WINBOX_CONNECT));
      return;
    }
    this.addTab({ protocol: c.protocol, title: item.name, itemId: item.id });
  }

  quickConnect(address: string, protocol: string, username?: string, password?: string) {
    const m = /^([a-z]+):\/\//i.exec(address);
    const proto = (m ? m[1].toLowerCase() : protocol) as Protocol;
    this.addTab({ protocol: proto, title: address, quick: { address, protocol, username, password } });
  }

  private addTab(t: Pick<Tab, "protocol" | "title" | "itemId" | "quick">) {
    const key = `t${++tabSeq}`;
    this.tabs.push({ key, sessionId: null, state: "connecting", message: "", gen: 0, ...t });
    this.activeTab = key;
  }

  bindSession(key: string, id: number) {
    const t = this.tab(key);
    if (!t) {
      api.close(id).catch(() => {});
      return;
    }
    t.sessionId = id;
    const st = earlyStatus.get(id);
    if (st) {
      earlyStatus.delete(id);
      t.state = st.state;
      t.message = st.message;
    }
  }

  tab(key: string | null): Tab | undefined {
    return this.tabs.find((t) => t.key === key);
  }

  closeTab(key: string) {
    const i = this.tabs.findIndex((t) => t.key === key);
    if (i < 0) return;
    const t = this.tabs[i];
    if (t.sessionId !== null) api.close(t.sessionId).catch(() => {});
    this.tabs.splice(i, 1);
    if (this.activeTab === key) {
      this.activeTab = this.tabs[Math.min(i, this.tabs.length - 1)]?.key ?? null;
    }
  }

  reconnect(key: string) {
    const t = this.tab(key);
    if (!t) return;
    if (t.sessionId !== null && t.state === "connected") api.close(t.sessionId).catch(() => {});
    t.sessionId = null;
    t.state = "connecting";
    t.message = "";
    t.gen++;
  }

  cycleTab(dir: 1 | -1) {
    if (!this.tabs.length) return;
    const i = this.tabs.findIndex((t) => t.key === this.activeTab);
    this.activeTab = this.tabs[(i + dir + this.tabs.length) % this.tabs.length].key;
  }

  // ---------- Оверлеи (скрывают окна RDP, которые иначе перекрыли бы диалог) ----------

  // untrack: вызываются из $effect, чтение счётчика не должно делать его зависимостью
  pushOverlay() {
    this.overlays = untrack(() => this.overlays) + 1;
  }
  popOverlay() {
    this.overlays = Math.max(0, untrack(() => this.overlays) - 1);
  }
}

export const app = new AppState();

/** Тема по id: готовая или своя */
export function findTheme(id: string): CustomTheme | undefined {
  return PRESETS.find((t) => t.id === id) ?? app.settings.customThemes.find((t) => t.id === id);
}

/** Применить тему; preview — показать тему без сохранения (редактор тем) */
export function applyTheme(theme: string, preview?: CustomTheme) {
  const root = document.documentElement;
  for (const k of THEME_VARS) root.style.removeProperty(k);
  const t = preview ?? findTheme(theme);
  let dark: boolean;
  if (t) {
    dark = t.base === "dark";
    for (const [k, v] of Object.entries(themeVars(t))) root.style.setProperty(k, v);
  } else {
    dark = theme === "dark" || (theme !== "light" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  }
  root.dataset.theme = dark ? "dark" : "light";
  // Сигнал терминалам: цвета поменялись
  root.dataset.themeRev = String(Number(root.dataset.themeRev ?? 0) + 1);
}

export async function setupEvents() {
  await listen<{ id: number; state: TabState; message: string }>("session-status", (e) => {
    const t = app.tabs.find((t) => t.sessionId === e.payload.id);
    if (!t) {
      earlyStatus.set(e.payload.id, { state: e.payload.state, message: e.payload.message });
      return;
    }
    t.state = e.payload.state;
    t.message = e.payload.message;
  });
  await listen<number>("rdp-docked", (e) => {
    const t = app.tabs.find((t) => t.sessionId === e.payload);
    if (t) {
      t.detached = false;
      app.activeTab = t.key;
    }
  });
  await listen<PromptReq>("prompt", (e) => {
    app.prompts.push(e.payload);
  });
}
