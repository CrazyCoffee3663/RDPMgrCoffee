import { api, errText, fieldValue, parseConn, inTauri, type Item, type Folder } from "./api";
import { app, GATEWAY_FOLDER, type Tool } from "./state.svelte";
import { dialogs, type MenuItem } from "./dialogs.svelte";
import { exportItems } from "./exporter";

export async function copyUsername(item: Item) {
  if (!item.username) return app.toast("Логин не указан", true);
  await api.clipboardSet(item.username);
  app.toast("Логин скопирован");
}

export async function copyPassword(item: Item) {
  try {
    const sec = app.settings.clipboardClearSeconds;
    await api.copySecret(item.id, "password", sec || null);
    app.toast(sec ? `Пароль скопирован, буфер очистится через ${sec} с` : "Пароль скопирован");
  } catch (e) {
    app.toast(errText(e), true);
  }
}

export async function deleteItem(item: Item) {
  const ok = await dialogs.confirm(
    "Удалить запись?",
    `«${item.name}» будет перемещена в корзину Bitwarden. Восстановить её можно в веб-хранилище.`,
    "Удалить",
    true,
  );
  if (!ok) return;
  try {
    await api.remove(item.id);
    app.view.items = app.view.items.filter((i) => i.id !== item.id);
    if (app.editing === item.id) app.editing = null;
    if (app.selected === item.id) app.selected = null;
    app.toast("Запись удалена");
  } catch (e) {
    app.toast(errText(e), true);
  }
}

export async function duplicateItem(item: Item) {
  try {
    const sec = item.hasPassword || item.fields.some((f) => f.value === null) ? await api.secret(item.id) : null;
    const created = await api.save({
      kind: item.kind,
      orgId: item.orgId,
      collectionIds: item.collectionIds,
      folderId: item.folderId,
      name: item.name + " (копия)",
      notes: item.notes,
      username: item.username,
      password: sec?.password ?? null,
      uris: item.uris,
      fields: sec ? sec.fields : item.fields,
      favorite: false,
    });
    app.upsert(created);
    app.selected = created.id;
    app.editing = created.id;
  } catch (e) {
    app.toast(errText(e), true);
  }
}

// ---------- Внешние инструменты и WinBox ----------

/** Сравнение версий вида 4.5 / 3.43 */
export function newerVersion(a: string, b: string): boolean {
  const pa = a.split(".").map(Number);
  const pb = b.split(".").map(Number);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const x = pa[i] ?? 0;
    const y = pb[i] ?? 0;
    if (x !== y) return x > y;
  }
  return false;
}

/** Скачать и установить WinBox — только после подтверждения пользователя */
export async function installWinbox(version: string, update: boolean): Promise<boolean> {
  const ok = await dialogs.confirm(
    update ? `Обновить WinBox до ${version}?` : `Установить WinBox ${version}?`,
    `Будет скачан архив WinBox_Windows.zip (~21 МБ) с официального сайта download.mikrotik.com и распакован в папку программы.`,
    update ? "Обновить" : "Скачать и установить",
  );
  if (!ok) return false;
  app.toast(`Загрузка WinBox ${version}…`);
  try {
    await api.winboxInstall(version);
    app.winboxUpdate = null;
    app.toast(`WinBox ${version} установлен`);
    return true;
  } catch (e) {
    app.toast(errText(e), true);
    return false;
  }
}

/** Проверка обновлений WinBox: раз в сутки (или сразу при force) */
export async function checkWinbox(force = false) {
  const key = "winboxChecked";
  if (!force) {
    if (!app.settings.winboxAutoCheck) return;
    const last = Number(localStorage.getItem(key) ?? 0);
    if (Date.now() - last < 24 * 3600_000) return;
  }
  try {
    const st = await api.winboxStatus();
    localStorage.setItem(key, String(Date.now()));
    if (st.installed && st.latest && (st.installed === "?" || newerVersion(st.latest, st.installed)))
      app.winboxUpdate = { installed: st.installed, latest: st.latest };
    else app.winboxUpdate = null;
    return st;
  } catch {
    return undefined;
  }
}

