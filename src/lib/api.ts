import { invoke as tauriInvoke, Channel } from "@tauri-apps/api/core";
import { revealItemInDir, openUrl as openerOpenUrl } from "@tauri-apps/plugin-opener";
import { listen as tauriListen, type EventCallback, type UnlistenFn } from "@tauri-apps/api/event";

// Вне Tauri (npm run dev в обычном браузере) работают демо-данные из mock.ts
export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
let mock: typeof import("./mock") | null = null;
const mockReady = !inTauri && import.meta.env.DEV ? import("./mock").then((m) => (mock = m)) : null;

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (inTauri) return tauriInvoke<T>(cmd, args);
  await mockReady;
  if (!mock) throw "Приложение запущено вне Tauri";
  return mock.mockInvoke<T>(cmd, args);
}

export function makeChannel<T>(): Channel<T> {
  if (inTauri) return new Channel<T>();
  // Демо-режим: достаточно объекта с onmessage (mock.ts может быть ещё не загружен)
  return { onmessage: () => {} } as unknown as Channel<T>;
}

export async function listen<T>(event: string, cb: EventCallback<T>): Promise<UnlistenFn> {
  if (inTauri) return tauriListen<T>(event, cb);
  await mockReady;
  return mock!.mockListen(event, cb as (e: unknown) => void);
}

export type Folder = { id: string; name: string };
export type Collection = { id: string; orgId: string; name: string; readOnly: boolean };
export type Org = { id: string; name: string };
export type Field = { name: string; value: string | null; kind: number };
export type Item = {
  id: string;
  orgId: string | null;
  folderId: string | null;
  collectionIds: string[];
  kind: number;
  name: string;
  notes: string;
  username: string;
  hasPassword: boolean;
  hasTotp: boolean;
  uris: string[];
  fields: Field[];
  favorite: boolean;
  edit: boolean;
  revision: string;
};
export type VaultView = { folders: Folder[]; collections: Collection[]; orgs: Org[]; items: Item[] };
export type Status = { state: "setup" | "locked" | "unlocked"; server: string; email: string; local: boolean };
export type LoginResult = { status: "ok" | "twoFactor" | "newDevice"; providers: number[] };
export type Secret = { password: string; totp: string; fields: Field[]; sshPrivateKey: string };

export type ItemInput = {
  id?: string | null;
  kind?: number;
  orgId?: string | null;
  collectionIds?: string[];
  folderId: string | null;
  name: string;
  notes: string;
  username: string;
  password: string | null;
  uris: string[];
  fields: Field[];
  favorite: boolean;
};

export type Opened = { id: number; kind: "term" | "rdp"; protocol: string; title: string };
export type OpenArgs = {
  itemId?: string;
  quick?: string;
  quickProtocol?: string;
  quickUsername?: string;
  quickPassword?: string;
  cols: number;
  rows: number;
  rect?: [number, number, number, number];
  /** Писать журнал FreeRDP на диск */
  rdpLog?: boolean;
  /** Повтор после неудачи Kerberos: только NTLM */
  forceNtlm?: boolean;
};

