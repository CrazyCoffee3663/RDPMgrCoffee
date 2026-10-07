<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";

  type Folder = { id: string; name: string };
  type Conn = {
    id: string;
    name: string;
    folderId: string | null;
    uri: string;
    username: string;
    protocol: string;
  };
  type VaultData = { folders: Folder[]; items: Conn[] };
  type VaultStatus = { status: string; serverUrl: string; email: string };

  const CONN_PROTOCOLS = ["rdp", "ssh", "vnc"];

  let phase: "checking" | "setup" | "locked" | "open" = "checking";
  let busy = false;
  let error = "";
  let info: VaultStatus = { status: "", serverUrl: "", email: "" };

  let server = "";
  let email = "";
  let code = "";
  let password = "";

  let data: VaultData = { folders: [], items: [] };
  let query = "";
  let notice = "";
  let noticeIsError = false;

  onMount(refreshStatus);

  async function refreshStatus() {
    error = "";
    try {
      info = await invoke<VaultStatus>("vault_status");
      if (info.status === "unauthenticated") {
        server = info.serverUrl || server;
        phase = "setup";
      } else {
        phase = "locked";
      }
    } catch (e) {
      error = String(e);
      phase = "setup";
    }
  }

  async function setup() {
    error = "";
    busy = true;
    try {
      data = await invoke<VaultData>("vault_setup", { server, email, password, code });
      password = "";
      code = "";
      phase = "open";
    } catch (e) {
      error = String(e);
    }
    busy = false;
  }

  async function unlock() {
    error = "";
    busy = true;
    try {
      data = await invoke<VaultData>("vault_unlock", { password });
      password = "";
      phase = "open";
    } catch (e) {
      error = String(e);
    }
    busy = false;
  }

  async function lock() {
    await invoke("vault_lock");
    data = { folders: [], items: [] };
    query = "";
    notice = "";
    phase = "locked";
  }

  async function reset() {
    error = "";
    password = "";
    await invoke("vault_reset");
    await refreshStatus();
  }

  async function connect(item: Conn) {
    notice = "";
    try {
      await invoke("connect", { id: item.id });
      notice = `Запускаю подключение: ${item.name}`;
      noticeIsError = false;
    } catch (e) {
      notice = String(e);
      noticeIsError = true;
    }
  }

  $: folderName = new Map<string, string>(
    data.folders.map((f): [string, string] => [f.id, f.name])
  );
  $: q = query.trim().toLowerCase();
  $: filtered = data.items.filter(
    (i) =>
      CONN_PROTOCOLS.includes(i.protocol) &&
      (!q ||
        i.name.toLowerCase().includes(q) ||
        i.uri.toLowerCase().includes(q) ||
        i.username.toLowerCase().includes(q))
  );
  $: groups = (() => {
    const m = new Map<string, Conn[]>();
    for (const i of filtered) {
      const g = (i.folderId && folderName.get(i.folderId)) || "Без папки";
      if (!m.has(g)) m.set(g, []);
      m.get(g)!.push(i);
    }
    return [...m.entries()].sort((a, b) => a[0].localeCompare(b[0], "ru"));
  })();
</script>

