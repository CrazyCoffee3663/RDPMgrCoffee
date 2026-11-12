<script lang="ts">
  import {
    api, errText, parseConn, buildUri, KNOWN_FIELDS, PROTOCOLS, PROTOCOL_LABEL, DEFAULT_PORTS,
    type Field, type Item, type Protocol,
  } from "./api";
  import { app } from "./state.svelte";
  import { dialogs } from "./dialogs.svelte";
  import Icon from "./Icon.svelte";

  type Form = {
    id: string | null;
    kind: number;
    isConn: boolean;
    /** Запись RD-шлюза (адрес rdgw://) */
    isGateway: boolean;
    name: string;
    folderId: string | null;
    orgId: string | null;
    collectionIds: string[];
    protocol: Protocol;
    host: string;
    port: string;
    credMode: "own" | "ref";
    username: string;
    password: string | null;
    notes: string;
    favorite: boolean;
    opts: Record<string, string>;
    extra: Field[];
    uris: string[];
    otherUris: string[];
  };

  let f = $state<Form | null>(null);
  let original = $state<Item | undefined>();
  let showPw = $state(false);
  let busy = $state(false);
  let error = $state("");
  let ports = $state<string[]>([]);
  let loadedFor = "";
  let dirty = $state(false);

  // Списки для выбора: только записи рабочей папки, сгруппированные по папкам
  type Group = [string, Item[]];
  function groups(list: Item[]): Group[] {
    const m = new Map<string, Item[]>();
    for (const i of list) {
      if (i.id === f?.id) continue;
      const g = app.itemFolderRel(i);
      m.set(g, [...(m.get(g) ?? []), i]);
    }
    return [...m.entries()]
      .map(([g, l]): Group => [g, l.sort((a, b) => a.name.localeCompare(b.name, "ru"))])
      .sort((a, b) => (a[0] === "" ? -1 : b[0] === "" ? 1 : a[0].localeCompare(b[0], "ru")));
  }
  const credGroups = $derived(groups(app.credentials));
  const gwGroups = $derived(groups(app.gateways));
  const keyGroups = $derived(groups(app.sshKeys));
  const jumpGroups = $derived(groups(app.connections.filter((i) => parseConn(i)?.protocol === "ssh")));

  // Значение ссылки (id или имя записи) → id
  function refId(v: string): string {
    if (!v) return "";
    if (app.item(v)) return v;
    return app.view.items.find((i) => i.name === v)?.id ?? v;
  }

  function load(key: string) {
    error = "";
    showPw = false;
    dirty = false;
    if (key.startsWith("new:")) {
      const conn = key === "new:conn";
      const gw = key === "new:gateway";
      original = undefined;
      f = {
        id: null, kind: 1, isConn: conn, isGateway: gw, name: "", folderId: app.editingFolder, orgId: null, collectionIds: [],
        protocol: "rdp", host: "", port: "", credMode: "own", username: "", password: "", notes: "",
        favorite: false, opts: gw ? { samecreds: "1" } : {}, extra: [], uris: conn || gw ? [] : [""], otherUris: [],
      };
      return;
    }
    const it = app.item(key);
    if (!it) {
      f = null;
      return;
    }
    original = it;
    const c = parseConn(it);
    const gw = !c && app.gatewayIds.has(it.id);
    // Адрес шлюза: rdgw://host:port, https://host или просто host
    const gm = gw ? /^(?:[a-z]+:\/\/)?(\[[^\]]+\]|[^:/?#]+)(?::(\d+))?/i.exec((it.uris[0] ?? "").trim()) : null;
    const opts: Record<string, string> = {};
    const extra: Field[] = [];
    for (const fl of it.fields) {
      const k = fl.name.trim().toLowerCase();
      if (((c && KNOWN_FIELDS.includes(k)) || (gw && (k === "domain" || k === "samecreds"))) && fl.kind !== 1) opts[k] = fl.value ?? "";
      else extra.push({ ...fl });
    }
    for (const k of ["credential", "gateway", "sshkey", "jump"]) if (opts[k]) opts[k] = refId(opts[k]);
    f = {
      id: it.id,
      kind: it.kind,
      isConn: !!c,
      isGateway: gw,
      name: it.name,
      folderId: it.folderId,
      orgId: it.orgId,
      collectionIds: it.collectionIds,
      protocol: c?.protocol ?? "rdp",
      host: c?.host ?? gm?.[1]?.replace(/^\[|\]$/g, "") ?? "",
      port: c?.port ? String(c.port) : gm?.[2] ?? "",
      credMode: opts.credential ? "ref" : "own",
      username: it.username,
      password: null,
      notes: it.notes,
      favorite: it.favorite,
      opts,
      extra,
      uris: c || gw ? [] : it.uris.length ? [...it.uris] : [""],
      otherUris: c ? it.uris.filter((u) => u !== c.uri) : gw ? it.uris.slice(1) : [],
    };
  }

  $effect(() => {
    const key = app.editing;
    if (key && key !== loadedFor) {
      loadedFor = key;
      load(key);
    }
    if (!key) {
      loadedFor = "";
      f = null;
    }
  });

  $effect(() => {
    if (f?.isConn && f.protocol === "serial") api.serialPorts().then((p) => (ports = p)).catch(() => {});
  });

  async function revealPassword() {
    if (!f) return;
    if (f.password === null && f.id) {
      try {
        f.password = (await api.secret(f.id)).password;
      } catch (e) {
        error = errText(e);
        return;
      }
    }
    showPw = !showPw;
  }

  async function generate() {
    if (!f) return;
    f.password = await api.generate(20);
    showPw = true;
    dirty = true;
  }

  async function copyPw() {
    if (!f) return;
    const sec = app.settings.clipboardClearSeconds || null;
    if (f.password !== null) await api.clipboardSet(f.password, sec, true);
    else if (f.id) await api.copySecret(f.id, "password", sec);
    app.toast("Пароль скопирован");
  }

  async function close() {
    if (dirty && !(await dialogs.confirm("Закрыть без сохранения?", "Изменения будут потеряны.", "Закрыть", true))) return;
    app.editing = null;
  }

  async function save(connectAfter = false) {
    if (!f) return;
    error = "";
    if (!f.name.trim()) {
      if (f.isConn && f.host.trim()) f.name = f.host.trim();
      else return (error = "Укажите имя");
    }
    if ((f.isConn || f.isGateway) && !f.host.trim()) return (error = "Укажите адрес");
    const port = f.port.trim() ? Number(f.port) : null;
    if (port !== null && (!Number.isInteger(port) || port < 1 || port > 65535)) return (error = "Неверный порт");

    const fields: Field[] = [];
    if (f.isConn) {
      if (f.credMode === "ref") f.opts.credential = f.opts.credential || "";
      else delete f.opts.credential;
      for (const k of KNOWN_FIELDS) {
        const v = (f.opts[k] ?? "").trim();
        if (v && v !== "0") fields.push({ name: k, value: v, kind: 0 });
      }
      if (f.credMode === "ref" && !f.opts.credential) return (error = "Выберите запись с учётными данными");
    }
    const gwSame = f.isGateway && flag("samecreds");
    if (gwSame) fields.push({ name: "samecreds", value: "true", kind: 0 });
    else if (f.isGateway && (f.opts.domain ?? "").trim()) fields.push({ name: "domain", value: f.opts.domain.trim(), kind: 0 });
    for (const x of f.extra) if (x.name.trim()) fields.push(x);

    const gwHost = f.host.trim().includes(":") ? `[${f.host.trim()}]` : f.host.trim();
    const uris = f.isConn
      ? [buildUri(f.protocol, f.host, port), ...f.otherUris]
      : f.isGateway
        ? [`rdgw://${gwHost}${port ? ":" + port : ""}`, ...f.otherUris]
        : f.uris.map((u) => u.trim()).filter(Boolean);
    const own = (!f.isConn || f.credMode === "own") && !gwSame;

    busy = true;
    try {
      const saved = await api.save({
        id: f.id,
        kind: f.kind,
        orgId: f.orgId,
        collectionIds: f.collectionIds,
        folderId: f.folderId,
        name: f.name.trim(),
        notes: f.notes,
        username: own ? f.username : "",
        password: own ? f.password : original?.hasPassword ? "" : null,
        uris,
        fields,
        favorite: f.favorite,
      });
      app.upsert(saved);
      app.selected = saved.id;
      dirty = false;
      loadedFor = saved.id;
      app.editing = saved.id;
      load(saved.id);
      app.toast("Сохранено");
      if (connectAfter && parseConn(saved)) app.connect(saved);
    } catch (e) {
      error = errText(e);
    }
    busy = false;
  }

  function onKey(e: KeyboardEvent) {
    if (!f) return;
    if (e.ctrlKey && e.code === "KeyS") {
      e.preventDefault();
      save();
    }
  }

  const opt = (k: string) => f?.opts[k] ?? "";
  function setOpt(k: string, v: string | boolean) {
    if (!f) return;
    f.opts[k] = typeof v === "boolean" ? (v ? "1" : "") : v;
    dirty = true;
  }
  const flag = (k: string) => ["1", "true", "yes", "да", "on"].includes(opt(k).toLowerCase());
</script>

<svelte:window onkeydown={onKey} />

{#snippet refOptions(list: Group[], current: string)}
  {#each list as [g, items]}
    {#if g}
      <optgroup label={g}>
        {#each items as l}<option value={l.id}>{l.name}{l.username ? ` — ${l.username}` : ""}</option>{/each}
      </optgroup>
    {:else}
      {#each items as l}<option value={l.id}>{l.name}{l.username ? ` — ${l.username}` : ""}</option>{/each}
    {/if}
  {/each}
  {#if current && !list.some(([, items]) => items.some((i) => i.id === current))}
    <option value={current}>{app.item(current)?.name ?? current} (вне рабочей папки)</option>
  {/if}
{/snippet}

{#if f}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <section class="editor" oninput={() => (dirty = true)}>
    <header>
      <div class="ht">
        {#if f.isConn}<span class="badge proto-{f.protocol}">{PROTOCOL_LABEL[f.protocol]}</span>{/if}
        <span>
          {f.id
            ? f.isConn ? "Подключение" : f.isGateway ? "RD-шлюз" : "Запись"
            : f.isConn ? "Новое подключение" : f.isGateway ? "Новый RD-шлюз" : "Новый логин"}
        </span>
      </div>
      <button class="icon-btn" title="Закрыть" onclick={close}><Icon name="x" /></button>
    </header>

    <div class="body">
      <div class="grp">
        <label class="full">
          <span>Имя</span>
          <input bind:value={f.name} placeholder={f.isConn ? "Например, Сервер 1С" : ""} />
        </label>
        <label class="full">
          <span>Папка</span>
          <select bind:value={f.folderId}>
            <option value={app.defaultFolderId()}>{app.root ? `${app.root} (корень)` : "— Без папки —"}</option>
            {#each app.scopedFolders as { folder, rel }}<option value={folder.id}>{rel}</option>{/each}
            {#if f.folderId && f.folderId !== app.defaultFolderId() && !app.scopedFolders.some((x) => x.folder.id === f?.folderId)}
              <option value={f.folderId}>{app.folderName.get(f.folderId) ?? "?"} (вне рабочей папки)</option>
            {/if}
          </select>
        </label>
        {#if !f.id && app.view.orgs.length}
          <label>
            <span>Владелец</span>
            <select bind:value={f.orgId} onchange={() => f && (f.collectionIds = [])}>
              <option value={null}>Личное хранилище</option>
              {#each app.view.orgs as o}<option value={o.id}>{o.name}</option>{/each}
            </select>
          </label>
          {#if f.orgId}
            <label>
              <span>Коллекция</span>
              <select
                value={f.collectionIds[0] ?? ""}
                onchange={(e) => f && (f.collectionIds = [(e.target as HTMLSelectElement).value])}
              >
                <option value="" disabled>Выберите…</option>
                {#each app.view.collections.filter((c) => c.orgId === f?.orgId && !c.readOnly) as c}
                  <option value={c.id}>{c.name}</option>
                {/each}
              </select>
            </label>
          {/if}
        {/if}
      </div>

      {#if f.isConn}
        <div class="sect">Подключение</div>
        <div class="grp">
          <div class="protos full">
            {#each PROTOCOLS as p}
              <button
                class="proto proto-{p}"
                class:on={f.protocol === p}
                onclick={() => f && ((f.protocol = p), (dirty = true))}>{PROTOCOL_LABEL[p]}</button
              >
            {/each}
          </div>
          {#if f.protocol === "serial"}
            <label>
              <span>COM-порт</span>
              <input bind:value={f.host} list="serial-ports" placeholder="COM3" />
              <datalist id="serial-ports">{#each ports as p}<option value={p}></option>{/each}</datalist>
            </label>
            <label>
              <span>Скорость</span>
              <select value={opt("baud") || "9600"} onchange={(e) => setOpt("baud", (e.target as HTMLSelectElement).value)}>
                {#each ["1200", "2400", "4800", "9600", "19200", "38400", "57600", "115200", "230400"] as b}
                  <option value={b}>{b}</option>
                {/each}
              </select>
            </label>
            <label class="full">
              <span>Параметры порта <span class="faint">(скорость,биты,чётность,стоп,поток)</span></span>
              <input value={opt("sercfg")} oninput={(e) => setOpt("sercfg", e.currentTarget.value)} placeholder="9600,8,n,1,N" />
            </label>
          {:else}
            <label class="host">
              <span>Адрес</span>
              <input bind:value={f.host} placeholder="host.example.com или 10.0.0.5" />
            </label>
            <label class="port">
              <span>Порт</span>
              <input bind:value={f.port} placeholder={String(DEFAULT_PORTS[f.protocol] ?? "")} inputmode="numeric" />
            </label>
          {/if}
        </div>

        {#if f.protocol === "winbox"}
          <p class="faint wbhint">
            Роутер MikroTik: двойной клик открывает WinBox (скачивается с сайта MikroTik при первом запуске). Адрес — IP или DNS-имя
            роутера; порт по умолчанию 8291. WinBox получает адрес и логин, пароль копируется в буфер обмена (вставьте Ctrl+V) и стирается через 45 с.
          </p>
        {/if}
        {#if f.protocol !== "serial"}
          <div class="sect">Учётные данные</div>
          <div class="grp">
            <div class="seg full">
              <button class:on={f.credMode === "own"} onclick={() => f && ((f.credMode = "own"), (dirty = true))}>Указать здесь</button>
              <button class:on={f.credMode === "ref"} onclick={() => f && ((f.credMode = "ref"), (dirty = true))}>Из другой записи</button>
            </div>
            {#if f.credMode === "ref"}
              <label class="full">
                <span>Запись с логином и паролем</span>
                <select value={opt("credential")} onchange={(e) => setOpt("credential", (e.target as HTMLSelectElement).value)}>
                  <option value="">Выберите…</option>
                  {@render refOptions(credGroups, opt("credential"))}
                </select>
                <small class="faint">Одна учётка на много серверов: смените пароль в одном месте.</small>
              </label>
            {:else}
              <label class="full">
                <span>Логин</span>
                <input bind:value={f.username} placeholder={f.protocol === "rdp" ? "user или ДОМЕН\\user" : "root"} autocomplete="off" />
              </label>
              <label class="full">
                <span>Пароль</span>
                <div class="pw">
                  <input
                    type={showPw ? "text" : "password"}
                    value={f.password ?? ""}
                    oninput={(e) => f && (f.password = e.currentTarget.value)}
                    placeholder={f.password === null && original?.hasPassword ? "•••••••• (не изменён)" : ""}
                    autocomplete="new-password"
                    class="mono"
                  />
                  <button class="icon-btn" title={showPw ? "Скрыть" : "Показать"} onclick={revealPassword}>
                    <Icon name={showPw ? "eyeOff" : "eye"} size={15} />
                  </button>
                  <button class="icon-btn" title="Сгенерировать" onclick={generate}><Icon name="dice" size={15} /></button>
                  <button class="icon-btn" title="Копировать" onclick={copyPw}><Icon name="copy" size={15} /></button>
                </div>
              </label>
            {/if}
            {#if f.protocol === "rdp"}
              <label class="full">
                <span>Домен</span>
                <input value={opt("domain")} oninput={(e) => setOpt("domain", e.currentTarget.value)} placeholder="CORP" />
              </label>
            {/if}
            {#if f.protocol === "ssh"}
              <label class="full">
                <span>SSH-ключ</span>
                <select value={opt("sshkey")} onchange={(e) => setOpt("sshkey", (e.target as HTMLSelectElement).value)}>
                  <option value="">Не использовать</option>
                  {@render refOptions(keyGroups, opt("sshkey"))}
                </select>
              </label>
            {/if}
          </div>
        {/if}

        {#if f.protocol === "rdp"}
          <div class="sect">RDP</div>
          <div class="grp">
            <label class="full">
              <span>RD-шлюз</span>
              <select value={opt("gateway")} onchange={(e) => setOpt("gateway", (e.target as HTMLSelectElement).value)}>
                <option value="">Без шлюза</option>
                {@render refOptions(gwGroups, opt("gateway"))}
              </select>
              {#if !app.gateways.length}
                <small class="faint">Шлюзов пока нет — создайте во вкладке «Логины» (правый клик → «Новый RD-шлюз»).</small>
              {/if}
            </label>
            <label>
              <span>Разрешение</span>
              <select value={opt("resolution") || "auto"} onchange={(e) => setOpt("resolution", (e.target as HTMLSelectElement).value)}>
                <option value="auto">По размеру вкладки</option>
                {#each ["1280x720", "1366x768", "1600x900", "1920x1080", "2560x1440"] as r}<option value={r}>{r}</option>{/each}
              </select>
            </label>
            <label>
              <span>Сертификат</span>
              <select value={opt("cert") || "tofu"} onchange={(e) => setOpt("cert", (e.target as HTMLSelectElement).value === "tofu" ? "" : (e.target as HTMLSelectElement).value)}>
                <option value="tofu">Проверять отпечаток (спросить при первом входе)</option>
                <option value="ignore">Не проверять — небезопасно</option>
              </select>
            </label>
            <label>
              <span>Проверка подлинности</span>
              <select value={opt("auth") === "ntlm" ? "ntlm" : "kerberos"} onchange={(e) => setOpt("auth", (e.target as HTMLSelectElement).value === "ntlm" ? "ntlm" : "")}>
                <option value="kerberos">Kerberos → NTLM (как mstsc, надёжнее)</option>
                <option value="ntlm">Только NTLM — быстрее вне домена, слабее</option>
              </select>
            </label>
            <div class="checks full">
              <label class="chk"><input type="checkbox" checked={flag("admin")} onchange={(e) => setOpt("admin", e.currentTarget.checked)} /> Консоль (/admin)</label>
              <label class="chk"><input type="checkbox" checked={flag("sound")} onchange={(e) => setOpt("sound", e.currentTarget.checked)} /> Звук</label>
              <label class="chk"><input type="checkbox" checked={flag("microphone")} onchange={(e) => setOpt("microphone", e.currentTarget.checked)} /> Микрофон</label>
              <label class="chk"><input type="checkbox" checked={flag("printers")} onchange={(e) => setOpt("printers", e.currentTarget.checked)} /> Принтеры</label>
              <label class="chk"><input type="checkbox" checked={opt("clipboard") !== "off"} onchange={(e) => setOpt("clipboard", e.currentTarget.checked ? "" : "off")} /> Буфер обмена</label>
              <label class="chk"><input type="checkbox" checked={flag("drives")} onchange={(e) => setOpt("drives", e.currentTarget.checked)} /> Все диски</label>
              <label class="chk"><input type="checkbox" checked={flag("smartcard")} onchange={(e) => setOpt("smartcard", e.currentTarget.checked)} /> Смарт-карты</label>
            </div>
            <label class="full">
              <span>Доп. параметры FreeRDP</span>
              <input class="mono" value={opt("freerdp")} oninput={(e) => setOpt("freerdp", e.currentTarget.value)} placeholder="/network:lan +fonts" />
            </label>
          </div>
        {/if}

        {#if f.protocol === "ssh"}
          <div class="sect">SSH</div>
          <div class="grp">
            <label>
              <span>Keepalive, сек</span>
              <input value={opt("keepalive")} oninput={(e) => setOpt("keepalive", e.currentTarget.value)} placeholder="30" inputmode="numeric" />
            </label>
            <label>
              <span>Промежуточный хост (jump)</span>
              <select value={opt("jump")} onchange={(e) => setOpt("jump", (e.target as HTMLSelectElement).value)}>
                <option value="">Напрямую</option>
                {@render refOptions(jumpGroups, opt("jump"))}
              </select>
            </label>
            <label class="full">
              <span>Туннели <small class="faint">— по одному на строку: <code>L 8080 10.0.0.5:80</code> (локальный порт → хост за сервером), <code>D 1080</code> (SOCKS5-прокси)</small></span>
              <textarea class="mono" rows="3" value={opt("tunnels")} oninput={(e) => setOpt("tunnels", e.currentTarget.value)} placeholder="L 13389 10.0.0.5:3389"></textarea>
            </label>
            <label class="chk full">
              <input type="checkbox" checked={flag("mikrotik")} onchange={(e) => setOpt("mikrotik", e.currentTarget.checked)} />
              Это MikroTik — в меню появится WinBox (подключается к тому же адресу, порт 8291)
            </label>
          </div>
        {/if}
      {:else if f.isGateway}
        <div class="sect">RD-шлюз</div>
        <div class="grp">
          <label class="host">
            <span>Адрес шлюза</span>
            <input bind:value={f.host} placeholder="gw.example.com" />
          </label>
          <label class="port">
            <span>Порт</span>
            <input bind:value={f.port} placeholder="443" inputmode="numeric" />
          </label>
        </div>
        <div class="sect">Учётные данные шлюза</div>
        <div class="grp">
          <label class="chk full">
            <input type="checkbox" checked={flag("samecreds")} onchange={(e) => setOpt("samecreds", e.currentTarget.checked)} />
            Использовать те же данные, что и для подключения
          </label>
          {#if flag("samecreds")}
            <small class="faint full">Логин, пароль и домен шлюза берутся из учётных данных RDP-подключения, которое через него идёт.</small>
          {:else}
          <label class="full"><span>Логин</span><input bind:value={f.username} placeholder="user или ДОМЕН\user" autocomplete="off" /></label>
          <label class="full">
            <span>Пароль</span>
            <div class="pw">
              <input
                type={showPw ? "text" : "password"}
                value={f.password ?? ""}
                oninput={(e) => f && (f.password = e.currentTarget.value)}
                placeholder={f.password === null && original?.hasPassword ? "•••••••• (не изменён)" : ""}
                autocomplete="new-password"
                class="mono"
              />
              <button class="icon-btn" title={showPw ? "Скрыть" : "Показать"} onclick={revealPassword}><Icon name={showPw ? "eyeOff" : "eye"} size={15} /></button>
              <button class="icon-btn" title="Сгенерировать" onclick={generate}><Icon name="dice" size={15} /></button>
              <button class="icon-btn" title="Копировать" onclick={copyPw}><Icon name="copy" size={15} /></button>
            </div>
          </label>
          <label class="full">
            <span>Домен</span>
            <input value={opt("domain")} oninput={(e) => setOpt("domain", e.currentTarget.value)} placeholder="CORP" />
          </label>
          {/if}
        </div>
      {:else}
        <div class="sect">Учётные данные</div>
        <div class="grp">
          <label class="full"><span>Логин</span><input bind:value={f.username} autocomplete="off" /></label>
          {#if f.kind === 1}
            <label class="full">
              <span>Пароль</span>
              <div class="pw">
                <input
                  type={showPw ? "text" : "password"}
                  value={f.password ?? ""}
                  oninput={(e) => f && (f.password = e.currentTarget.value)}
                  placeholder={f.password === null && original?.hasPassword ? "•••••••• (не изменён)" : ""}
                  autocomplete="new-password"
                  class="mono"
                />
                <button class="icon-btn" title={showPw ? "Скрыть" : "Показать"} onclick={revealPassword}><Icon name={showPw ? "eyeOff" : "eye"} size={15} /></button>
                <button class="icon-btn" title="Сгенерировать" onclick={generate}><Icon name="dice" size={15} /></button>
                <button class="icon-btn" title="Копировать" onclick={copyPw}><Icon name="copy" size={15} /></button>
              </div>
            </label>
            <div class="full uris">
              <span class="lbl">Адреса (URI)</span>
              {#each f.uris as _, i}
                <div class="pw">
                  <input bind:value={f.uris[i]} placeholder="https://… или host" />
                  <button class="icon-btn" title="Убрать" onclick={() => f && (f.uris.splice(i, 1), (dirty = true))}><Icon name="x" size={14} /></button>
                </div>
              {/each}
              <button class="link" onclick={() => f && f.uris.push("")}>+ адрес</button>
            </div>
          {/if}
        </div>
      {/if}

      <div class="sect">Дополнительно</div>
      <div class="grp">
        {#if f.extra.length}
          <div class="full extra">
            {#each f.extra as x, i}
              <div class="xrow">
                <input bind:value={x.name} placeholder="Имя поля" />
                {#if x.value === null}
                  <input disabled placeholder="•••• скрытое поле" />
                {:else}
                  <input bind:value={x.value} placeholder="Значение" type={x.kind === 1 ? "password" : "text"} />
                {/if}
                <button class="icon-btn" title="Удалить поле" onclick={() => f && (f.extra.splice(i, 1), (dirty = true))}><Icon name="x" size={14} /></button>
              </div>
            {/each}
          </div>
        {/if}
        <button class="link full" onclick={() => f && f.extra.push({ name: "", value: "", kind: 0 })}>+ поле</button>
        <label class="full"><span>Заметки</span><textarea bind:value={f.notes} rows="3"></textarea></label>
        <label class="chk full"><input type="checkbox" bind:checked={f.favorite} /> Избранное</label>
      </div>

      {#if error}<div class="error-text err">{error}</div>{/if}
    </div>

    <footer>
      {#if f.isConn}
        <button class="btn" onclick={() => save(true)} disabled={busy} title="Сохранить и подключиться">
          <Icon name="play" size={13} /> Подключиться
        </button>
      {/if}
      <span class="sp"></span>
      <button class="btn" onclick={close} disabled={busy}>Отмена</button>
      <button class="btn primary" onclick={() => save()} disabled={busy}>{busy ? "Сохранение…" : "Сохранить"}</button>
    </footer>
  </section>
{/if}

<style>
  .editor {
    width: 360px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-panel);
    border-left: 1px solid var(--border);
    min-height: 0;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 8px 8px 14px;
    border-bottom: 1px solid var(--border);
  }
  .ht {
    display: flex;
    align-items: center;
    gap: 8px;
    font-weight: 600;
    font-size: 14px;
  }
  .badge {
    font-size: 10px;
    font-weight: 700;
    padding: 2px 6px;
    border-radius: 4px;
    color: var(--pc);
    background: color-mix(in srgb, var(--pc) 15%, transparent);
  }
  .body {
    flex: 1;
    overflow: auto;
    padding: 4px 14px 16px;
  }
  .sect {
    margin: 16px 0 8px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-faint);
  }
  .wbhint {
    margin: 2px 0 10px;
    font-size: 12px;
    line-height: 1.45;
  }
  .grp {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
    padding-top: 10px;
  }
  .grp:first-child {
    padding-top: 12px;
  }
  .full {
    grid-column: 1 / -1;
  }
  .host {
    grid-column: 1 / 2;
  }
  .grp:has(.host) {
    grid-template-columns: 1fr 90px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  label > span,
  .lbl {
    font-size: 12px;
    color: var(--text-muted);
  }
  small {
    font-size: 11px;
    line-height: 1.4;
  }
  .mono {
    font-family: var(--mono);
  }
  .pw {
    display: flex;
    gap: 2px;
    align-items: center;
  }
  .protos,
  .seg {
    display: flex;
    gap: 4px;
    background: var(--bg-sunken);
    padding: 3px;
    border-radius: var(--radius);
  }
  .proto,
  .seg button {
    flex: 1;
    padding: 5px 0;
    border-radius: var(--radius-sm);
    font-weight: 600;
    color: var(--text-muted);
  }
  .proto:hover,
  .seg button:hover {
    color: var(--text);
  }
  .proto.on {
    background: var(--bg-panel);
    color: var(--pc);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.15);
  }
  .seg button {
    font-weight: 500;
  }
  .seg button.on {
    background: var(--bg-panel);
    color: var(--text);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.15);
  }
  .checks {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 16px;
  }
  .chk {
    flex-direction: row;
    align-items: center;
    gap: 7px;
    cursor: pointer;
  }
  .extra {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .xrow {
    display: grid;
    grid-template-columns: 1fr 1.4fr 28px;
    gap: 4px;
  }
  .uris {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .link {
    justify-self: start;
    align-self: flex-start;
    color: var(--accent);
    padding: 2px 0;
  }
  .link:hover {
    text-decoration: underline;
  }
  .err {
    margin-top: 14px;
  }
  footer {
    display: flex;
    gap: 8px;
    padding: 10px 14px;
    border-top: 1px solid var(--border);
  }
  .sp {
    flex: 1;
  }
</style>
