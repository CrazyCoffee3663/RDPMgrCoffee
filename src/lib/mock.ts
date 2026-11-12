// Демо-данные для просмотра интерфейса в обычном браузере (только `npm run dev`, вне Tauri).
// В сборку приложения не попадает.

type Handler = (args: any) => any;
let seq = 100;
const uid = () => "id-" + ++seq;

const folders = [
  { id: "f1", name: "Офис" },
  { id: "f2", name: "Офис/Серверы" },
  { id: "f3", name: "Клиенты" },
  { id: "f4", name: "Сеть" },
];

const items: any[] = [
  conn("Контроллер домена", "f2", "rdp://dc01.corp.local", "CORP\\admin", [["gateway", "gw"]], true),
  conn("Сервер 1С", "f2", "rdp://srv-1c.corp.local:3390", "", [["credential", "cred1"]]),
  conn("Файловый сервер", "f2", "rdp://fs01.corp.local", "admin"),
  conn("Веб-сервер", "f1", "ssh://web01.corp.local", "deploy", [], true),
  conn("База данных", "f1", "ssh://db01.corp.local:2222", "postgres"),
  conn("Коммутатор ядра", "f4", "telnet://10.0.0.2", "admin"),
  conn("Консоль маршрутизатора", "f4", "serial://COM3", "", [["baud", "115200"]]),
  conn("ООО Ромашка — терминал", "f3", "rdp://ts.romashka.ru", "ivanov"),
  conn("Домашний NAS", null, "ssh://nas.local", "root"),
  login("cred1", "Сервисная учётка 1С", null, "CORP\\svc_1c"),
  login("gw", "RD Gateway", null, "CORP\\gwuser", ["https://gw.corp.local"]),
  login(uid(), "Почта", "f1", "user@corp.local", ["https://mail.corp.local"]),
  { ...login(uid(), "Ключ деплоя", null, ""), kind: 5, hasPassword: false },
];

function conn(name: string, folderId: string | null, uri: string, username: string, fields: [string, string][] = [], favorite = false) {
  return {
    id: uid(), orgId: null, folderId, collectionIds: [], kind: 1, name, notes: "", username,
    hasPassword: !!username, hasTotp: false, uris: [uri],
    fields: fields.map(([name, value]) => ({ name, value, kind: 0 })), favorite, edit: true, revision: "",
  };
}
function login(id: string, name: string, folderId: string | null, username: string, uris: string[] = []) {
  return {
    id, orgId: null, folderId, collectionIds: [], kind: 1, name, notes: "", username, hasPassword: true,
    hasTotp: false, uris, fields: [], favorite: false, edit: true, revision: "",
  };
}

let state: "setup" | "locked" | "unlocked" = "locked";
const channels = new Map<number, (d: ArrayBuffer) => void>();
const enc = new TextEncoder();
const listeners = new Map<string, ((e: any) => void)[]>();

export function mockEmit(event: string, payload: any) {
  for (const f of listeners.get(event) ?? []) f({ payload });
}
export function mockListen(event: string, f: (e: any) => void) {
  listeners.set(event, [...(listeners.get(event) ?? []), f]);
  return Promise.resolve(() => {});
}