export const api = {
  status: () => invoke<Status>("vault_status"),
  /** Подсказка из прежней версии (com.user.connmgr): сервер/почта для формы входа */
  legacyHint: () => invoke<{ server: string; email: string } | null>("legacy_hint"),
  login: (server: string, email: string, password: string) =>
    invoke<LoginResult>("vault_login", { server, email, password }),
  login2fa: (provider: number, code: string) => invoke<LoginResult>("vault_login_2fa", { provider, code }),
  loginNewDevice: (code: string) => invoke<LoginResult>("vault_login_new_device", { code }),
  sendEmailCode: () => invoke<void>("vault_send_email_code"),
  unlock: (password: string) => invoke<boolean>("vault_unlock", { password }),
  /** Локальное хранилище без Bitwarden */
  createLocal: (password: string) => invoke<void>("vault_create_local", { password }),
  changeLocalPassword: (old: string, next: string) => invoke<void>("vault_change_local_password", { old, new: next }),
  /** Зашифрованная резервная копия в «Загрузки»; возвращает путь */
  localBackup: () => invoke<string>("vault_local_backup"),
  localRestore: (text: string) => invoke<void>("vault_local_restore", { text }),
  lock: () => invoke<void>("vault_lock"),
  logout: () => invoke<void>("vault_logout"),
  sync: (force: boolean) => invoke<boolean>("vault_sync", { force }),
  view: () => invoke<VaultView>("vault_view"),
  secret: (id: string) => invoke<Secret>("item_secret", { id }),
  save: (item: ItemInput) => invoke<Item>("item_save", { item }),
  move: (id: string, folderId: string | null) =>
    invoke<Item>("item_move", { id, folderId, toRoot: folderId === null }),
  setFavorite: (id: string, favorite: boolean) => invoke<Item>("item_move", { id, favorite }),
  remove: (id: string) => invoke<void>("item_delete", { id }),
  saveFolder: (id: string | null, name: string) => invoke<Folder>("folder_save", { id, name }),
  deleteFolder: (id: string) => invoke<void>("folder_delete", { id }),
  generate: (length: number, upper = true, lower = true, digits = true, symbols = true) =>
    invoke<string>("generate_password", { length, upper, lower, digits, symbols }),
  copySecret: (id: string, what: string, clearAfter: number | null) =>
    invoke<void>("item_copy", { id, what, clearAfter }),
  /** secret: не сохранять в журнале буфера Windows (Win+V) и облачном буфере */
  clipboardSet: (text: string, clearAfter: number | null = null, secret = false) =>
    invoke<void>("clipboard_set", { text, clearAfter, secret }),
  clipboardGet: () => invoke<string>("clipboard_get"),

  open: (args: OpenArgs, onData: Channel<ArrayBuffer>) => invoke<Opened>("session_open", { args, onData }),
  input: (id: number, data: string) => invoke<void>("session_input", { id, data }),
  /** Двоичные данные в сеанс (VNC) */
  inputBytes: (id: number, data: number[]) => invoke<void>("session_input_bytes", { id, data }),
  resize: (id: number, cols: number, rows: number) => invoke<void>("session_resize", { id, cols, rows }),
  close: (id: number) => invoke<void>("session_close", { id }),
  rdpPlace: (id: number, x: number, y: number, w: number, h: number, visible: boolean) =>
    invoke<void>("rdp_place", { id, x, y, w, h, visible }),
  promptAnswer: (id: number, value: string | null) => invoke<void>("prompt_answer", { id, value }),
  serialPorts: () => invoke<string[]>("serial_ports"),
  sftpList: (id: number, path: string) =>
    invoke<{ path: string; entries: { name: string; dir: boolean; link: boolean; size: number; mtime: number }[] }>("sftp_list", { id, path }),
  sftpDownload: (id: number, path: string) => invoke<string>("sftp_download", { id, path }),
  /** Кусок файла: сырое тело + заголовки x-sid/x-path/x-offset (файл не целиком в памяти webview) */
  sftpUploadChunk: (id: number, path: string, offset: number, data: Uint8Array) =>
    inTauri
      ? tauriInvoke<void>("sftp_upload_chunk", data, { headers: { "x-sid": String(id), "x-path": encodeURIComponent(path), "x-offset": String(offset) } })
      : invoke<void>("sftp_upload_chunk", { id, path, offset, data: Array.from(data) }),
  sftpOp: (id: number, op: "mkdir" | "rmdir" | "rm" | "rename", path: string, to?: string) =>
    invoke<void>("sftp_op", { id, op, path, to }),
  /** Сохранить текстовый файл в «Загрузки» (без перезаписи), вернуть путь */
  saveDownload: (name: string, text: string) => invoke<string>("save_download", { name, text }),
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  exportEncrypted: (file: any, ids: string[], password: string, name: string) =>
    invoke<string>("export_encrypted", { file, ids, password, name }),
  /** Расшифровать файл зашифрованного экспорта — возвращает JSON с паролями */
  importEncrypted: (text: string, password: string) => invoke<string>("import_encrypted", { text, password }),
  /** Сколько миллисекунд не было ввода с клавиатуры и мыши во всей системе */
  idleMs: () => invoke<number>("idle_ms"),
  /** Веб-вкладка: встроенная страница поверх области вкладки (логические пиксели) */
  webOpen: (label: string, url: string, x: number, y: number, w: number, h: number) =>
    invoke<void>("web_open", { label, url, x, y, w, h }),
  webPlace: (label: string, x: number, y: number, w: number, h: number, visible: boolean) =>
    invoke<void>("web_place", { label, x, y, w, h, visible }),
  webClose: (label: string) => invoke<void>("web_close", { label }),
  webNav: (label: string, op: "back" | "forward" | "reload" | "stop") => invoke<void>("web_nav", { label, op }),
  /** Открыть ссылку https в браузере по умолчанию */
  openUrl: (url: string) => (inTauri ? openerOpenUrl(url) : Promise.resolve(void window.open(url, "_blank"))),
  /** Показать файл в проводнике */
  reveal: (path: string) => (inTauri ? revealItemInDir(path) : Promise.resolve()),
  /** Возвращает подсказку, если пароль положен в буфер обмена */
  toolRun: (itemId: string, command: string, args: string) => invoke<string | null>("tool_run", { itemId, command, args }),
  /** Выполнить инструмент с выводом в текущее окно; код выхода или null, если остановлен */
  toolExec: (itemId: string, command: string, args: string, onData: Channel<string>) =>
    invoke<number | null>("tool_exec", { itemId, command, args, onData }),
  toolStop: () => invoke<void>("tool_stop"),
  winboxStatus: () => invoke<{ installed: string | null; latest: string | null; path: string }>("winbox_status"),
  winboxInstall: (version: string) => invoke<void>("winbox_install", { version }),
  freerdpStatus: () => invoke<{ installed: string | null; latest: string | null; url: string }>("freerdp_status"),
  freerdpUpdate: () => invoke<{ version: string; already: boolean }>("freerdp_update"),
  appUpdateStatus: () => invoke<{ current: string; latest: string | null; url: string }>("app_update_status"),
  appUpdateInstall: () => invoke<{ version: string; already: boolean }>("app_update_install"),
  rdpDetach: (id: number, title: string) => invoke<void>("rdp_detach", { id, title }),
  rdpAttach: (id: number) => invoke<void>("rdp_attach", { id }),
  probeHosts: (targets: { id: string; host: string; port: number; kind: string }[], timeoutMs: number, parallel: number) =>
    invoke<{ id: string; up: boolean }[]>("probe_hosts", { targets, timeoutMs, parallel }),
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  importMremoteng: (xml: string, password: string | null) => invoke<any>("import_mremoteng", { xml, password }),
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  importMremotengFile: (path: string, password: string | null) => invoke<any>("import_mremoteng_file", { path, password }),
  mremotengDefaultFile: () => invoke<string | null>("mremoteng_default_file"),
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  importTerminals: (config: string, favorites: string, credentials: string, password: string | null) =>
    invoke<any>("import_terminals", { config, favorites, credentials, password }),
  terminalsDefaultDir: () => invoke<string | null>("terminals_default_dir"),
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  importTerminalsDir: (dir: string, password: string | null) => invoke<any>("import_terminals_dir", { dir, password }),
};

