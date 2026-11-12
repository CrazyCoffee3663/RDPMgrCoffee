<script lang="ts">
  import { parseConn, PROTOCOL_LABEL, type Item, type Folder } from "./api";
  import { app } from "./state.svelte";
  import { menu } from "./dialogs.svelte";
  import * as act from "./actions";
  import Icon from "./Icon.svelte";

  type Node = {
    key: string;
    name: string;
    path: string;
    folder: Folder | null;
    org: boolean;
    hidden: boolean;
    children: Node[];
    items: Item[];
    count: number;
  };

  let query = $state("");
  let searchEl = $state<HTMLInputElement>();
  let treeEl = $state<HTMLDivElement>();
  let dragOver = $state<string | null>(null);
  /** Куда вставится перетаскиваемое: до/после строки или внутрь папки */
  let dropPos = $state<"before" | "after" | "into">("into");

  const source = $derived(app.mode === "conn" ? app.connections : app.logins);

  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return source;
    return source.filter(
      (i) =>
        i.name.toLowerCase().includes(q) ||
        i.username.toLowerCase().includes(q) ||
        i.uris.some((u) => u.toLowerCase().includes(q)) ||
        i.notes.toLowerCase().includes(q),
    );
  });

  const tree = $derived.by(() => {
    const root: Node = { key: "root", name: "", path: "", folder: null, org: false, hidden: false, children: [], items: [], count: 0 };
    const showHidden = app.showHidden;
    const byPath = new Map<string, Node>();
    const ensure = (path: string): Node => {
      if (!path) return root;
      let n = byPath.get(path);
      if (n) return n;
      const i = path.lastIndexOf("/");
      const parent = ensure(i >= 0 ? path.slice(0, i) : "");
      n = {
        key: "f:" + path,
        name: i >= 0 ? path.slice(i + 1) : path,
        path,
        folder: null,
        org: false,
        hidden: app.isHidden(path),
        children: [],
        items: [],
        count: 0,
      };
      parent.children.push(n);
      byPath.set(path, n);
      return n;
    };
    for (const { folder, rel } of app.scopedFolders) if (showHidden || !app.isHidden(rel)) ensure(rel).folder = folder;
    const folderPath = new Map(
      app.view.folders.filter((f) => app.inScopeName(f.name)).map((f) => [f.id, app.relName(f.name)] as [string, string]),
    );
    const orgName = new Map(app.view.orgs.map((o) => [o.id, o.name]));
    const colName = new Map(app.view.collections.map((c) => [c.id, c.name]));
    const orgNodes = new Map<string, Node>();

    for (const it of filtered) {
      const fp = it.folderId ? folderPath.get(it.folderId) : undefined;
      if (fp !== undefined) {
        if (!showHidden && app.isHidden(fp)) continue;
        ensure(fp).items.push(it);
      } else if (it.orgId) {
        // Записи организации без личной папки — группируем по коллекции
        const col = it.collectionIds.map((c) => colName.get(c)).find(Boolean) ?? "";
        const label = [orgName.get(it.orgId) ?? "Организация", col].filter(Boolean).join(" / ");
        let n = orgNodes.get(label);
        if (!n) {
          n = { key: "o:" + label, name: label, path: "", folder: null, org: true, hidden: false, children: [], items: [], count: 0 };
          orgNodes.set(label, n);
        }
        n.items.push(it);
      } else {
        root.items.push(it);
      }
    }
    const ord = app.settings.order ?? {};
    const rank = (list: string[] | undefined, k: string) => {
      const i = list ? list.indexOf(k) : -1;
      return i < 0 ? Infinity : i;
    };
    const sortNode = (n: Node): number => {
      const o = ord[n.key];
      n.children.sort((a, b) => rank(o, a.key) - rank(o, b.key) || a.name.localeCompare(b.name, "ru"));
      n.items.sort((a, b) => rank(o, a.id) - rank(o, b.id) || a.name.localeCompare(b.name, "ru"));
      n.count = n.items.length + n.children.reduce((s, c) => s + sortNode(c), 0);
      return n.count;
    };
    root.children.push(...[...orgNodes.values()].sort((a, b) => a.name.localeCompare(b.name, "ru")));
    sortNode(root);
    // При поиске прячем пустые папки
    if (query.trim()) {
      const prune = (n: Node) => {
        n.children = n.children.filter((c) => c.count > 0);
        n.children.forEach(prune);
      };
      prune(root);
    }
    return root;
  });

  // Плоский список видимых строк — для клавиатурной навигации
  type Row =
    | { kind: "node"; node: Node; depth: number; parent: Node }
    | { kind: "item"; item: Item; depth: number; parent: Node };
  const rows = $derived.by(() => {
    const out: Row[] = [];
    const searching = !!query.trim();
    const walk = (n: Node, depth: number) => {
      for (const c of n.children) {
        out.push({ kind: "node", node: c, depth, parent: n });
        if (searching || app.expanded.includes(c.key)) walk(c, depth + 1);
      }
      for (const it of n.items) out.push({ kind: "item", item: it, depth, parent: n });
    };
    walk(tree, 0);
    return out;
  });

  const liveItems = $derived(
    new Map(app.tabs.filter((t) => t.itemId).map((t) => [t.itemId!, t.state] as [string, string])),
  );

  function onItemKey(e: KeyboardEvent) {
    const idx = rows.findIndex((r) => r.kind === "item" && r.item.id === app.selected);
    const sel = app.item(app.selected);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const dir = e.key === "ArrowDown" ? 1 : -1;
      for (let i = idx + dir; i >= 0 && i < rows.length; i += dir) {
        const r = rows[i];
        if (r.kind === "item") {
          app.selected = r.item.id;
          document.getElementById("row-" + r.item.id)?.scrollIntoView({ block: "nearest" });
          break;
        }
      }
    } else if (e.key === "Enter" && sel) {
      e.preventDefault();
      if (parseConn(sel)) app.connect(sel);
      else app.editing = sel.id;
    } else if (e.key === "F2" && sel) {
      app.editing = sel.id;
    } else if (e.key === "Delete" && sel) {
      act.deleteItem(sel);
    }
  }

  function folderMenu(e: MouseEvent, n: Node) {
    const f = n.folder;
    menu.show(e, [
      {
        label: app.mode === "conn" ? "Новое подключение здесь" : "Новый логин здесь",
        icon: "plus",
        action: () => act.newItem(app.mode === "conn" ? "conn" : "login", f?.id ?? null),
        disabled: !f,
      },
      {
        label: "Новый RD-шлюз здесь",
        icon: "building",
        action: () => act.newItem("gateway", f?.id ?? null),
        disabled: !f || app.mode !== "logins",
      },
      { label: "Новая папка внутри", icon: "folderPlus", action: () => act.newFolder(n.path), disabled: n.org },
      "sep",
      n.hidden
        ? { label: "Показывать в дереве", icon: "eye", action: () => app.setHidden(n.path, false), disabled: !app.settings.hiddenFolders.includes(n.path) }
        : { label: "Скрыть папку", icon: "eyeOff", action: () => app.setHidden(n.path, true), disabled: n.org },
      { label: "Переместить в корень", icon: "folder", action: () => act.moveFolder(n.path, ""), disabled: n.org || !n.path.includes("/") },
      { label: "Переименовать", icon: "edit", action: () => f && act.renameFolder(f), disabled: !f },
      { label: "Удалить папку", icon: "trash", danger: true, action: () => f && act.deleteFolder(f), disabled: !f },
    ]);
  }

  function rootMenu(e: MouseEvent) {
    menu.show(e, [
      {
        label: app.mode === "conn" ? "Новое подключение" : "Новый логин",
        icon: "plus",
        action: () => act.newItem(app.mode === "conn" ? "conn" : "login"),
      },
      { label: "Новый RD-шлюз", icon: "building", action: () => act.newItem("gateway") },
      { label: "Новая папка", icon: "folderPlus", action: () => act.newFolder(null) },
      { label: "Объединить дубли шлюзов", icon: "building", action: () => act.tidyGateways(), disabled: !app.gateways.length },
      "sep",
      {
        label: app.showHidden ? "Не показывать скрытые папки" : `Показать скрытые папки (${app.settings.hiddenFolders.length})`,
        icon: app.showHidden ? "eyeOff" : "eye",
        action: () => (app.showHidden = !app.showHidden),
        disabled: !app.settings.hiddenFolders.length,
      },
    ]);
  }

  // ---- перетаскивание: перенос в папку и ручной порядок ----
  let drag: { kind: "folder"; path: string } | { kind: "item"; id: string } | null = null;
  const parentOf = (path: string) => (path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "");

  function onDragStart(e: DragEvent, it: Item) {
    if (!it.edit) return e.preventDefault();
    drag = { kind: "item", id: it.id };
    e.dataTransfer!.setData("text/plain", it.name);
    e.dataTransfer!.effectAllowed = "move";
  }
  function onFolderDragStart(e: DragEvent, n: Node) {
    if (n.org) return e.preventDefault();
    e.stopPropagation();
    drag = { kind: "folder", path: n.path };
    e.dataTransfer!.setData("text/plain", n.name);
    e.dataTransfer!.effectAllowed = "move";
  }
  function onDragEnd() {
    drag = null;
    dragOver = null;
  }

  /** Можно ли бросить на строку r (или в корень, r = null) в позицию pos */
  function allowed(r: Row | null, pos: "before" | "after" | "into"): boolean {
    if (!drag) return false;
    if (!r) return drag.kind === "item" || parentOf(drag.path) !== "";
    if (r.parent.org) return false;
    if (drag.kind === "folder") {
      if (r.kind !== "node" || r.node.org) return false;
      const p = r.node.path;
      if (p === drag.path || p.startsWith(drag.path + "/")) return false;
      return pos !== "into" || p !== parentOf(drag.path);
    }
    if (pos === "into") return r.kind === "node" && !!r.node.folder;
    if (r.kind !== "item" || r.item.id === drag.id) return false;
    return r.parent.key === "root" || !!r.parent.folder;
  }

  function rowDragOver(e: DragEvent, r: Row) {
    if (!drag) return;
    const el = e.currentTarget as HTMLElement;
    const b = el.getBoundingClientRect();
    const f = (e.clientY - b.top) / b.height;
    let pos: "before" | "after" | "into" =
      r.kind === "node" ? (f < 0.28 ? "before" : f > 0.72 ? "after" : "into") : f < 0.5 ? "before" : "after";
    // Папку на папку без порядка — внутрь; запись на папку — внутрь
    if (drag.kind === "item" && r.kind === "node") pos = "into";
    if (!allowed(r, pos)) {
      if (pos !== "into" && allowed(r, "into")) pos = "into";
      else return;
    }
    e.preventDefault();
    e.stopPropagation();
    dragOver = r.kind === "node" ? r.node.key : "i:" + r.item.id;
    dropPos = pos;
  }

  function rootDragOver(e: DragEvent) {
    if (!allowed(null, "into")) return;
    e.preventDefault();
    dragOver = "root";
    dropPos = "into";
  }

  /** Записать порядок детей родителя: ключ key ставим до/после ref */
  function setOrder(parent: Node, keys: string[], key: string, ref: string, pos: "before" | "after") {
    const list = keys.filter((k) => k !== key);
    const i = list.indexOf(ref);
    list.splice(i < 0 ? list.length : pos === "before" ? i : i + 1, 0, key);
    // Папки и записи упорядочиваются отдельно — сохраняем порядок другой группы
    const other = (app.settings.order[parent.key] ?? []).filter((k) => k !== key && !keys.includes(k));
    app.settings.order = { ...app.settings.order, [parent.key]: [...other, ...list] };
    app.saveSettings();
  }

  async function rowDrop(e: DragEvent, r: Row | null) {
    e.preventDefault();
    e.stopPropagation();
    const d = drag;
    const pos = dropPos;
    const ok = !!d && allowed(r, r ? pos : "into");
    onDragEnd();
    if (!d || !ok) return;
    if (d.kind === "folder") {
      if (!r) return void act.moveFolder(d.path, "");
      const node = (r as Extract<Row, { kind: "node" }>).node;
      if (pos === "into") return void act.moveFolder(d.path, node.path);
      const siblings = r.parent.children.map((c) => c.key);
      const newPath = await act.moveFolder(d.path, r.parent.path);
      if (newPath) setOrder(r.parent, siblings, "f:" + newPath, node.key, pos);
      return;
    }
    const it = app.item(d.id);
    if (!it) return;
    if (!r) return void act.moveItem(it, app.defaultFolderId());
    if (pos === "into" && r.kind === "node") return void act.moveItem(it, r.node.folder!.id);
    if (r.kind !== "item" || pos === "into") return;
    const targetFolder = r.parent.folder?.id ?? app.defaultFolderId();
    const siblings = r.parent.items.map((i) => i.id);
    if (it.folderId !== targetFolder) await act.moveItem(it, targetFolder);
    setOrder(r.parent, siblings, it.id, r.item.id, pos);
  }

  export function focusSearch() {
    searchEl?.focus();
    searchEl?.select();
  }

  function subtitle(it: Item): string {
    const c = parseConn(it);
    if (c) return c.host + (c.port ? ":" + c.port : "");
    if (it.kind === 5) return "SSH-ключ";
    if (app.gatewayIds.has(it.id)) return (it.uris[0] ?? "").replace(/^[a-z]+:\/\//i, "");
    return it.username || it.uris[0] || "";
  }
</script>

<aside class="sidebar" style="width:{app.settings.sidebarWidth}px">
  <div class="modes">
    <button class:on={app.mode === "conn"} onclick={() => (app.mode = "conn")}>
      <Icon name="monitor" size={14} /> Подключения <span class="cnt">{app.connections.length}</span>
    </button>
    <button class:on={app.mode === "logins"} onclick={() => (app.mode = "logins")}>
      <Icon name="key" size={14} /> Логины <span class="cnt">{app.logins.length}</span>
    </button>
  </div>

  <div class="tools">
    <div class="search">
      <Icon name="search" size={14} />
      <input
        bind:this={searchEl}
        bind:value={query}
        placeholder="Поиск…"
        onkeydown={(e) => {
          if (e.key === "Escape") query = "";
          if (e.key === "ArrowDown") {
            e.preventDefault();
            const first = rows.find((r) => r.kind === "item");
            if (first && first.kind === "item") app.selected = first.item.id;
            treeEl?.focus();
          }
          if (e.key === "Enter") {
            const first = rows.find((r) => r.kind === "item");
            if (first && first.kind === "item") {
              app.selected = first.item.id;
              if (parseConn(first.item)) app.connect(first.item);
              else app.editing = first.item.id;
            }
          }
        }}
      />
      {#if query}<button class="clear" onclick={() => (query = "")}><Icon name="x" size={12} /></button>{/if}
    </div>
    <button
      class="icon-btn"
      title={app.mode === "conn" ? "Новое подключение (Ctrl+N)" : "Новый логин (Ctrl+N)"}
      onclick={() => act.newItem(app.mode === "conn" ? "conn" : "login")}><Icon name="plus" /></button
    >
    <button class="icon-btn" title="Новая папка" onclick={() => act.newFolder(null)}><Icon name="folderPlus" /></button>
    {#if app.settings.hiddenFolders.length}
      <button
        class="icon-btn"
        class:on={app.showHidden}
        title={app.showHidden ? "Не показывать скрытые папки" : "Показать скрытые папки"}
        onclick={() => (app.showHidden = !app.showHidden)}><Icon name={app.showHidden ? "eye" : "eyeOff"} /></button
      >
    {/if}
  </div>

  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div
    class="tree"
    class:drop={dragOver === "root"}
    bind:this={treeEl}
    tabindex="0"
    role="tree"
    onkeydown={onItemKey}
    oncontextmenu={rootMenu}
    ondragover={rootDragOver}
    ondragleave={() => dragOver === "root" && (dragOver = null)}
    ondrop={(e) => rowDrop(e, null)}
  >
    {#each rows as r (r.kind === "node" ? r.node.key : r.item.id)}
      {#if r.kind === "node"}
        {@const n = r.node}
        {@const open = !!query.trim() || app.expanded.includes(n.key)}
        <div
          class="row folder"
          class:drop={dragOver === n.key && dropPos === "into"}
          class:drop-before={dragOver === n.key && dropPos === "before"}
          class:drop-after={dragOver === n.key && dropPos === "after"}
          class:hiddenf={n.hidden}
          style="padding-left:{8 + r.depth * 16}px"
          draggable={!n.org}
          ondragstart={(e) => onFolderDragStart(e, n)}
          ondragend={onDragEnd}
          role="treeitem"
          aria-selected="false"
          aria-expanded={open}
          tabindex="-1"
          onclick={() => app.toggleExpanded(n.key)}
          onkeydown={() => {}}
          oncontextmenu={(e) => folderMenu(e, n)}
          ondragover={(e) => rowDragOver(e, r)}
          ondragleave={() => dragOver === n.key && (dragOver = null)}
          ondrop={(e) => rowDrop(e, r)}
        >
          <span class="chev" class:open><Icon name="chevron" size={12} /></span>
          <span class="ficon"><Icon name={n.org ? "building" : open ? "folderOpen" : "folder"} size={15} /></span>
          <span class="name">{n.name}</span>
          {#if n.hidden}<span class="ficon" title="Скрытая папка"><Icon name="eyeOff" size={12} /></span>{/if}
          <span class="cnt">{n.count}</span>
        </div>
      {:else}
        {@const it = r.item}
        {@const c = parseConn(it)}
        {@const live = liveItems.get(it.id)}
        <div
          id="row-{it.id}"
          class="row item {c ? 'proto-' + c.protocol : ''}"
          class:sel={app.selected === it.id}
          style="padding-left:{8 + r.depth * 16 + 14}px"
          role="treeitem"
          aria-selected={app.selected === it.id}
          tabindex="-1"
          class:drop-before={dragOver === "i:" + it.id && dropPos === "before"}
          class:drop-after={dragOver === "i:" + it.id && dropPos === "after"}
          draggable={it.edit}
          ondragstart={(e) => onDragStart(e, it)}
          ondragend={onDragEnd}
          ondragover={(e) => rowDragOver(e, r)}
          ondragleave={() => dragOver === "i:" + it.id && (dragOver = null)}
          ondrop={(e) => rowDrop(e, r)}
          onclick={() => {
            app.selected = it.id;
            treeEl?.focus();
            if (app.editing && app.editing !== it.id && !app.editing.startsWith("new:")) app.editing = it.id;
          }}
          ondblclick={() => (c ? app.connect(it) : (app.editing = it.id))}
          onkeydown={() => {}}
          oncontextmenu={(e) => {
            app.selected = it.id;
            menu.show(e, act.itemMenu(it));
          }}
        >
          {#if c}
            <span class="badge">{PROTOCOL_LABEL[c.protocol]}</span>
          {:else if app.gatewayIds.has(it.id)}
            <span class="badge gw" title="RD-шлюз">GW</span>
          {:else}
            <span class="ficon"><Icon name={it.kind === 5 ? "key" : "user"} size={14} /></span>
          {/if}
          <span class="name">{it.name}</span>
          {#if it.favorite}<span class="fav"><Icon name="star" size={11} /></span>{/if}
          {#if live}<span class="dot {live}" title={live === "connected" ? "Подключено" : live}></span>{/if}
          <span class="sub">{subtitle(it)}</span>
        </div>
      {/if}
    {/each}

    {#if rows.length === 0}
      <div class="empty">
        {#if query}
          Ничего не найдено
        {:else if app.root && !app.rootFolder}
          <p>Рабочая папка «{app.root}» не найдена в Bitwarden.</p>
          <button class="btn primary" onclick={() => act.createRootFolder()}><Icon name="folderPlus" size={14} /> Создать её</button>
        {:else if app.mode === "conn"}
          <p>Подключений пока нет.</p>
          <button class="btn primary" onclick={() => act.newItem("conn")}><Icon name="plus" size={14} /> Новое подключение</button>
          <p class="faint">
            Подключение — это запись Bitwarden с адресом вида <code>rdp://</code>, <code>ssh://</code>,
            <code>telnet://</code> или <code>serial://</code>.
          </p>
        {:else}
          <p>Логинов пока нет.</p>
          <button class="btn primary" onclick={() => act.newItem("login")}><Icon name="plus" size={14} /> Новый логин</button>
        {/if}
      </div>
    {/if}
  </div>
</aside>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    background: var(--bg-panel);
    border-right: 1px solid var(--border);
    min-width: 220px;
    max-width: 600px;
    flex-shrink: 0;
  }
  .modes {
    display: flex;
    gap: 2px;
    padding: 8px 8px 0;
  }
  .modes button {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 6px;
    color: var(--text-muted);
    border-radius: var(--radius-sm);
  }
  .modes button:hover {
    background: var(--bg-hover);
  }
  .modes button.on {
    background: var(--accent-soft);
    color: var(--accent);
    font-weight: 600;
  }
  .cnt {
    font-size: 11px;
    color: var(--text-faint);
    font-weight: 400;
  }
  .tools {
    display: flex;
    gap: 2px;
    padding: 8px;
    align-items: center;
  }
  .search {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 8px;
    background: var(--bg-sunken);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text-faint);
  }
  .search:focus-within {
    border-color: var(--accent);
  }
  .search input {
    border: 0;
    background: none;
    padding: 5px 0;
    box-shadow: none !important;
  }
  .clear {
    display: flex;
    padding: 2px;
    color: var(--text-faint);
  }
  .tree {
    flex: 1;
    overflow: auto;
    padding: 2px 4px 12px;
    outline: none;
  }
  .tree.drop {
    background: var(--accent-soft);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding-right: 8px;
    border-radius: var(--radius-sm);
    cursor: default;
    white-space: nowrap;
    overflow: hidden;
  }
  .row:hover {
    background: var(--bg-hover);
  }
  .row.sel {
    background: var(--bg-active);
  }
  .tree:focus .row.sel {
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .row.drop-before {
    box-shadow: inset 0 2px 0 var(--accent);
  }
  .row.drop-after {
    box-shadow: inset 0 -2px 0 var(--accent);
  }
  .row.drop {
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .folder {
    font-weight: 500;
  }
  .hiddenf {
    opacity: 0.55;
  }
  .icon-btn.on {
    color: var(--accent);
  }
  .chev {
    display: inline-flex;
    color: var(--text-faint);
    transition: transform 0.12s;
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .ficon {
    display: inline-flex;
    color: var(--text-muted);
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    flex-shrink: 1;
  }
  .sub {
    margin-left: auto;
    padding-left: 8px;
    font-size: 11px;
    color: var(--text-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    flex-shrink: 2;
    min-width: 0;
  }
  .folder .cnt {
    margin-left: auto;
  }
  .badge {
    font-size: 9.5px;
    font-weight: 700;
    letter-spacing: 0.03em;
    width: 38px;
    text-align: center;
    padding: 2px 0;
    border-radius: 4px;
    color: var(--pc);
    background: color-mix(in srgb, var(--pc) 15%, transparent);
    flex-shrink: 0;
  }
  .badge.gw {
    --pc: #64748b;
    width: 26px;
  }
  .fav {
    color: var(--warn);
    display: inline-flex;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--text-faint);
  }
  .dot.connected {
    background: var(--ok);
  }
  .dot.connecting {
    background: var(--warn);
  }
  .dot.error {
    background: var(--danger);
  }
  .empty {
    padding: 24px 16px;
    text-align: center;
    color: var(--text-muted);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
  }
  .empty p {
    margin: 0;
    line-height: 1.5;
  }
  code {
    font-family: var(--mono);
    font-size: 11.5px;
  }
</style>
