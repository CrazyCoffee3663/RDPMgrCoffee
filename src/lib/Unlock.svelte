<script lang="ts">
  import { onMount } from "svelte";
  import { api, errText } from "./api";
  import { app } from "./state.svelte";
  import Icon from "./Icon.svelte";
  import WindowControls from "./WindowControls.svelte";
  import { dialogs } from "./dialogs.svelte";

  const PROVIDER_NAMES: Record<number, string> = {
    0: "Приложение-аутентификатор",
    1: "Код из письма",
    3: "YubiKey OTP",
  };

  let step = $state<"creds" | "twoFactor" | "newDevice">("creds");
  /** Где хранить данные: сервер Bitwarden или локальный зашифрованный файл */
  let where = $state<"bitwarden" | "local">("bitwarden");
  let password2 = $state("");
  let understood = $state(false);
  let server = $state(app.status.server || "");
  let email = $state(app.status.email || "");
  let password = $state("");
  let code = $state("");
  let providers = $state<number[]>([]);
  let provider = $state(0);
  let busy = $state(false);
  let error = $state("");
  let info = $state("");
  let legacyNote = $state("");

  // Identifier сменился (com.user.connmgr → io.github.connmgr): подставляем сервер/почту
  // из данных прежней версии (bw CLI). Пароль не трогаем, старый каталог не удаляем.
  onMount(async () => {
    if (app.phase === "setup" && !server && !email) {
      const h = await api.legacyHint().catch(() => null);
      if (h && (h.server || h.email)) {
        server = h.server;
        email = h.email;
        legacyNote = "Адрес и почта подставлены из прежней версии программы. Мастер-пароль введите заново.";
      }
    }
  });

  const supported = $derived(providers.filter((p) => p in PROVIDER_NAMES));

  async function handle(r: { status: string; providers: number[] }) {
    if (r.status === "ok") {
      password = "";
      code = "";
      await app.afterUnlock(false);
    } else if (r.status === "twoFactor") {
      if (step === "twoFactor") error = "Неверный код";
      providers = r.providers;
      if (!r.providers.includes(provider)) provider = supported[0] ?? r.providers[0];
      step = "twoFactor";
      code = "";
      if (provider === 1 && supported.length === 1) await sendEmail();
    } else if (r.status === "newDevice") {
      step = "newDevice";
      code = "";
      info = "Bitwarden отправил код подтверждения нового устройства на вашу почту.";
    }
  }

  async function run(f: () => Promise<void>) {
    error = "";
    busy = true;
    try {
      await f();
    } catch (e) {
      error = errText(e);
    }
    busy = false;
  }

  const login = () => run(async () => handle(await api.login(server, email, password)));
  const submitCode = () =>
    run(async () => {
      if (step === "twoFactor") await handle(await api.login2fa(provider, code));
      else await handle(await api.loginNewDevice(code));
    });
  async function sendEmail() {
    await run(async () => {
      await api.sendEmailCode();
      info = "Код отправлен на почту.";
    });
  }
  const unlock = () =>
    run(async () => {
      const hasCache = await api.unlock(password);
      password = "";
      await app.afterUnlock(hasCache);
    });
  async function switchAccount() {
    if (
      app.status.local &&
      !(await dialogs.confirm(
        "Сменить хранилище?",
        "Файлы локального хранилища будут перенесены в папку резервной копии рядом с ним (ничего не удаляется). Вернуть их можно, восстановив из резервной копии.",
        "Сменить",
        true,
      ))
    )
      return;
    await run(async () => {
      await app.logout();
      step = "creds";
      password = "";
    });
  }

  const createLocal = () =>
    run(async () => {
      if (password !== password2) throw "Пароли не совпадают";
      await api.createLocal(password);
      password = "";
      password2 = "";
      await app.afterUnlock(true);
    });

  async function restore(e: Event) {
    const f = (e.currentTarget as HTMLInputElement).files?.[0];
    (e.currentTarget as HTMLInputElement).value = "";
    if (!f) return;
    await run(async () => {
      await api.localRestore(await f.text());
      app.status = await api.status();
      app.phase = "locked";
    });
  }
  const pwScore = $derived(
    (password.length >= 12 ? 1 : 0) +
      (password.length >= 16 ? 1 : 0) +
      (/[a-zа-я]/.test(password) && /[A-ZА-Я]/.test(password) ? 1 : 0) +
      (/\d/.test(password) ? 1 : 0) +
      (/[^\p{L}\d]/u.test(password) ? 1 : 0),
  );