export function errText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}

// ---------- Модель подключений поверх записей Bitwarden ----------

export const PROTOCOLS = ["rdp", "ssh", "telnet", "serial", "winbox", "vnc", "web"] as const;
export type Protocol = (typeof PROTOCOLS)[number];
export const DEFAULT_PORTS: Record<Protocol, number | null> = { rdp: 3389, ssh: 22, telnet: 23, serial: null, winbox: 8291, vnc: 5900, web: null };
export const PROTOCOL_LABEL: Record<Protocol, string> = { rdp: "RDP", ssh: "SSH", telnet: "Telnet", serial: "COM", winbox: "MikroTik", vnc: "VNC", web: "WEB" };
/** Протоколы, которые открываются во вкладке (WinBox запускается отдельной программой) */
export const TAB_PROTOCOLS = PROTOCOLS.filter((p) => p !== "winbox" && p !== "web");

/** Поля, которыми управляет редактор подключения. Остальные показываются как «дополнительные». */
export const KNOWN_FIELDS = [
  "credential", "domain", "gateway", "sshkey", "resolution", "admin", "sound", "microphone",
  "drives", "printers", "clipboard", "smartcard", "auth", "gwtransport", "tunnels", "jump", "mikrotik", "winboxport", "protocol", "cert", "freerdp", "keepalive", "baud", "sercfg",
];

