// Экспорт подключений в файл БЕЗ логинов и паролей.
//
// Попадает: имя, папка, протокол, адрес, порт, домен, параметры подключения, адрес RD-шлюза.
// Не попадает: логин, пароль, TOTP, ссылки на учётные записи и SSH-ключи, скрытые поля,
// заметки (в них часто хранят пароли), промежуточный SSH-хост (ссылка на другую запись).
// Формат совместим с импортом (ImportDialog): те же поля, что у импорта из mRemoteNG.
import { api, errText, fieldValue, parseConn, inTauri, KNOWN_FIELDS, type Item } from "./api";
import { app } from "./state.svelte";

export const EXPORT_FORMAT = "rdpmgrcoffee-connections";

/** Поля, которые никогда не выгружаются: ссылки на секреты и учётки */
const NEVER = new Set(["credential", "sshkey", "jump", "gateway", "domain", "totp", "password", "username"]);
/** Поля с именами, похожими на секреты, — тоже не выгружаем */
const SECRETISH = /pass|парол|secret|token|key|ключ|otp|pin/i;

export type ExportedGw = { host: string; username: ""; password: ""; domain: string; same: boolean };
export type ExportedConn = {
  path: string[];
  name: string;
  protocol: string;
  host: string;
  port: number | null;
  username: "";
  password: "";
  domain: string;
  notes: "";
  opts: Record<string, string>;
  gateway: ExportedGw | null;
};
export type ExportFile = {
  format: typeof EXPORT_FORMAT;
  version: 1;
  exported: string;
  folders: string[][];
  conns: ExportedConn[];
};