<main>
  {#if phase === "checking"}
    <p class="hint center">Запуск…</p>
  {:else if phase === "setup"}
    <form class="unlock" on:submit|preventDefault={setup}>
      <h1>Первый запуск</h1>
      <p class="hint">Подключение к хранилищу паролей Bitwarden / Vaultwarden</p>
      <input type="text" placeholder="Адрес сервера, https://…" bind:value={server} disabled={busy} />
      <input type="email" placeholder="Почта" bind:value={email} disabled={busy} />
      <input type="password" placeholder="Мастер-пароль" bind:value={password} disabled={busy} />
      <input type="text" placeholder="Код 2FA (если включена)" bind:value={code} disabled={busy} />
      <button type="submit" disabled={busy || !server || !email || !password}>
        {busy ? "Вход и синхронизация…" : "Войти"}
      </button>
      <p class="hint small">Если сервер находится в сети Tailscale, сначала включите Tailscale.</p>
      {#if error}<p class="error">{error}</p>{/if}
    </form>
  {:else if phase === "locked"}
    <form class="unlock" on:submit|preventDefault={unlock}>
      <h1>Менеджер подключений</h1>
      <p class="hint">{info.email}{info.serverUrl ? " · " + info.serverUrl : ""}</p>
      <input
        type="password"
        placeholder="Мастер-пароль"
        bind:value={password}
        disabled={busy}
        autofocus
      />
      <button type="submit" disabled={busy || !password}>
        {busy ? "Синхронизация…" : "Разблокировать"}
      </button>
      <button type="button" class="link" on:click={reset} disabled={busy}>
        Сменить сервер или аккаунт
      </button>
      {#if error}<p class="error">{error}</p>{/if}
    </form>
  {:else}
    <header>
      <input class="search" placeholder="Поиск по имени, адресу, логину…" bind:value={query} />
      <button on:click={lock}>Заблокировать</button>
    </header>

    {#if notice}<p class={noticeIsError ? "error" : "hint"}>{notice}</p>{/if}

    {#if groups.length === 0}
      <p class="hint empty">
        Ничего не найдено. Подключениями считаются записи Bitwarden с адресом вида
        rdp://хост, ssh://хост или vnc://хост.
      </p>
    {/if}

    {#each groups as [group, items]}
      <section>
        <h2>{group} <span>{items.length}</span></h2>
        {#each items as item}
          <div class="row" title="Двойной клик — подключиться" on:dblclick={() => connect(item)}>
            <span class="badge">{item.protocol || "—"}</span>
            <span class="name">{item.name}</span>
            <span class="uri">{item.uri}</span>
            <span class="user">{item.username}</span>
          </div>
        {/each}
      </section>
    {/each}
  {/if}
</main>

<style>
  :global(body) {
    margin: 0;
    font-family: "Segoe UI", system-ui, sans-serif;
    background: #f4f5f7;
    color: #1d2330;
  }
  main { padding: 20px; max-width: 1000px; margin: 0 auto; }
  .unlock { max-width: 340px; margin: 10vh auto 0; display: flex; flex-direction: column; gap: 12px; }
  h1 { font-size: 22px; margin: 0; }
  .hint { color: #6b7385; margin: 0; }
  .small { font-size: 12px; }
  .center { text-align: center; margin-top: 20vh; }
  .empty { margin-top: 24px; }
  .error { color: #c0392b; white-space: pre-wrap; }
  input[type="password"], input[type="text"], input[type="email"], .search {
    padding: 10px 12px; border: 1px solid #c9cedb; border-radius: 8px; font-size: 14px; background: #fff; color: inherit;
  }
  button {
    padding: 10px 16px; border: 0; border-radius: 8px; background: #2f6feb; color: #fff; font-size: 14px; cursor: pointer;
  }
  button.link { background: none; color: #6b7385; padding: 4px; text-decoration: underline; }
  button:disabled { opacity: 0.5; cursor: default; }
  header { display: flex; gap: 12px; align-items: center; margin-bottom: 16px; }
  .search { flex: 1; }
  section { margin-bottom: 20px; }
  h2 { font-size: 13px; text-transform: uppercase; letter-spacing: 0.04em; color: #6b7385; margin: 0 0 6px; }
  h2 span { font-weight: normal; opacity: 0.7; }
  .row {
    display: grid; grid-template-columns: 56px 1.2fr 1.4fr 1fr; gap: 12px; align-items: center;
    padding: 10px 12px; background: #fff; border-radius: 8px; margin-bottom: 4px; font-size: 14px;
    cursor: pointer; user-select: none;
  }
  .row:hover { background: #eaf1ff; }
  .badge { font-size: 11px; text-transform: uppercase; background: #e3e7f0; border-radius: 4px; padding: 2px 6px; text-align: center; }
  .name { font-weight: 600; }
  .uri, .user { color: #6b7385; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  @media (prefers-color-scheme: dark) {
    :global(body) { background: #14171f; color: #e6e9f0; }
    input[type="password"], input[type="text"], input[type="email"], .search, .row { background: #1d212c; border-color: #2c3242; color: #e6e9f0; }
    .row:hover { background: #26304a; }
    .badge { background: #2c3242; }
    button.link { color: #9aa3b8; }
  }
</style>