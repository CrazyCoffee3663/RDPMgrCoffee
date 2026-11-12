<script lang="ts">
  import { api, buildUri, errText, parseConn, DEFAULT_PORTS, type Field, type Protocol } from "./api";
  import { app, GATEWAY_FOLDER } from "./state.svelte";
  import { createRootFolder, ensureFolder, gwKey as makeKey, itemGwKey } from "./actions";
  import Icon from "./Icon.svelte";

  let { onclose }: { onclose: () => void } = $props();

  type Gw = { host: string; username: string; password: string; domain: string; same: boolean };
  type Conn = {
    path: string[];
    name: string;
    protocol: string;
    host: string;
    port: number | null;
    username: string;
    password: string;
    domain: string;
    notes: string;
    opts: Record<string, string>;
    gateway: Gw | null;
  };
  type Parsed = { conns: Conn[]; folders: string[][]; skipped: string[] };

  let xml = "";
  /** Путь к файлу, если он выбран без диалога (найден в %APPDATA%) */
  let filePath = "";
  let fileName = $state("");
  let found = $state<string | null>(null);
  // Файл mRemoteNG по умолчанию — чтобы не открывать проводник
  api.mremotengDefaultFile().then((p) => (found = p)).catch(() => {});

  async function useFound() {
    if (!found) return;
    filePath = found;
    xml = "";
    fileName = found;
    filePw = "";
    needPw = false;
    await parse();
  }
  let needPw = $state(false);
  let filePw = $state("");
  let parsed = $state<Parsed | null>(null);
  let error = $state("");
  let target = $state("mRemoteNG");
  let keepTree = $state(true);
  let skipDupes = $state(true);
  let busy = $state(false);
  let progress = $state({ done: 0, total: 0 });
  let finished = $state<{ conns: number; gws: number; failed: string[]; skipped: number } | null>(null);

  /** Ключ дубля: протокол + адрес + порт (порт по умолчанию нормализуется) */
  function connKey(protocol: string, host: string, port: number | null): string {
    const h = host.trim().toLowerCase().replace(/^\[|\]$/g, "");
    return `${protocol}|${h}|${port ?? DEFAULT_PORTS[protocol as Protocol] ?? ""}`;
  }
  const existingKeys = $derived(
    new Map(
      app.connections.flatMap((i) => {
        const c = parseConn(i);
        return c ? [[connKey(c.protocol, c.host, c.port), i.name] as [string, string]] : [];
      }),
    ),
  );
  /** Подключения из файла, которые уже есть в хранилище или повторяются в самом файле */
  const dupes = $derived.by(() => {
    if (!parsed) return [] as string[];
    const seen = new Set<string>();
    const out: string[] = [];
    for (const c of parsed.conns) {
      const k = connKey(c.protocol, c.host, c.port);
      const ex = existingKeys.get(k);
      if (ex !== undefined) out.push(`${c.name} — уже есть «${ex}»`);
      else if (seen.has(k)) out.push(`${c.name} — повторяется в файле`);
      seen.add(k);
    }
    return out;
  });
  const toImport = $derived(parsed ? parsed.conns.length - (skipDupes ? dupes.length : 0) : 0);

  const gwCount = $derived(parsed ? new Set(parsed.conns.filter((c) => c.gateway).map((c) => connGwKey(c))).size : 0);

  /** Ключ шлюза подключения. Если учётка шлюза совпадает с учёткой подключения — это «те же данные» */
  function connGwKey(c: Conn): string {
    const g = c.gateway!;
    const same =
      g.same ||
      (g.username.toLowerCase() === c.username.toLowerCase() &&
        g.domain.toLowerCase() === c.domain.toLowerCase() &&
        g.password === c.password);
    return makeKey(g.host, same, g.username, g.domain, g.password);
  }

  async function pick(e: Event) {
    const file = (e.currentTarget as HTMLInputElement).files?.[0];
    if (!file) return;
    fileName = file.name;
    filePath = "";
    xml = await file.text();
    filePw = "";
    needPw = false;
    await parse();
  }

  async function parse() {
    error = "";
    parsed = null;
    try {
      const pw = needPw ? filePw : null;
      parsed = filePath ? await api.importMremotengFile(filePath, pw) : await api.importMremoteng(xml, pw);
      needPw = false;
    } catch (e) {
      const t = errText(e);
      if (t === "MREMOTENG_PASSWORD") needPw = true;
      else error = t;
    }
  }


  async function run() {
    if (!parsed) return;
    busy = true;
    error = "";
    const failed: string[] = [];
    let conns = 0;
    let gws = 0;
    let skipped = 0;
    const seen = new Set<string>(skipDupes ? existingKeys.keys() : []);
    const list = parsed.conns.filter((c) => {
      const k = connKey(c.protocol, c.host, c.port);
      if (skipDupes && seen.has(k)) return (skipped++, false);
      seen.add(k);
      return true;
    });
    progress = { done: 0, total: list.length };
    try {
      if (app.root) await createRootFolder();
      const base = app.fullName(target.trim().replace(/^\/+|\/+$/g, ""));
      const baseId = base ? await ensureFolder(base) : app.defaultFolderId();
      const folderIds = new Map<string, string | null>([["", baseId]]);
      const folderFor = async (path: string[]): Promise<string | null> => {
        const rel = keepTree ? path.join("/") : "";
        if (!folderIds.has(rel)) {
          const full = [base, rel].filter(Boolean).join("/");
          folderIds.set(rel, await ensureFolder(full));
        }
        return folderIds.get(rel)!;
      };
      if (keepTree) for (const p of parsed.folders) {
          const pp = p.join("/");
          if (list.some((c) => (c.path.join("/") + "/").startsWith(pp + "/"))) await folderFor(p);
        }

      // Шлюзы: одинаковые объединяем, существующие в хранилище переиспользуем.
      // Все шлюзы складываем в папку RDG (она скрыта в дереве).
      const gwIds = new Map<string, string>();
      for (const g of app.gateways) {
        const k = await itemGwKey(g);
        if (!gwIds.has(k)) gwIds.set(k, g.id);
      }
      let rdgId: string | null = null;
      for (const c of list) {
        if (!c.gateway) continue;
        const k = connGwKey(c);
        if (gwIds.has(k)) continue;
        const same = k.endsWith("#same");
        const g = c.gateway;
        rdgId ??= await ensureFolder(app.fullName(GATEWAY_FOLDER));
        const fields: Field[] = same
          ? [{ name: "samecreds", value: "true", kind: 0 }]
          : g.domain
            ? [{ name: "domain", value: g.domain, kind: 0 }]
            : [];
        const host = g.host.includes(":") && !g.host.includes("]") && g.host.split(":").length > 2 ? `[${g.host}]` : g.host;
        const saved = await api.save({
          id: null, kind: 1, orgId: null, collectionIds: [], folderId: rdgId,
          name: `Шлюз ${g.host}${same ? "" : g.username ? ` (${g.username})` : ""}`,
          notes: "", username: same ? "" : g.username, password: same || !g.password ? null : g.password,
          uris: [`rdgw://${host}`], fields, favorite: false,
        });
        app.upsert(saved);
        gwIds.set(k, saved.id);
        gws++;
      }

      for (const c of list) {
        try {
          const fields: Field[] = [];
          if (c.domain) fields.push({ name: "domain", value: c.domain, kind: 0 });
          if (c.gateway) fields.push({ name: "gateway", value: gwIds.get(connGwKey(c))!, kind: 0 });
          for (const [k, v] of Object.entries(c.opts)) fields.push({ name: k, value: v, kind: 0 });
          const saved = await api.save({
            id: null, kind: 1, orgId: null, collectionIds: [], folderId: await folderFor(c.path),
            name: c.name, notes: c.notes, username: c.username, password: c.password || null,
            uris: [buildUri(c.protocol as never, c.host, c.port)], fields, favorite: false,
          });
          app.upsert(saved);
          conns++;
        } catch (e) {
          failed.push(`${c.name}: ${errText(e)}`);
        }
        progress.done++;
      }
      finished = { conns, gws, failed, skipped };
      for (const rel of folderIds.keys()) app.toggleExpanded("f:" + app.relName([base, rel].filter(Boolean).join("/")), true);
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="backdrop" role="presentation" onmousedown={(e) => e.target === e.currentTarget && !busy && onclose()}>
  <div class="dialog" role="dialog" aria-label="Импорт из mRemoteNG">
    <header>
      <span>Импорт из mRemoteNG</span>
      <button class="icon-btn" onclick={onclose} disabled={busy}><Icon name="x" /></button>
    </header>
    <div class="body">
      {#if finished}
        <p>
          Импортировано подключений: <b>{finished.conns}</b>{#if finished.gws}, создано RD-шлюзов: <b>{finished.gws}</b>{/if}{#if finished.skipped}, пропущено дублей: <b>{finished.skipped}</b>{/if}.
        </p>
        {#if finished.failed.length}
          <p class="err">Не удалось импортировать:</p>
          <ul class="list selectable">{#each finished.failed as x}<li>{x}</li>{/each}</ul>
        {/if}
        <div class="actions"><button class="btn primary" onclick={onclose}>Готово</button></div>
      {:else}
        <p class="hint">
          Выберите файл <code>confCons.xml</code> — обычно он лежит в
          <code class="selectable">%APPDATA%\mRemoteNG</code>. Пароли расшифровываются локально и сохраняются в Bitwarden.
        </p>
        {#if found && fileName !== found}
          <div class="found">
            <span>Найден файл mRemoteNG:<br /><code class="selectable">{found}</code></span>
            <button class="btn primary" onclick={useFound}>Открыть его</button>
          </div>
        {/if}
        <label class="file btn">
          <Icon name="folder" size={14} />
          {fileName || (found ? "Выбрать другой файл…" : "Выбрать файл…")}
          <input type="file" accept=".xml" onchange={pick} hidden />
        </label>

        {#if needPw}
          <div class="row">
            <span>Файл защищён паролем</span>
            <input type="password" bind:value={filePw} placeholder="Пароль mRemoteNG" onkeydown={(e) => e.key === "Enter" && parse()} />
          </div>
          <div class="actions"><button class="btn primary" onclick={parse} disabled={!filePw}>Открыть</button></div>
        {/if}

        {#if parsed}
          <div class="summary">
            <div><b>{parsed.conns.length}</b> подключений</div>
            <div><b>{parsed.folders.length}</b> папок</div>
            <div><b>{gwCount}</b> RD-шлюзов</div>
          </div>
          {#if parsed.skipped.length}
            <details>
              <summary class="faint">Пропущено {parsed.skipped.length} (протокол не поддерживается)</summary>
              <ul class="list">{#each parsed.skipped as x}<li>{x}</li>{/each}</ul>
            </details>
          {/if}
          <label class="row">
            <span>Папка для импорта <small>{app.root ? `внутри «${app.root}»` : ""}</small></span>
            <input bind:value={target} placeholder="(корень)" />
          </label>
          {#if dupes.length}
            <details>
              <summary>Дубли: {dupes.length} (тот же протокол, адрес и порт)</summary>
              <ul class="list">{#each dupes as x}<li>{x}</li>{/each}</ul>
            </details>
            <label class="chk"><input type="checkbox" bind:checked={skipDupes} /> Не импортировать дубли</label>
          {/if}
          <label class="chk"><input type="checkbox" bind:checked={keepTree} /> Сохранить структуру папок mRemoteNG</label>
          {#if busy}
            <div class="bar"><div style="width: {progress.total ? (progress.done / progress.total) * 100 : 0}%"></div></div>
            <div class="faint">{progress.done} из {progress.total}…</div>
          {/if}
          <div class="actions">
            <button class="btn" onclick={onclose} disabled={busy}>Отмена</button>
            <button class="btn primary" onclick={run} disabled={busy || !toImport}>
              Импортировать {toImport}
            </button>
          </div>
        {/if}
        {#if error}<div class="err selectable">{error}</div>{/if}
      {/if}
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
    width: min(540px, 94vw);
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
    padding: 16px 20px 20px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  p {
    margin: 0;
    line-height: 1.5;
  }
  .hint {
    color: var(--text-muted);
  }
  .found {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 12px;
    border-radius: var(--radius);
    background: var(--accent-soft);
    line-height: 1.5;
  }
  .found span {
    flex: 1;
    min-width: 0;
    word-break: break-all;
  }
  .file {
    align-self: flex-start;
    cursor: pointer;
  }
  .row {
    display: grid;
    grid-template-columns: 1fr 220px;
    gap: 12px;
    align-items: center;
  }
  small {
    color: var(--text-faint);
  }
  .summary {
    display: flex;
    gap: 18px;
    padding: 10px 14px;
    border-radius: var(--radius);
    background: var(--accent-soft);
  }
  .summary b {
    font-size: 16px;
  }
  .list {
    margin: 6px 0 0;
    padding-left: 18px;
    max-height: 140px;
    overflow: auto;
    color: var(--text-muted);
  }
  .chk {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .bar {
    height: 6px;
    border-radius: 3px;
    background: var(--border);
    overflow: hidden;
  }
  .bar div {
    height: 100%;
    background: var(--accent);
    transition: width 0.15s;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .err {
    color: var(--danger, #e5534b);
    white-space: pre-wrap;
  }
</style>