let sid = 0;
const H: Record<string, Handler> = {
  vault_status: () => ({ state, server: "https://vault.example.com", email: "admin@example.com" }),
  vault_login: () => ((state = "unlocked"), { status: "ok", providers: [] }),
  vault_unlock: () => ((state = "unlocked"), true),
  vault_lock: () => (state = "locked"),
  vault_logout: () => (state = "setup"),
  vault_sync: () => false,
  vault_view: () => ({ folders, collections: [], orgs: [], items }),
  item_secret: () => ({ password: "S3cr3t-P@ss", totp: "", fields: [], sshPrivateKey: "" }),
  item_save: ({ item }) => {
    const saved = {
      ...item, id: item.id ?? uid(), kind: item.kind ?? 1, collectionIds: item.collectionIds ?? [],
      orgId: item.orgId ?? null, hasPassword: !!item.password, hasTotp: false, edit: true, revision: "",
    };
    const i = items.findIndex((x) => x.id === saved.id);
    if (i >= 0) items[i] = saved;
    else items.push(saved);
    return saved;
  },
  item_move: ({ id, folderId, toRoot, favorite }) => {
    const it = items.find((x) => x.id === id);
    if (folderId || toRoot) it.folderId = folderId ?? null;
    if (favorite !== undefined) it.favorite = favorite;
    return it;
  },
  item_delete: ({ id }) => {
    items.splice(items.findIndex((x) => x.id === id), 1);
  },
  folder_save: ({ id, name }) => {
    const f = id ? folders.find((x) => x.id === id)! : { id: uid(), name };
    f.name = name;
    if (!id) folders.push(f);
    return f;
  },
  folder_delete: () => {},
  generate_password: () => "Xk7#mP2q-Lw9zR4v@Tn8",
  item_copy: () => {},
  clipboard_set: () => {},
  clipboard_get: () => "",
  serial_ports: () => ["COM1", "COM3"],
  rdp_detach: () => {},
  tool_run: () => {},
  sftp_list: ({ path }) => ({
    path: path || "/home/deploy",
    entries: [
      { name: "logs", dir: true, link: false, size: 0, mtime: 1794480000 },
      { name: "app", dir: true, link: false, size: 0, mtime: 1794480000 },
      { name: "backup.tar.gz", dir: false, link: false, size: 734003200, mtime: 1794470000 },
      { name: ".bashrc", dir: false, link: false, size: 3771, mtime: 1794400000 },
    ],
  }),
  sftp_download: ({ path }) => "C:\Users\demo\Downloads\\" + String(path).split("/").pop(),
  sftp_upload: () => {},
  sftp_op: () => {},
  winbox_status: () => ({ installed: "4.4", latest: "4.5", path: "C:\tools\winbox" }),
  winbox_install: () => {},
  rdp_attach: () => {},
  probe_hosts: ({ targets }) => targets.map((t: { id: string }, i: number) => ({ id: t.id, up: i % 3 !== 2 })),
  mremoteng_default_file: () => "C:\Users\demo\AppData\Roaming\mRemoteNG\confCons.xml",
  import_mremoteng_file: (a) => H.import_mremoteng(a),
  import_mremoteng: ({ password }) => {
    if (!password) throw "MREMOTENG_PASSWORD";
    return {
      folders: [["Офис"], ["Офис", "Серверы"]],
      skipped: ["web (HTTPS)"],
      conns: [
        { path: ["Офис", "Серверы"], name: "dc01", protocol: "rdp", host: "10.1.0.10", port: null, username: "admin", password: "p1", domain: "CORP", notes: "", opts: { admin: "true" }, gateway: { host: "gw.corp", username: "", password: "", domain: "", same: true } },
        { path: ["Офис", "Серверы"], name: "dc02", protocol: "rdp", host: "10.1.0.11", port: null, username: "petr", password: "p3", domain: "CORP", notes: "", opts: {}, gateway: { host: "GW.corp", username: "petr", password: "p3", domain: "CORP", same: false } },
        { path: ["Офис", "Серверы"], name: "dc03", protocol: "rdp", host: "10.1.0.12", port: null, username: "ivan", password: "p4", domain: "CORP", notes: "", opts: {}, gateway: { host: "gw.corp:443", username: "ivan", password: "p4", domain: "CORP", same: false } },
        { path: ["Офис"], name: "nas", protocol: "ssh", host: "10.1.0.20", port: 2222, username: "root", password: "p2", domain: "", notes: "", opts: {}, gateway: null },
      ],
    };
  },
  session_open: ({ args, onData }) => {
    const id = ++sid;
    channels.set(id, onData.onmessage);
    const it = items.find((x) => x.id === args.itemId);
    const proto = (it?.uris[0] ?? args.quick ?? "ssh://").split("://")[0];
    setTimeout(() => mockEmit("session-status", { id, state: "connecting", message: "" }));
    if (proto !== "rdp") {
      setTimeout(() => {
        const send = (s: string) => onData.onmessage?.(enc.encode(s).buffer);
        send(`\x1b[90mПодключение к ${it?.name ?? args.quick}…\x1b[0m\r\n`);
        send("Welcome to Ubuntu 26.04 LTS (GNU/Linux 6.14.0 x86_64)\r\n\r\n");
        send("\x1b[1;32mdeploy@web01\x1b[0m:\x1b[1;34m~\x1b[0m$ ");
        mockEmit("session-status", { id, state: "connected", message: "" });
      }, 400);
    } else {
      setTimeout(() => mockEmit("session-status", { id, state: "connected", message: "" }), 600);
    }
    return { id, kind: proto === "rdp" ? "rdp" : "term", protocol: proto, title: it?.name ?? args.quick };
  },
  session_input: ({ id, data }) => {
    const out = channels.get(id);
    const s = data === "\r" ? "\r\n\x1b[1;32mdeploy@web01\x1b[0m:\x1b[1;34m~\x1b[0m$ " : data;
    out?.(enc.encode(s).buffer);
  },
  session_resize: () => {},
  session_close: ({ id }) => mockEmit("session-status", { id, state: "closed", message: "" }),
  rdp_place: () => {},
  prompt_answer: () => {},
};

export async function mockInvoke<T>(cmd: string, args: any = {}): Promise<T> {
  await new Promise((r) => setTimeout(r, 60));
  const h = H[cmd];
  if (!h) throw `mock: нет обработчика ${cmd}`;
  return h(args) as T;
}

export class MockChannel<T> {
  onmessage: (d: T) => void = () => {};
}