export async function runTool(item: Item, tool: Tool) {
  if (tool.popup) return openToolWindow(item, tool);
  try {
    const note = await api.toolRun(item.id, tool.command, tool.args);
    if (note) app.toast(note);
  } catch (e) {
    const t = errText(e);
    if (t !== "WINBOX_MISSING") return app.toast(t, true);
    // WinBox ещё не установлен — предложить скачать
    const st = await api.winboxStatus().catch(() => null);
    if (!st?.latest) return app.toast("WinBox не установлен, а сайт MikroTik недоступен", true);
    if (await installWinbox(st.latest, false)) await runTool(item, tool);
  }
}

let toolWinSeq = 0;
/** Отдельное окно в стиле приложения с выводом программы (ping, tracert…) */
async function openToolWindow(item: Item, tool: Tool) {
  const q = new URLSearchParams({
    tool: tool.name,
    item: item.id,
    title: item.name,
    cmd: tool.command,
    args: tool.args,
  }).toString();
  if (!inTauri) {
    window.open("/?" + q, "_blank", "width=760,height=480");
    return;
  }
  const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow");
  const w = new WebviewWindow(`tool-${Date.now().toString(36)}-${++toolWinSeq}`, {
    url: "/?" + q,
    title: `${tool.name} — ${item.name}`,
    width: 760,
    height: 480,
    minWidth: 420,
    minHeight: 220,
    center: true,
    decorations: false,
    shadow: true,
    dragDropEnabled: false,
  });
  w.once("tauri://error", (e) => app.toast(`Не удалось открыть окно: ${errText(e.payload)}`, true));
}

/** Экспорт папки со всеми вложенными (rel — путь относительно рабочей папки) */
export function exportFolder(rel: string) {
  const items = app.connections.filter((i) => {
    const r = app.itemFolderRel(i);
    return r === rel || r.startsWith(rel + "/");
  });
  return exportItems(items, rel.split("/").pop() || "папка", rel);
}

/** Экспорт всех подключений */
export function exportAll() {
  return exportItems(app.connections, "все подключения");
}

// ---------- Действия над несколькими записями ----------

export function multiMenu(items: Item[]): MenuItem[] {
  const conns = items.filter((i) => parseConn(i));
  const editable = items.filter((i) => i.edit);
  return [
    {
      label: `Подключить (${conns.length})`,
      icon: "play",
      action: () => conns.forEach((c) => app.connect(c)),
      disabled: !conns.length,
    },
    {
      label: `Экспорт без паролей (${conns.length})`,
      icon: "download",
      action: () => exportItems(conns, `выбранные (${conns.length})`),
      disabled: !conns.length,
    },
    "sep",
    { label: `Переместить в папку… (${editable.length})`, icon: "folder", action: () => moveMany(editable), disabled: !editable.length },
    {
      label: "В избранное",
      icon: "star",
      action: () => editable.filter((i) => !i.favorite).forEach((i) => toggleFavorite(i)),
      disabled: !editable.some((i) => !i.favorite),
    },
    {
      label: "Убрать из избранного",
      icon: "star",
      action: () => editable.filter((i) => i.favorite).forEach((i) => toggleFavorite(i)),
      disabled: !editable.some((i) => i.favorite),
    },
    "sep",
    { label: `Удалить (${editable.length})`, icon: "trash", danger: true, action: () => deleteMany(editable), disabled: !editable.length, hint: "Del" },
  ];
}

export async function moveMany(items: Item[], folderId?: string | null) {
  if (folderId === undefined) {
    const opts: [string, string][] = [["__root", app.root ? `${app.root} (корень)` : "— Без папки —"]];
    for (const { folder, rel } of app.scopedFolders) opts.push([folder.id, rel]);
    const v = await dialogs.select("Переместить в папку", `Записей: ${items.length}`, opts, "", "Переместить");
    if (v === null) return;
    folderId = v === "__root" ? app.defaultFolderId() : v;
  }
  for (const it of items) await moveItem(it, folderId);
}