function gatewayOf(it: Item): ExportedGw | null {
  const ref = fieldValue(it, "gateway");
  if (!ref) return null;
  const g = app.item(ref) ?? app.view.items.find((i) => i.name === ref);
  const host = (g?.uris[0] ?? "").trim().replace(/^[a-z]+:\/\//i, "").replace(/^[^@/]*@/, "").replace(/\/.*$/, "");
  if (!host) return null;
  const same = !!g && fieldValue(g, "samecreds") === "true";
  return { host, username: "", password: "", domain: g && !same ? fieldValue(g, "domain") : "", same };
}

/** Домен без логина: поле domain, иначе из «ДОМЕН\логин» / «логин@домен» записи или её учётки */
function domainOf(it: Item): string {
  const fromUser = (u: string) => {
    u = u.trim();
    if (u.includes("\\")) return u.split("\\")[0];
    if (u.includes("@")) return u.slice(u.lastIndexOf("@") + 1);
    return "";
  };
  const own = fieldValue(it, "domain") || fromUser(it.username);
  if (own) return own;
  const ref = fieldValue(it, "credential");
  const cred = ref ? (app.item(ref) ?? app.view.items.find((i) => i.name === ref)) : undefined;
  return cred ? fieldValue(cred, "domain") || fromUser(cred.username) : "";
}

/** Собрать файл экспорта. base — путь папки, относительно которого строятся пути (её родитель) */
export function buildExport(items: Item[], base = ""): ExportFile {
  const conns: ExportedConn[] = [];
  const folders = new Set<string>();
  for (const it of items) {
    const c = parseConn(it);
    if (!c || app.gatewayIds.has(it.id)) continue;
    let rel = app.itemFolderRel(it);
    if (base && (rel === base || rel.startsWith(base + "/"))) {
      const parent = base.includes("/") ? base.slice(0, base.lastIndexOf("/")) : "";
      rel = parent ? rel.slice(parent.length + 1) : rel;
    }
    const path = rel ? rel.split("/") : [];
    for (let i = 1; i <= path.length; i++) folders.add(path.slice(0, i).join("/"));
    const opts: Record<string, string> = {};
    for (const f of it.fields) {
      const n = f.name.trim().toLowerCase();
      // kind 1 — скрытое поле, 3 — связанное (ссылается на логин/пароль)
      if (f.kind === 1 || f.kind === 3 || NEVER.has(n) || !f.value) continue;
      if (!KNOWN_FIELDS.includes(n) && SECRETISH.test(n)) continue;
      opts[n] = f.value;
    }
    conns.push({
      path,
      name: it.name,
      protocol: c.protocol,
      host: c.host,
      port: c.port,
      username: "",
      password: "",
      domain: domainOf(it),
      notes: "",
      opts,
      gateway: c.protocol === "rdp" ? gatewayOf(it) : null,
    });
  }
  return {
    format: EXPORT_FORMAT,
    version: 1,
    exported: new Date().toISOString(),
    folders: [...folders].sort().map((f) => f.split("/")),
    conns,
  };
}

/** Экспортировать записи в файл в «Загрузках» */
export async function exportItems(items: Item[], label: string, base = "") {
  const data = buildExport(items, base);
  if (!data.conns.length) return app.toast("Нет подключений для экспорта", true);
  const date = new Date().toISOString().slice(0, 10);
  const safe = label.replace(/[<>:"/\\|?*]+/g, "_").trim() || "подключения";
  const name = `RDPMgrCoffee — ${safe} — ${date}.json`;
  const text = JSON.stringify(data, null, 2);
  try {
    if (inTauri) {
      const path = await api.saveDownload(name, text);
      app.toast(`Экспортировано подключений: ${data.conns.length} (без логинов и паролей)`);
      api.reveal(path).catch(() => {});
    } else {
      const a = document.createElement("a");
      a.href = URL.createObjectURL(new Blob([text], { type: "application/json" }));
      a.download = name;
      a.click();
      URL.revokeObjectURL(a.href);
      app.toast(`Экспортировано подключений: ${data.conns.length} (без логинов и паролей)`);
    }
  } catch (e) {
    app.toast(errText(e), true);
  }
}

/** Разобрать файл экспорта для импорта. Логины и пароли, если их кто-то дописал, отбрасываются. */
export function parseExport(text: string): { conns: ExportedConn[]; folders: string[][]; skipped: string[] } {
  let d: Partial<ExportFile>;
  try {
    d = JSON.parse(text);
  } catch {
    throw "Файл повреждён: это не JSON";
  }
  if (d.format !== EXPORT_FORMAT || !Array.isArray(d.conns)) throw "Это не файл экспорта RDPMgrCoffee";
  const str = (v: unknown) => (typeof v === "string" ? v : "");
  const conns: ExportedConn[] = [];
  const skipped: string[] = [];
  for (const c of d.conns) {
    const host = str(c?.host).trim();
    const protocol = str(c?.protocol).toLowerCase();
    if (!host || !["rdp", "ssh", "telnet", "serial", "winbox"].includes(protocol)) {
      skipped.push(str(c?.name) || "(без имени)");
      continue;
    }
    const opts: Record<string, string> = {};
    for (const [k, v] of Object.entries(c.opts ?? {})) {
      const n = k.trim().toLowerCase();
      if (typeof v === "string" && !NEVER.has(n) && !(SECRETISH.test(n) && !KNOWN_FIELDS.includes(n))) opts[n] = v;
    }
    const g = c.gateway;
    conns.push({
      path: Array.isArray(c.path) ? c.path.map(str).filter(Boolean) : [],
      name: str(c.name) || host,
      protocol,
      host,
      port: typeof c.port === "number" && c.port > 0 && c.port < 65536 ? c.port : null,
      username: "",
      password: "",
      domain: str(c.domain),
      notes: "",
      opts,
      gateway: g && str(g.host) ? { host: str(g.host), username: "", password: "", domain: str(g.domain), same: !!g.same } : null,
    });
  }
  const folders = Array.isArray(d.folders) ? d.folders.filter(Array.isArray).map((p) => p.map(str).filter(Boolean)) : [];
  return { conns, folders, skipped };
}
