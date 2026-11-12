<script lang="ts">
  // Панель файлов SFTP для SSH-вкладки: навигация, скачивание в «Загрузки», загрузка, папки, удаление.
  import { api, errText } from "./api";
  import { app } from "./state.svelte";
  import { dialogs } from "./dialogs.svelte";
  import Icon from "./Icon.svelte";

  let { sessionId, onclose }: { sessionId: number; onclose: () => void } = $props();

  type Entry = { name: string; dir: boolean; link: boolean; size: number; mtime: number };
  let path = $state("");
  let entries = $state<Entry[]>([]);
  let loading = $state(false);
  let error = $state("");
  let sel = $state<string | null>(null);
  let busy = $state("");
  let dropOver = $state(false);
  let pathInput = $state("");

  const join = (dir: string, name: string) => (dir.endsWith("/") ? dir : dir + "/") + name;
  const parent = (p: string) => (p === "/" ? "/" : p.replace(/\/[^/]+\/?$/, "") || "/");

  async function load(p: string) {
    loading = true;
    error = "";
    try {
      const r = await api.sftpList(sessionId, p);
      path = r.path;
      pathInput = r.path;
      entries = r.entries;
      sel = null;
    } catch (e) {
      error = errText(e);
    } finally {
      loading = false;
    }
  }
  $effect(() => {
    void sessionId;
    load("");
  });

  async function open(e: Entry) {
    if (e.dir || e.link) {
      // Ссылку пробуем как папку, не вышло — скачиваем
      const before = path;
      await load(join(path, e.name));
      if (!error || !e.link) return;
      await load(before);
    }
    download(e);
  }

  async function download(e: Entry) {
    busy = `Скачивание ${e.name}…`;
    try {
      const local = await api.sftpDownload(sessionId, join(path, e.name));
      app.toast(`Сохранено: ${local}`);
      api.reveal(local).catch(() => {});
    } catch (err) {
      app.toast(errText(err), true);
    } finally {
      busy = "";
    }
  }

  async function upload(files: FileList | File[]) {
    for (const f of Array.from(files)) {
      busy = `Загрузка ${f.name}…`;
      try {
        await api.sftpUpload(sessionId, join(path, f.name), new Uint8Array(await f.arrayBuffer()));
      } catch (err) {
        app.toast(`${f.name}: ${errText(err)}`, true);
      }
    }
    busy = "";
    load(path);
  }

  async function mkdir() {
    const n = await dialogs.input("Новая папка", path, "", "Создать");
    if (!n?.trim()) return;
    try {
      await api.sftpOp(sessionId, "mkdir", join(path, n.trim()));
      load(path);
    } catch (e) {
      app.toast(errText(e), true);
    }
  }

  async function rename(e: Entry) {
    const n = await dialogs.input("Переименовать", e.name, e.name, "Сохранить");
    if (!n?.trim() || n === e.name) return;
    try {
      await api.sftpOp(sessionId, "rename", join(path, e.name), join(path, n.trim()));
      load(path);
    } catch (err) {
      app.toast(errText(err), true);
    }
  }

  async function remove(e: Entry) {
    if (!(await dialogs.confirm(`Удалить «${e.name}»?`, e.dir ? "Папка должна быть пустой." : "Файл будет удалён на сервере.", "Удалить", true)))
      return;
    try {
      await api.sftpOp(sessionId, e.dir ? "rmdir" : "rm", join(path, e.name));
      load(path);
    } catch (err) {
      app.toast(errText(err), true);
    }
  }

  function size(n: number) {
    if (n < 1024) return `${n} Б`;
    const u = ["КБ", "МБ", "ГБ", "ТБ"];
    let i = -1;
    do {
      n /= 1024;
      i++;
    } while (n >= 1024 && i < u.length - 1);
    return `${n.toFixed(n < 10 ? 1 : 0)} ${u[i]}`;
  }
  const date = (t: number) => (t ? new Date(t * 1000).toLocaleString("ru", { dateStyle: "short", timeStyle: "short" }) : "");

  let fileEl = $state<HTMLInputElement>();
</script>

<div
  class="sftp"
  class:over={dropOver}
  role="region"
  aria-label="Файлы SFTP"
  ondragover={(e) => {
    if (e.dataTransfer?.types.includes("Files")) {
      e.preventDefault();
      dropOver = true;
    }
  }}
  ondragleave={() => (dropOver = false)}
  ondrop={(e) => {
    e.preventDefault();
    dropOver = false;
    if (e.dataTransfer?.files.length) upload(e.dataTransfer.files);
  }}