export async function deleteMany(items: Item[]) {
  if (items.length === 1) return deleteItem(items[0]);
  const ok = await dialogs.confirm(
    `Удалить записи: ${items.length}?`,
    items.slice(0, 8).map((i) => "• " + i.name).join("\n") + (items.length > 8 ? `\n… и ещё ${items.length - 8}` : "") +
      "\n\nЗаписи уйдут в корзину Bitwarden.",
    "Удалить",
    true,
  );
  if (!ok) return;
  try {
    for (const it of items) {
      await api.remove(it.id);
      app.view.items = app.view.items.filter((i) => i.id !== it.id);
    }
    app.selection = [];
  } catch (e) {
    app.toast(errText(e), true);
  }
}

export async function toggleFavorite(item: Item) {
  try {
    app.upsert(await api.setFavorite(item.id, !item.favorite));
  } catch (e) {
    app.toast(errText(e), true);
  }
}

export async function moveItem(item: Item, folderId: string | null) {
  if (item.folderId === folderId) return;
  try {
    app.upsert(await api.move(item.id, folderId));
  } catch (e) {
    app.toast(errText(e), true);
  }
}

export async function newFolder(parent: string | null) {
  const name = await dialogs.input(
    "Новая папка",
    parent ? `Будет создана внутри «${parent}»` : "Вложенность задаётся через «/», например «Клиенты/Офис»",
    "",
    "Создать",
  );
  if (!name) return;
  try {
    const f = await api.saveFolder(null, app.fullName(parent ? `${parent}/${name.trim()}` : name.trim()));
    app.view.folders.push(f);
    app.toggleExpanded("f:" + app.relName(f.name), true);
  } catch (e) {
    app.toast(errText(e), true);
  }
}

export async function renameFolder(folder: Folder) {
  const parts = folder.name.split("/");
  const last = parts.pop()!;
  const name = await dialogs.input("Переименовать папку", "", last, "Сохранить");
  if (!name || name.trim() === last) return;
  const prefix = parts.length ? parts.join("/") + "/" : "";
  const oldFull = folder.name;
  const newFull = prefix + name.trim();
  try {
    // Вложенные папки в Bitwarden — это просто имена с «/», переименовываем и их
    const affected = app.view.folders.filter((f) => f.name === oldFull || f.name.startsWith(oldFull + "/"));
    for (const f of affected) {
      const updated = await api.saveFolder(f.id, newFull + f.name.slice(oldFull.length));
      f.name = updated.name;
    }
  } catch (e) {
    app.toast(errText(e), true);
  }
}

/**
 * Перенести папку (путь относительно рабочей) внутрь другой; newParent "" — в корень.
 * В Bitwarden вложенность — это имя с «/», поэтому переименовываем папку и все вложенные.
 */
export async function moveFolder(path: string, newParent: string): Promise<string | null> {
  const name = path.split("/").pop()!;
  const target = newParent ? `${newParent}/${name}` : name;
  if (target === path) return path;
  if (newParent === path || newParent.startsWith(path + "/")) return null;
  const oldFull = app.fullName(path);
  const newFull = app.fullName(target);
  if (app.view.folders.some((f) => f.name === newFull || f.name.startsWith(newFull + "/"))) {
    app.toast(`Папка «${target}» уже существует`, true);
    return null;
  }
  const affected = app.view.folders.filter((f) => f.name === oldFull || f.name.startsWith(oldFull + "/"));
  try {
    for (const f of affected) {
      const updated = await api.saveFolder(f.id, newFull + f.name.slice(oldFull.length));
      f.name = updated.name;
    }
    // Перенести состояние «раскрыта» и «скрыта»
    const re = (p: string) => (p === path || p.startsWith(path + "/") ? target + p.slice(path.length) : p);
    app.expanded = app.expanded.map((k) => (k.startsWith("f:") ? "f:" + re(k.slice(2)) : k));
    if (newParent) app.toggleExpanded("f:" + newParent, true);
    app.toggleExpanded("f:" + target, app.expanded.includes("f:" + target)); // сохранить
    app.settings.hiddenFolders = app.settings.hiddenFolders.map(re);
    const rk = (k: string) => (k.startsWith("f:") ? "f:" + re(k.slice(2)) : k);
    app.settings.order = Object.fromEntries(Object.entries(app.settings.order).map(([k, v]) => [rk(k), v.map(rk)]));
    app.saveSettings();
    return target;
  } catch (e) {
    app.toast(errText(e), true);
    return null;
  }
}