</script>

<div class="titlebar" data-tauri-drag-region>
  <span class="tb-name" data-tauri-drag-region>RDPMgrCoffee</span>
  <WindowControls />
</div>
<div class="wrap">
  <div class="card">
    <div class="logo">
      <div class="mark"><Icon name="plug" size={22} /></div>
      <div>
        <div class="title">RDPMgrCoffee</div>
        <div class="muted small">Менеджер подключений с зашифрованным хранилищем</div>
      </div>
    </div>

    {#if app.phase === "locked"}
      <form onsubmit={(e) => (e.preventDefault(), unlock())}>
        <div class="who">
          {#if app.status.local}
            <Icon name="lock" size={14} />
            <span>Локальное хранилище</span>
            <span class="faint">· этот компьютер</span>
          {:else}
            <Icon name="user" size={14} />
            <span class="selectable">{app.status.email}</span>
            <span class="faint">· {app.status.server.replace(/^https?:\/\//, "")}</span>
          {/if}
        </div>
        <!-- svelte-ignore a11y_autofocus -->
        <input type="password" placeholder="Мастер-пароль" bind:value={password} disabled={busy} autofocus />
        <button class="btn primary" type="submit" disabled={busy || !password}>
          <Icon name="unlock" size={15} />
          {busy ? "Разблокировка…" : "Разблокировать"}
        </button>
        <button type="button" class="link" onclick={switchAccount} disabled={busy}>
          {app.status.local ? "Другое хранилище или Bitwarden" : "Сменить аккаунт или сервер"}
        </button>
      </form>
    {:else if step === "creds"}
      <div class="where">
        <button class:on={where === "bitwarden"} onclick={() => ((where = "bitwarden"), (error = ""))} disabled={busy}>
          <Icon name="cloud" size={14} /> Bitwarden / Vaultwarden
        </button>
        <button class:on={where === "local"} onclick={() => ((where = "local"), (error = ""))} disabled={busy}>
          <Icon name="lock" size={14} /> Только на этом компьютере
        </button>
      </div>
    {/if}
    {#if app.phase !== "locked" && step === "creds" && where === "local"}
      <form onsubmit={(e) => (e.preventDefault(), createLocal())}>
        <p class="muted small">
          Подключения и пароли хранятся в файле на этом компьютере, полностью зашифрованном (AES-256 + HMAC, ключ из
          мастер-пароля через Argon2id). Без мастер-пароля файл прочитать нельзя.
        </p>
        <label>
          <span>Мастер-пароль <small class="faint">(не короче 12 символов)</small></span>
          <input type="password" bind:value={password} disabled={busy} autocomplete="new-password" />
        </label>
        <div class="meter" data-s={pwScore}><i></i></div>
        <label>
          <span>Повторите мастер-пароль</span>
          <input type="password" bind:value={password2} disabled={busy} autocomplete="new-password" />
        </label>
        <label class="chk small">
          <input type="checkbox" bind:checked={understood} disabled={busy} />
          <span>Я понимаю: забытый мастер-пароль восстановить невозможно, данные будут потеряны.</span>
        </label>
        <button
          class="btn primary"
          type="submit"
          disabled={busy || password.length < 12 || password !== password2 || !understood}
        >
          {busy ? "Создание…" : "Создать хранилище"}
        </button>
        <label class="link restore">
          Восстановить из резервной копии (.rdpvault)…
          <input type="file" accept=".rdpvault,.json" hidden onchange={restore} disabled={busy} />
        </label>
        <p class="faint small">Без синхронизации между компьютерами. Делайте резервные копии в настройках.</p>
      </form>
    {:else if app.phase !== "locked" && step === "creds"}
      <form onsubmit={(e) => (e.preventDefault(), login())}>
        {#if legacyNote}<p class="faint small">{legacyNote}</p>{/if}
        <label>
          <span>Сервер</span>
          <input placeholder="https://vault.bitwarden.com или адрес Vaultwarden" bind:value={server} disabled={busy} />
        </label>
        <label>
          <span>Почта</span>
          <input type="email" bind:value={email} disabled={busy} />
        </label>
        <label>
          <span>Мастер-пароль</span>
          <input type="password" bind:value={password} disabled={busy} />
        </label>
        <button class="btn primary" type="submit" disabled={busy || !email || !password}>
          {busy ? "Вход…" : "Войти"}
        </button>
        <p class="faint small">
          Пароли не покидают компьютер: шифрование и расшифровка выполняются локально, как в официальных
          клиентах Bitwarden.
        </p>
      </form>
    {:else if app.phase !== "locked" && step !== "creds"}
      <form onsubmit={(e) => (e.preventDefault(), submitCode())}>
        {#if step === "twoFactor"}
          <div class="subtitle">Двухфакторная аутентификация</div>
          {#if supported.length > 1}
            <select bind:value={provider} disabled={busy}>
              {#each supported as p}<option value={p}>{PROVIDER_NAMES[p]}</option>{/each}
            </select>
          {:else if supported.length === 1}
            <div class="muted">{PROVIDER_NAMES[supported[0]]}</div>
          {:else}
            <div class="error-text">
              Включённые способы 2FA (Duo, WebAuthn/FIDO2) не поддерживаются. Добавьте в Bitwarden
              приложение-аутентификатор или вход по коду из письма.
            </div>
          {/if}
        {:else}
          <div class="subtitle">Подтверждение нового устройства</div>
        {/if}
        <!-- svelte-ignore a11y_autofocus -->
        <input placeholder="Код" bind:value={code} disabled={busy} autofocus autocomplete="one-time-code" />
        <button class="btn primary" type="submit" disabled={busy || !code}>Продолжить</button>
        {#if step === "twoFactor" && provider === 1}
          <button type="button" class="link" onclick={sendEmail} disabled={busy}>Отправить код ещё раз</button>
        {/if}
        <button type="button" class="link" onclick={() => ((step = "creds"), (error = ""), (info = ""))} disabled={busy}>
          Назад
        </button>
        {#if info}<p class="muted small">{info}</p>{/if}
      </form>
    {/if}

    {#if error}<p class="error-text">{error}</p>{/if}
  </div>
</div>

<style>
  .where {
    display: flex;
    gap: 4px;
    padding: 3px;
    border-radius: var(--radius);
    background: var(--bg-sunken);
    margin-bottom: 12px;
  }
  .where button {
    flex: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 6px 8px;
    font-size: 12.5px;
    border-radius: var(--radius-sm);
    color: var(--text-muted);
  }
  .where button.on {
    background: var(--bg-panel);
    color: var(--text);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.15);
  }
  .meter {
    height: 4px;
    border-radius: 2px;
    background: var(--bg-sunken);
    overflow: hidden;
    margin: -4px 0 4px;
  }
  .meter i {
    display: block;
    height: 100%;
    width: 0;
    background: var(--danger);
    transition: width 0.2s;
  }
  .meter[data-s="1"] i { width: 25%; }
  .meter[data-s="2"] i { width: 50%; background: var(--warn); }
  .meter[data-s="3"] i { width: 75%; background: var(--warn); }
  .meter[data-s="4"] i,
  .meter[data-s="5"] i { width: 100%; background: var(--ok); }
  .chk {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    flex-direction: row;
  }
  .chk input {
    width: auto;
    margin-top: 2px;
  }
  .restore {
    cursor: pointer;
    text-align: center;
  }
  .titlebar {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    z-index: 5;
    display: flex;
    align-items: center;
    height: 34px;
    padding: 6px 8px;
  }
  .tb-name {
    flex: 1;
    padding-left: 6px;
    font-size: 12px;
    color: var(--text-faint);
  }
  .wrap {
    height: 100vh;
    display: grid;
    place-items: center;
    background:
      radial-gradient(1200px 600px at 10% -10%, var(--accent-soft), transparent 60%),
      var(--bg);
  }
  .card {
    width: 380px;
    padding: 28px;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 14px;
    box-shadow: var(--shadow);
  }
  .logo {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 22px;
  }
  .mark {
    width: 40px;
    height: 40px;
    border-radius: 10px;
    display: grid;
    place-items: center;
    background: var(--accent);
    color: #fff;
  }
  .title {
    font-size: 18px;
    font-weight: 600;
  }
  .subtitle {
    font-size: 14px;
    font-weight: 600;
  }
  .small {
    font-size: 12px;
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  label span {
    font-size: 12px;
    color: var(--text-muted);
  }
  .btn {
    padding: 8px 14px;
  }
  .who {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-muted);
    overflow: hidden;
    white-space: nowrap;
  }
  .link {
    color: var(--text-muted);
    padding: 2px;
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .link:hover {
    color: var(--text);
  }
  p {
    margin: 0;
  }
</style>