>
  <div class="head">
    <b>Файлы</b>
    <span class="sp"></span>
    <button class="icon-btn" title="Загрузить файлы" onclick={() => fileEl?.click()}><Icon name="download" size={14} /></button>
    <button class="icon-btn" title="Новая папка" onclick={mkdir}><Icon name="folderPlus" size={14} /></button>
    <button class="icon-btn" title="Обновить" onclick={() => load(path)}><Icon name="refresh" size={14} /></button>
    <button class="icon-btn" title="Закрыть" onclick={onclose}><Icon name="x" size={14} /></button>
    <input type="file" multiple hidden bind:this={fileEl} onchange={(e) => e.currentTarget.files && upload(e.currentTarget.files)} />
  </div>
  <form class="path" onsubmit={(e) => (e.preventDefault(), load(pathInput))}>
    <button type="button" class="icon-btn" title="Вверх" onclick={() => load(parent(path))} disabled={path === "/"}>
      <Icon name="chevron" size={13} />
    </button>
    <input class="mono" bind:value={pathInput} />
  </form>
  <div class="list">
    {#if error}
      <div class="msg err selectable">
        {error}
        <div><button class="btn" onclick={() => load(path)}>Повторить</button></div>
      </div>
    {:else if loading && !entries.length}
      <div class="msg faint">Загрузка…</div>
    {/if}
    {#each entries as e (e.name)}
      <div
        class="row"
        class:sel={sel === e.name}
        role="option"
        aria-selected={sel === e.name}
        tabindex="-1"
        onclick={() => (sel = e.name)}
        ondblclick={() => open(e)}
        onkeydown={(k) => k.key === "Enter" && open(e)}
        title={e.dir ? "Двойной клик — открыть" : "Двойной клик — скачать в «Загрузки»"}
      >
        <Icon name={e.dir ? "folder" : e.link ? "copy" : "edit"} size={13} />
        <span class="n">{e.name}</span>
        <span class="s faint">{e.dir ? "" : size(e.size)}</span>
        {#if sel === e.name}
          <span class="acts">
            {#if !e.dir}<button class="icon-btn" title="Скачать" onclick={(ev) => (ev.stopPropagation(), download(e))}><Icon name="download" size={12} /></button>{/if}
            <button class="icon-btn" title="Переименовать" onclick={(ev) => (ev.stopPropagation(), rename(e))}><Icon name="edit" size={12} /></button>
            <button class="icon-btn" title="Удалить" onclick={(ev) => (ev.stopPropagation(), remove(e))}><Icon name="trash" size={12} /></button>
          </span>
        {:else}
          <span class="d faint">{date(e.mtime)}</span>
        {/if}
      </div>
    {/each}
  </div>
  <div class="foot faint">{busy || "Перетащите файлы сюда, чтобы загрузить"}</div>
</div>

<style>
  .sftp {
    position: relative;
    z-index: 20;
    width: 340px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-panel);
    border-left: 1px solid var(--border);
    color: var(--text);
  }
  .sftp.over {
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 6px 6px 6px 10px;
  }
  .sp {
    flex: 1;
  }
  .path {
    display: flex;
    gap: 4px;
    padding: 0 6px 6px;
  }
  .path :global(svg) {
    transform: rotate(-90deg);
  }
  .path input {
    font-size: 12px;
    padding: 4px 7px;
  }
  .list {
    flex: 1;
    overflow: auto;
    padding: 0 4px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 6px;
    border-radius: var(--radius-sm);
    cursor: default;
    color: var(--text-muted);
  }
  .row:hover {
    background: var(--bg-hover);
  }
  .row.sel {
    background: var(--bg-active);
    color: var(--text);
  }
  .n {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }
  .s,
  .d {
    font-size: 11px;
    white-space: nowrap;
  }
  .acts {
    display: flex;
  }
  .msg {
    padding: 12px;
  }
  .err {
    color: var(--danger);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .foot {
    padding: 6px 10px;
    font-size: 11.5px;
    border-top: 1px solid var(--border);
  }
  .mono {
    font-family: var(--mono);
  }
</style>