export type ConnInfo = { protocol: Protocol; host: string; port: number | null; uri: string };

/** Хост для URI: IPv6 в скобках (зеркало backend parse_uri/build_args) */
export function bracketHost(host: string): string {
  return host.includes(":") ? `[${host}]` : host;
}

/** Порт в диапазоне u16, иначе null (зеркало backend parse::<u16>) */
function parsePort(p: string): number | null {
  if (!/^\d+$/.test(p)) return null;
  const n = Number(p);
  return n <= 65535 ? n : null;
}

/** Единый список «истинных» значений флажков (зеркало backend conn::is_truthy) */
export function flagValue(item: Item, name: string): boolean {
  return ["1", "true", "yes", "да", "on"].includes(fieldValue(item, name).trim().toLowerCase());
}

// Разбор адреса зеркалит backend conn::parse_uri (общие тест-векторы в probes):
// userinfo по последнему @, голый IPv6 не режется, порт вне u16 — отказ, uri trimmed.
export function parseConn(item: Item): ConnInfo | null {
  if (item.kind !== 1) return null;
  // Веб-подключение: обычная ссылка http(s) с полем protocol=web (запись остаётся понятной Bitwarden)
  const isWeb = fieldValue(item, "protocol").toLowerCase() === "web";
  for (const raw of item.uris) {
    const uri = raw.trim();
    const sep = uri.indexOf("://");
    if (sep < 1) continue;
    const scheme = uri.slice(0, sep).toLowerCase();
    const rest = uri.slice(sep + 3);
    const auth = rest.split(/[/?#]/)[0] ?? "";
    const at = auth.lastIndexOf("@");
    const hp = at >= 0 ? auth.slice(at + 1) : auth;
    let host: string;
    let port: number | null = null;
    if (hp.startsWith("[")) {
      const close = hp.indexOf("]");
      if (close < 0) continue;
      host = hp.slice(1, close);
      const after = hp.slice(close + 1);
      if (after.startsWith(":")) {
        const p = after.slice(1);
        port = parsePort(p);
        if (/^\d+$/.test(p) && port === null) continue;
      }
    } else if (scheme === "serial") {
      host = hp;
    } else if ((hp.match(/:/g) ?? []).length >= 2) {
      // Голый IPv6: порт возможен только в скобках
      host = hp;
    } else {
      const colon = hp.lastIndexOf(":");
      const p = colon > 0 ? hp.slice(colon + 1) : "";
      if (colon > 0 && /^\d+$/.test(p)) {
        port = parsePort(p);
        if (port === null) continue;
        host = hp.slice(0, colon);
      } else {
        host = hp;
      }
    }
    if (!host) continue;
    if (isWeb && (scheme === "http" || scheme === "https")) {
      return { protocol: "web", host, port, uri };
    }
    if ((PROTOCOLS as readonly string[]).includes(scheme)) {
      return { protocol: scheme as Protocol, host, port, uri };
    }
  }
  return null;
}

export function buildUri(protocol: Protocol, host: string, port: number | null): string {
  if (protocol === "web") {
    const h = host.trim();
    return /^https?:\/\//i.test(h) ? h : `https://${h}${port ? ":" + port : ""}`;
  }
  const h = protocol !== "serial" ? bracketHost(host.trim()) : host.trim();
  const p = port && port !== DEFAULT_PORTS[protocol] ? `:${port}` : "";
  return `${protocol}://${h}${p}`;
}

export function fieldValue(item: Item, name: string): string {
  const f = item.fields.find((f) => f.name.trim().toLowerCase() === name);
  return f?.value ?? "";
}
