<script lang="ts">
  import { api, errText } from "./api";
  import { app } from "./state.svelte";
  import Icon from "./Icon.svelte";
  import WindowControls from "./WindowControls.svelte";

  const PROVIDER_NAMES: Record<number, string> = {
    0: "Приложение-аутентификатор",
    1: "Код из письма",
    3: "YubiKey OTP",
  };

  let step = $state<"creds" | "twoFactor" | "newDevice">("creds");
  let server = $state(app.status.server || "");
  let email = $state(app.status.email || "");
  let password = $state("");
  let code = $state("");
  let providers = $state<number[]>([]);
  let provider = $state(0);
  let busy = $state(false);
  let error = $state("");
  let info = $state("");

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
  const switchAccount = () =>
    run(async () => {
      await app.logout();
      step = "creds";
      password = "";
    });
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
        <div class="muted small">Менеджер подключений с хранилищем Bitwarden</div>
      </div>
    </div>

    {#if app.phase === "locked"}
      <form onsubmit={(e) => (e.preventDefault(), unlock())}>
        <div class="who">
          <Icon name="user" size={14} />
          <span class="selectable">{app.status.email}</span>
          <span class="faint">· {app.status.server.replace(/^https?:\/\//, "")}</span>
        </div>
        <!-- svelte-ignore a11y_autofocus -->
        <input type="password" placeholder="Мастер-пароль" bind:value={password} disabled={busy} autofocus />
        <button class="btn primary" type="submit" disabled={busy || !password}>
          <Icon name="unlock" size={15} />
          {busy ? "Разблокировка…" : "Разблокировать"}
        </button>
        <button type="button" class="link" onclick={switchAccount} disabled={busy}>Сменить аккаунт или сервер</button>
      </form>
    {:else if step === "creds"}
      <form onsubmit={(e) => (e.preventDefault(), login())}>
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
    {:else}
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