/** Папка по полному имени: найти или создать. */
export async function ensureFolder(full: string): Promise<string> {
  const ex = app.view.folders.find((f) => f.name === full);
  if (ex) return ex.id;
  const f = await api.saveFolder(null, full);
  app.view.folders.push(f);
  return f.id;
}

// ---------- RD-шлюзы: дубли и папка RDG ----------

/** Адрес шлюза без схемы, пути и порта 443, в нижнем регистре */
export function normGwHost(addr: string): string {
  return addr
    .trim()
    .toLowerCase()
    .replace(/^[a-z]+:\/\//, "")
    .replace(/[/?#].*$/, "")
    .replace(/:443$/, "");
}

/** Ключ для поиска одинаковых шлюзов: адрес + учётные данные (или «те же, что у подключения») */
export function gwKey(host: string, same: boolean, username = "", domain = "", password = ""): string {
  if (!same && !username.trim() && !password) same = true;
  if (!same && username.includes("\\") && !domain) [domain, username] = username.split("\\", 2);
  return same
    ? `${normGwHost(host)}#same`
    : `${normGwHost(host)}#${username.trim().toLowerCase()}|${domain.trim().toLowerCase()}|${password}`;
}

/** Ключ существующей записи шлюза (читает пароль из хранилища) */
export async function itemGwKey(g: Item): Promise<string> {
  const same = (fieldValue(g, "samecreds") ?? "").toLowerCase() === "true";
  const pw = !same && g.hasPassword ? (await api.secret(g.id)).password : "";
  return gwKey(g.uris[0] ?? "", same, g.username, fieldValue(g, "domain") ?? "", pw);
}

/** Объединить одинаковые шлюзы и сложить все шлюзы в папку RDG */
export async function tidyGateways() {
  try {
    const gws = app.gateways.filter((g) => g.edit);
    const groups = new Map<string, Item[]>();
    for (const g of gws) {
      const k = await itemGwKey(g);
      groups.set(k, [...(groups.get(k) ?? []), g]);
    }
    const refs = (g: Item) =>
      app.view.items.filter((c) => {
        const v = fieldValue(c, "gateway");
        return v === g.id || v === g.name;
      });
    const dups = [...groups.values()].reduce((s, l) => s + l.length - 1, 0);
    const rdgFull = app.fullName(GATEWAY_FOLDER);
    const rdgId = app.view.folders.find((f) => f.name === rdgFull)?.id;
    const keepers = [...groups.values()].map((l) => [...l].sort((a, b) => refs(b).length - refs(a).length)[0]);
    const toMove = keepers.filter((g) => !rdgId || g.folderId !== rdgId).length;
    if (!dups && !toMove) return app.toast("Шлюзы уже в порядке");
    const ok = await dialogs.confirm(
      "Навести порядок в RD-шлюзах?",
      [
        dups ? `Одинаковых шлюзов: ${dups} — они будут объединены, подключения переключатся на оставшийся, лишние уйдут в корзину Bitwarden.` : "",
        toMove ? `Шлюзов будет перенесено в папку «${GATEWAY_FOLDER}» (скрыта в дереве): ${toMove}.` : "",
      ]
        .filter(Boolean)
        .join("\n"),
      "Выполнить",
    );
    if (!ok) return;
    const folderId = await ensureFolder(rdgFull);
    let merged = 0;
    for (const list of groups.values()) {
      const keep = [...list].sort((a, b) => refs(b).length - refs(a).length)[0];
      for (const g of list) {
        if (g.id === keep.id) continue;
        for (const c of refs(g)) {
          const saved = await api.save({
            id: c.id, kind: c.kind, orgId: c.orgId, collectionIds: c.collectionIds, folderId: c.folderId,
            name: c.name, notes: c.notes, username: c.username, password: null, uris: c.uris,
            fields: c.fields.map((f) => (f.name.trim().toLowerCase() === "gateway" ? { ...f, value: keep.id } : f)),
            favorite: c.favorite,
          });
          app.upsert(saved);
        }
        await api.remove(g.id);
        app.view.items = app.view.items.filter((i) => i.id !== g.id);
        merged++;
      }
      if (keep.folderId !== folderId) app.upsert(await api.move(keep.id, folderId));
    }
    app.toast(merged ? `Объединено шлюзов: ${merged}` : "Шлюзы перенесены в папку " + GATEWAY_FOLDER);
  } catch (e) {
    app.toast(errText(e), true);
  }
}

export async function deleteFolder(folder: Folder) {
  // Сама папка и все вложенные («Папка/…»)
  const folders = app.view.folders.filter((f) => f.name === folder.name || f.name.startsWith(folder.name + "/"));
  const ids = new Set(folders.map((f) => f.id));
  const inside = app.view.items.filter((i) => i.folderId && ids.has(i.folderId));
  const sub = folders.length - 1;
  const what = [sub ? `вложенных папок: ${sub}` : "", inside.length ? `записей: ${inside.length}` : ""].filter(Boolean).join(", ");
  let withItems = false;
  if (inside.length) {
    const r = await dialogs.choose(
      `Удалить папку «${app.relName(folder.name) || folder.name}»?`,
      `Внутри ${what}.\n\n«Удалить всё» — записи уйдут в корзину Bitwarden (их можно восстановить в течение 30 дней).\n«Только папки» — записи переместятся в ${app.root ? `«${app.root}»` : "корень"}.`,
      `Удалить всё (${inside.length})`,
      "Только папки",
      true,
    );
    if (!r) return;
    withItems = r === "ok";
  } else if (!(await dialogs.confirm("Удалить папку?", `Папка «${folder.name}»${sub ? ` и ${what}` : ""} будет удалена.`, "Удалить", true))) {
    return;
  }
  try {
    const fallback = app.defaultFolderId();
    for (const it of inside) {
      if (withItems) {
        if (!it.edit) continue;
        await api.remove(it.id);
        app.view.items = app.view.items.filter((i) => i.id !== it.id);
      } else if (fallback && it.edit) {
        app.upsert(await api.move(it.id, fallback));
      }
    }
    // Сначала вложенные, потом родителя
    for (const f of [...folders].sort((a, b) => b.name.length - a.name.length)) {
      await api.deleteFolder(f.id);
      app.view.folders = app.view.folders.filter((x) => x.id !== f.id);
      for (const i of app.view.items) if (i.folderId === f.id) i.folderId = null;
    }
    if (withItems) app.toast(`Удалено записей: ${inside.filter((i) => i.edit).length}`);
  } catch (e) {
    app.toast(errText(e), true);
  }
}

export function newItem(kind: "conn" | "login" | "gateway", folderId: string | null = null) {
  app.editingFolder = folderId ?? app.defaultFolderId();
  app.editing = "new:" + kind;
}

/** Создать рабочую папку, если её ещё нет. */
export async function createRootFolder(): Promise<string | null> {
  if (!app.root) return null;
  if (app.rootFolder) return app.rootFolder.id;
  try {
    const f = await api.saveFolder(null, app.root);
    app.view.folders.push(f);
    return f.id;
  } catch (e) {
    app.toast(errText(e), true);
    return null;
  }
}

/**
 * Перенести подключения из остального хранилища в рабочую папку,
 * вместе с записями, на которые они ссылаются (учётки, шлюзы, SSH-ключи).
 * Структура папок сохраняется: «Офис/Серверы» → «<рабочая>/Офис/Серверы».
 */
export function needsMigration(): number {
  if (!app.root) return 0;
  let n = app.outsideConnections.length;
  for (const c of app.connections) {
    for (const k of ["credential", "gateway", "sshkey", "jump"]) {
      const v = c.fields.find((f) => f.name.trim().toLowerCase() === k)?.value;
      const ref = v ? app.item(v) ?? app.view.items.find((i) => i.name === v) : undefined;
      if (ref && !app.inScope(ref)) n++;
    }
  }
  return n;
}

export async function migrateToRoot() {
  const rootId = await createRootFolder();
  if (!rootId) return;
  const conns = app.outsideConnections;
  const move = new Map<string, Item>(conns.map((c) => [c.id, c]));
  // Ссылки проверяем у всех подключений: и переносимых, и уже лежащих в рабочей папке
  for (const c of [...conns, ...app.connections]) {
    for (const k of ["credential", "gateway", "sshkey", "jump"]) {
      const v = c.fields.find((f) => f.name.trim().toLowerCase() === k)?.value;
      if (!v) continue;
      const ref = app.item(v) ?? app.view.items.find((i) => i.name === v);
      if (ref && !app.inScope(ref)) move.set(ref.id, ref);
    }
  }
  if (!move.size) return app.toast("Всё уже в рабочей папке");
  const ok = await dialogs.confirm(
    "Перенести записи в рабочую папку?",
    `Подключений: ${conns.length}, связанных учёток и шлюзов: ${move.size - conns.length}.
` +
      `Они будут перемещены в папку «${app.root}» с сохранением вложенных папок. ` +
      "Содержимое записей не меняется, остальное хранилище не затрагивается.",
    "Перенести",
  );
  if (!ok) return;
  const folderIds = new Map(app.view.folders.map((f) => [f.name, f.id]));
  let done = 0;
  try {
    for (const it of move.values()) {
      const old = it.folderId ? app.folderName.get(it.folderId) ?? "" : "";
      const target = old ? `${app.root}/${old}` : app.root;
      let fid = folderIds.get(target);
      if (!fid) {
        const f = await api.saveFolder(null, target);
        app.view.folders.push(f);
        folderIds.set(f.name, f.id);
        fid = f.id;
      }
      app.upsert(await api.move(it.id, fid));
      done++;
    }
    app.toast(`Перенесено записей: ${done}`);
  } catch (e) {
    app.toast(`Перенесено ${done} из ${move.size}. ${errText(e)}`, true);
  }
}

export function itemMenu(item: Item): MenuItem[] {
  const conn = parseConn(item);
  const m: MenuItem[] = [];
  if (conn) m.push({ label: "Подключиться", icon: "play", action: () => app.connect(item), hint: "Enter" });
  if (app.gatewayIds.has(item.id)) {
    const users = app.connections.filter((c) => {
      const g = c.fields.find((f) => f.name.trim().toLowerCase() === "gateway")?.value;
      return g === item.id || g === item.name;
    }).length;
    m.push({ label: `Шлюз для подключений: ${users}`, icon: "building", action: () => {}, disabled: true });
  }
  m.push({ label: "Изменить", icon: "edit", action: () => (app.editing = item.id), disabled: !item.edit, hint: "F2" });
  m.push({ label: "Дублировать", icon: "copy", action: () => duplicateItem(item), disabled: item.kind !== 1 });
  m.push("sep");
  m.push({ label: "Копировать логин", icon: "user", action: () => copyUsername(item), disabled: !item.username });
  m.push({ label: "Копировать пароль", icon: "key", action: () => copyPassword(item), disabled: !item.hasPassword });
  if (conn) m.push({ label: "Копировать адрес", icon: "copy", action: () => api.clipboardSet(conn.host) });
  if (conn) m.push({ label: "Экспорт без пароля", icon: "download", action: () => exportItems([item], item.name) });
  m.push("sep");
  m.push({
    label: item.favorite ? "Убрать из избранного" : "В избранное",
    icon: "star",
    action: () => toggleFavorite(item),
    disabled: !item.edit,
  });
  m.push({ label: "Удалить", icon: "trash", danger: true, action: () => deleteItem(item), disabled: !item.edit, hint: "Del" });
  if (conn) {
    const tools = app.settings.tools.filter((t) => !t.protocols.length || t.protocols.includes(conn.protocol));
    if (tools.length) m.push("sep");
    for (const t of tools) m.push({ label: t.name, icon: t.command === "winbox" ? "cable" : t.command === "url" ? "cloud" : "terminal", action: () => runTool(item, t) });
  }
  return m;
}
