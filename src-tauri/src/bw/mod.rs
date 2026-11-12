//! Собственный клиент Bitwarden / Vaultwarden: вход (с 2FA), синхронизация,
//! локальная разблокировка по кешу, создание/изменение/удаление записей и папок.

pub mod crypto;

use base64::Engine;
use crypto::{Kdf, SymKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use zeroize::Zeroize;

pub type VResult<T> = Result<T, String>;

const DEVICE_TYPE: &str = "6"; // Windows Desktop
const CLIENT_ID: &str = "desktop";
const CLIENT_VERSION: &str = "2025.6.0";

// ---------- Сохраняемое состояние ----------

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub server: String,
    pub email: String,
    pub device_id: String,
    pub kdf: Option<Kdf>,
    /// Ключ пользователя, зашифрованный мастер-ключом
    pub protected_key: String,
    /// Refresh-токен, зашифрованный ключом пользователя
    pub refresh_token: Option<String>,
    /// Токен «запомнить устройство» для 2FA, зашифрован ключом пользователя
    pub remember_token: Option<String>,
    /// Время последней синхронизации (мс)
    pub last_sync: i64,
    /// Локальное хранилище без сервера. Поле email тогда хранит случайную соль для KDF.
    #[serde(default)]
    pub local: bool,
}

struct Unlocked {
    user_key: SymKey,
    org_keys: HashMap<String, SymKey>,
    access_token: Option<String>,
    token_expiry: Option<Instant>,
    refresh_token: Option<String>,
    master_hash: String,
    sync: Value,
}

impl Drop for Unlocked {
    fn drop(&mut self) {
        self.master_hash.zeroize();
        if let Some(t) = self.refresh_token.as_mut() {
            t.zeroize();
        }
        if let Some(t) = self.access_token.as_mut() {
            t.zeroize();
        }
    }
}

struct Pending {
    server: String,
    email: String,
    kdf: Kdf,
    master_key: [u8; 32],
    hash: String,
    device_id: String,
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.master_key.zeroize();
        self.hash.zeroize();
    }
}

#[derive(Default)]
struct Inner {
    account: Option<Account>,
    unlocked: Option<Unlocked>,
    pending: Option<Pending>,
}

pub struct Vault {
    dir: PathBuf,
    http: reqwest::Client,
    inner: Mutex<Inner>,
}

// ---------- Данные для интерфейса ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: String,
    pub server: String,
    pub email: String,
    pub local: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FolderV {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CollectionV {
    pub id: String,
    pub org_id: String,
    pub name: String,
    pub read_only: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OrgV {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FieldV {
    pub name: String,
    /// None — значение скрыто (поле типа «скрытое») и не передаётся в интерфейс
    pub value: Option<String>,
    /// 0 текст, 1 скрытое, 2 флажок
    pub kind: u8,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ItemV {
    pub id: String,
    pub org_id: Option<String>,
    pub folder_id: Option<String>,
    pub collection_ids: Vec<String>,
    /// 1 логин, 2 заметка, 3 карта, 4 личность, 5 SSH-ключ
    pub kind: u8,
    pub name: String,
    pub notes: String,
    pub username: String,
    pub has_password: bool,
    pub has_totp: bool,
    pub uris: Vec<String>,
    pub fields: Vec<FieldV>,
    pub favorite: bool,
    pub edit: bool,
    pub revision: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultView {
    pub folders: Vec<FolderV>,
    pub collections: Vec<CollectionV>,
    pub orgs: Vec<OrgV>,
    pub items: Vec<ItemV>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Secret {
    pub password: String,
    pub totp: String,
    pub fields: Vec<FieldV>,
    pub ssh_private_key: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemInput {
    pub id: Option<String>,
    pub kind: Option<u8>,
    pub org_id: Option<String>,
    #[serde(default)]
    pub collection_ids: Vec<String>,
    pub folder_id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub username: String,
    /// None — пароль не менять
    pub password: Option<String>,
    #[serde(default)]
    pub uris: Vec<String>,
    #[serde(default)]
    pub fields: Vec<FieldV>,
    #[serde(default)]
    pub favorite: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResult {
    /// ok | twoFactor | newDevice
    pub status: String,
    pub providers: Vec<u8>,
}

/// Расшифрованные данные записи для подключения (не уходят в интерфейс).
pub struct ConnSecret {
    pub item: ItemV,
    pub password: String,
    pub fields: Vec<FieldV>,
    pub ssh_private_key: String,
}

// ---------- Вспомогательные функции ----------

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Текущее время в ISO 8601 (UTC).
fn now_iso() -> String {
    let secs = (now_ms() / 1000) as i64;
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // Алгоритм civil_from_days (Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000Z",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Приводим ключи JSON к camelCase (старый Vaultwarden отдаёт PascalCase).
fn camelize(v: Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut out = Map::with_capacity(m.len());
            for (k, v) in m {
                let mut c = k.chars();
                let key = match c.next() {
                    Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
                    None => k,
                };
                out.insert(key, camelize(v));
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.into_iter().map(camelize).collect()),
        other => other,
    }
}

fn s(v: &Value) -> Option<&str> {
    v.as_str().filter(|x| !x.is_empty())
}

struct Endpoints {
    api: String,
    identity: String,
}

fn endpoints(server: &str) -> Endpoints {
    let lower = server.to_lowercase();
    let host = lower
        .split_once("://")
        .map(|(_, r)| r)
        .unwrap_or(&lower)
        .split('/')
        .next()
        .unwrap_or("")
        .to_string();
    if host == "vault.bitwarden.com" || host == "bitwarden.com" {
        return Endpoints {
            api: "https://api.bitwarden.com".into(),
            identity: "https://identity.bitwarden.com".into(),
        };
    }
    if host == "vault.bitwarden.eu" || host == "bitwarden.eu" {
        return Endpoints {
            api: "https://api.bitwarden.eu".into(),
            identity: "https://identity.bitwarden.eu".into(),
        };
    }
    Endpoints {
        api: format!("{}/api", server),
        identity: format!("{}/identity", server),
    }
}

fn normalize_server(server: &str) -> VResult<String> {
    let s = server.trim().trim_end_matches('/').to_string();
    if s.is_empty() {
        return Ok("https://vault.bitwarden.com".into());
    }
    if !s.starts_with("https://") && !s.starts_with("http://") {
        return Ok(format!("https://{}", s));
    }
    Ok(s)
}

/// Текст ошибки из ответа сервера.
fn server_error(v: &Value, status: u16) -> String {
    let candidates = [
        &v["errorModel"]["message"],
        &v["error_description"],
        &v["message"],
        &v["error"],
    ];
    for c in candidates {
        if let Some(t) = s(c) {
            return t.to_string();
        }
    }
    if let Some(obj) = v["validationErrors"].as_object() {
        if let Some((_, msgs)) = obj.iter().next() {
            if let Some(m) = msgs[0].as_str() {
                return m.to_string();
            }
        }
    }
    format!("Сервер вернул ошибку {}", status)
}

fn net_err(e: reqwest::Error) -> String {
    if e.is_connect() || e.is_timeout() {
        format!("Сервер недоступен: {}", e)
    } else {
        format!("Ошибка сети: {}", e)
    }
}

impl Vault {
    pub fn new(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        let http = reqwest::Client::builder()
            .user_agent(format!("connmgr/{}", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()
            .expect("http client");
        let account = std::fs::read_to_string(dir.join("account.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Account>(&t).ok())
            .filter(|a| !a.protected_key.is_empty());
        Self {
            dir,
            http,
            inner: Mutex::new(Inner {
                account,
                ..Default::default()
            }),
        }
    }

    fn write_file(&self, name: &str, data: &[u8]) -> VResult<()> {
        let tmp = self.dir.join(format!("{}.tmp", name));
        std::fs::write(&tmp, data).map_err(|e| format!("Запись {}: {}", name, e))?;
        std::fs::rename(&tmp, self.dir.join(name)).map_err(|e| format!("Запись {}: {}", name, e))
    }

    fn save_account(&self, a: &Account) -> VResult<()> {
        self.write_file("account.json", serde_json::to_string_pretty(a).unwrap().as_bytes())
    }

    /// Сохранить кеш/хранилище и его контрольную сумму (HMAC ключом пользователя):
    /// зашифрованные поля можно переставить между записями (адрес одной — пароль другой),
    /// контрольная сумма всего файла не даёт сделать это незаметно.
    fn save_cache(&self, sync: &Value, key: &SymKey) {
        if let Ok(t) = serde_json::to_vec(sync) {
            // У локального хранилища этот файл — единственная копия: держим предыдущую версию
            if self.dir.join("local.flag").exists() {
                let _ = std::fs::copy(self.dir.join("cache.json"), self.dir.join("cache.json.bak"));
                let _ = std::fs::copy(self.dir.join("cache.mac"), self.dir.join("cache.mac.bak"));
            }
            let _ = self.write_file("cache.mac", crypto::file_mac(key, &t).as_bytes());
            let _ = self.write_file("cache.json", &t);
        }
    }

    async fn is_local(&self) -> bool {
        self.inner.lock().await.account.as_ref().is_some_and(|a| a.local)
    }

    /// Убрать файлы локального хранилища в папку резервной копии (ничего не удаляется).
    fn stash_local(&self) -> VResult<PathBuf> {
        let dir = self.dir.join(format!("local-backup-{}", now_ms()));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        for f in ["account.json", "cache.json", "cache.json.bak", "cache.mac", "cache.mac.bak", "local.flag"] {
            let p = self.dir.join(f);
            if p.exists() {
                std::fs::rename(&p, dir.join(f)).map_err(|e| format!("Не удалось сохранить {}: {}", f, e))?;
            }
        }
        Ok(dir)
    }

    // ---------- Локальное хранилище (без сервера) ----------

    /// Создать новое локальное хранилище с мастер-паролем.
    pub async fn create_local(&self, password: &str) -> VResult<()> {
        if password.chars().count() < 12 {
            return Err("Мастер-пароль должен быть не короче 12 символов".into());
        }
        let device_id = {
            let g = self.inner.lock().await;
            if g.account.as_ref().is_some_and(|a| !a.protected_key.is_empty()) {
                return Err("Хранилище уже настроено".into());
            }
            g.account.as_ref().map(|a| a.device_id.clone()).filter(|d| !d.is_empty())
        }
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        // Argon2id с параметрами Bitwarden по умолчанию; соль случайная
        let kdf = Kdf { kind: 1, iterations: 3, memory: Some(64), parallelism: Some(4) };
        let salt = uuid::Uuid::new_v4().to_string();
        let (pw, s2, k2) = (password.to_string(), salt.clone(), kdf.clone());
        let (protected, user_key) = tokio::task::spawn_blocking(move || -> VResult<(String, SymKey)> {
            let mut mk = crypto::master_key(&pw, &s2, &k2)?;
            let uk = SymKey::generate();
            let p = crypto::protect_user_key(&mk, &uk);
            mk.zeroize();
            Ok((p, uk))
        })
        .await
        .map_err(|e| e.to_string())??;
        let account = Account {
            server: String::new(),
            email: salt,
            device_id,
            kdf: Some(kdf),
            protected_key: protected,
            refresh_token: None,
            remember_token: None,
            last_sync: 0,
            local: true,
        };
        let sync = json!({
            "object": "sync",
            "profile": { "id": uuid::Uuid::new_v4().to_string() },
            "folders": [],
            "collections": [],
            "ciphers": [],
        });
        let _ = std::fs::remove_file(self.dir.join("cache.json"));
        self.write_file("local.flag", b"1")?;
        self.save_account(&account)?;
        self.save_cache(&sync, &user_key);
        let mut g = self.inner.lock().await;
        g.account = Some(account);
        g.unlocked = Some(Unlocked {
            user_key,
            org_keys: HashMap::new(),
            access_token: None,
            token_expiry: None,
            refresh_token: None,
            master_hash: String::new(),
            sync,
        });
        Ok(())
    }

    /// Сменить мастер-пароль локального хранилища: перешифровывается только ключ пользователя.
    pub async fn change_local_password(&self, old: &str, new: &str) -> VResult<()> {
        if new.chars().count() < 12 {
            return Err("Новый мастер-пароль должен быть не короче 12 символов".into());
        }
        let account = {
            let g = self.inner.lock().await;
            g.unlocked.as_ref().ok_or("Хранилище заблокировано")?;
            g.account.clone().filter(|a| a.local).ok_or("Это не локальное хранилище")?
        };
        let kdf = account.kdf.clone().ok_or("Нет параметров ключа")?;
        let (old, new) = (old.to_string(), new.to_string());
        let (salt, protected) = (account.email.clone(), account.protected_key.clone());
        let new_salt = uuid::Uuid::new_v4().to_string();
        let (ns, k2) = (new_salt.clone(), kdf.clone());
        let new_protected = tokio::task::spawn_blocking(move || -> VResult<String> {
            let mut mk = crypto::master_key(&old, &salt, &k2)?;
            let uk = crypto::decrypt_user_key(&mk, &protected).map_err(|_| "Текущий мастер-пароль неверен".to_string());
            mk.zeroize();
            let uk = uk?;
            let mut nk = crypto::master_key(&new, &ns, &k2)?;
            let p = crypto::protect_user_key(&nk, &uk);
            nk.zeroize();
            Ok(p)
        })
        .await
        .map_err(|e| e.to_string())??;
        let mut g = self.inner.lock().await;
        let a = g.account.as_mut().ok_or("Учётная запись не настроена")?;
        a.email = new_salt;
        a.protected_key = new_protected;
        let acc = a.clone();
        self.save_account(&acc)
    }

    /// Резервная копия локального хранилища: те же зашифрованные данные одним файлом.
    pub async fn local_backup(&self) -> VResult<String> {
        let g = self.inner.lock().await;
        let a = g.account.clone().filter(|a| a.local).ok_or("Это не локальное хранилище")?;
        let u = g.unlocked.as_ref().ok_or("Хранилище заблокировано")?;
        let data = serde_json::to_vec(&u.sync).unwrap();
        Ok(serde_json::to_string_pretty(&json!({
            "format": "rdpmgrcoffee-vault",
            "version": 1,
            "created": now_iso(),
            "account": a,
            "data": u.sync,
            "mac": crypto::file_mac(&u.user_key, &data),
        }))
        .unwrap())
    }

    /// Восстановить локальное хранилище из резервной копии (только на экране настройки).
    pub async fn local_restore(&self, text: &str) -> VResult<()> {
        let v: Value = serde_json::from_str(text).map_err(|_| "Файл повреждён: это не JSON".to_string())?;
        if v["format"] != json!("rdpmgrcoffee-vault") {
            return Err("Это не резервная копия хранилища RDPMgrCoffee".into());
        }
        let mut account: Account =
            serde_json::from_value(v["account"].clone()).map_err(|_| "В копии нет данных хранилища".to_string())?;
        if !account.local || account.protected_key.is_empty() || account.kdf.is_none() || !v["data"]["ciphers"].is_array() {
            return Err("В копии нет данных хранилища".into());
        }
        {
            let g = self.inner.lock().await;
            if g.account.as_ref().is_some_and(|a| !a.protected_key.is_empty()) {
                return Err("Хранилище уже настроено".into());
            }
            if let Some(d) = g.account.as_ref().map(|a| a.device_id.clone()).filter(|d| !d.is_empty()) {
                account.device_id = d;
            }
        }
        let _ = std::fs::remove_file(self.dir.join("cache.json"));
        self.write_file("local.flag", b"1")?;
        let mac = v["mac"].as_str().ok_or("В резервной копии нет контрольной суммы — она создана старой версией программы")?;
        self.write_file("cache.mac", mac.as_bytes())?;
        self.write_file("cache.json", &serde_json::to_vec(&v["data"]).unwrap())?;
        self.save_account(&account)?;
        let mut g = self.inner.lock().await;
        g.account = Some(account);
        g.unlocked = None;
        Ok(())
    }

    /// Записать изменённую/новую запись локального хранилища.
    async fn local_save(&self, input: ItemInput) -> VResult<ItemV> {
        if input.org_id.is_some() {
            return Err("В локальном хранилище нет организаций".into());
        }
        let body = {
            let g = self.inner.lock().await;
            let u = g.unlocked.as_ref().ok_or("Хранилище заблокировано")?;
            match &input.id {
                Some(id) => {
                    let raw = find_cipher(&u.sync, id)?.clone();
                    let key = cipher_key(u, &raw)?;
                    let mut b = build_cipher(&key, Some(&raw), &input);
                    b["id"] = json!(id);
                    b["creationDate"] = raw["creationDate"].clone();
                    b
                }
                None => {
                    let mut b = build_cipher(&u.user_key, None, &input);
                    b["id"] = json!(uuid::Uuid::new_v4().to_string());
                    b["creationDate"] = json!(now_iso());
                    b
                }
            }
        };
        self.local_store(body).await
    }

    async fn local_store(&self, mut c: Value) -> VResult<ItemV> {
        if let Some(m) = c.as_object_mut() {
            m.remove("lastKnownRevisionDate");
            m.remove("encryptedFor");
        }
        c["object"] = json!("cipher");
        c["revisionDate"] = json!(now_iso());
        c["edit"] = json!(true);
        c["viewPassword"] = json!(true);
        c["organizationId"] = Value::Null;
        c["deletedDate"] = Value::Null;
        c["collectionIds"] = json!([]);
        self.store_cipher(c).await
    }

    /// Загрузить кеш с проверкой контрольной суммы. Кеш Bitwarden без суммы или с неверной
    /// суммой отбрасывается (данные заново придут с сервера); локальное хранилище — ошибка.
    fn load_cache(&self, key: &SymKey, local: bool) -> VResult<Option<Value>> {
        let Ok(t) = std::fs::read(self.dir.join("cache.json")) else { return Ok(None) };
        let mac = std::fs::read_to_string(self.dir.join("cache.mac")).unwrap_or_default();
        if !crypto::file_mac_ok(key, &t, &mac) {
            if local {
                return Err(format!(
                    "Файл хранилища изменён вне программы или повреждён ({}) — данные не загружены. Восстановите хранилище из резервной копии.",
                    self.dir.join("cache.json").display()
                ));
            }
            return Ok(None);
        }
        Ok(serde_json::from_slice(&t).ok())
    }

    pub async fn status(&self) -> Status {
        let g = self.inner.lock().await;
        match &g.account {
            None => Status {
                state: "setup".into(),
                server: String::new(),
                email: String::new(),
                local: false,
            },
            Some(a) => Status {
                state: if g.unlocked.is_some() { "unlocked" } else { "locked" }.into(),
                server: if a.local { String::new() } else { a.server.clone() },
                email: if a.local { String::new() } else { a.email.clone() },
                local: a.local,
            },
        }
    }

    // ---------- Вход ----------

    async fn prelogin(&self, server: &str, email: &str) -> VResult<Kdf> {
        let ep = endpoints(server);
        let body = json!({ "email": email });
        // Новые версии сервера держат prelogin в identity, старые — в api
        for url in [
            format!("{}/accounts/prelogin", ep.identity),
            format!("{}/accounts/prelogin", ep.api),
        ] {
            let r = self.http.post(&url).json(&body).send().await.map_err(net_err)?;
            if r.status().is_success() {
                let v = camelize(r.json::<Value>().await.map_err(net_err)?);
                return Ok(Kdf {
                    kind: v["kdf"].as_u64().unwrap_or(0) as u8,
                    iterations: v["kdfIterations"].as_u64().unwrap_or(600000) as u32,
                    memory: v["kdfMemory"].as_u64().map(|x| x as u32),
                    parallelism: v["kdfParallelism"].as_u64().map(|x| x as u32),
                });
            }
            if r.status().as_u16() != 404 {
                let st = r.status().as_u16();
                let v = camelize(r.json::<Value>().await.unwrap_or(Value::Null));
                return Err(server_error(&v, st));
            }
        }
        Err("Сервер не похож на Bitwarden/Vaultwarden".into())
    }

    /// Шаг 1: адрес, почта, мастер-пароль.
    pub async fn login_start(&self, server: &str, email: &str, password: &str) -> VResult<LoginResult> {
        let server = normalize_server(server)?;
        let email = email.trim().to_lowercase();
        if email.is_empty() {
            return Err("Укажите почту".into());
        }
        let kdf = self.prelogin(&server, &email).await?;
        let pw = password.to_string();
        let em = email.clone();
        let k2 = kdf.clone();
        // Argon2 может работать секунду и дольше — не блокируем асинхронный поток
        let (mk, hash) = tokio::task::spawn_blocking(move || -> VResult<([u8; 32], String)> {
            let mk = crypto::master_key(&pw, &em, &k2)?;
            let h = crypto::master_password_hash(&mk, &pw);
            Ok((mk, h))
        })
        .await
        .map_err(|e| e.to_string())??;

        let device_id = {
            let g = self.inner.lock().await;
            g.account
                .as_ref()
                .map(|a| a.device_id.clone())
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
        };
        {
            let mut g = self.inner.lock().await;
            g.pending = Some(Pending {
                server,
                email,
                kdf,
                master_key: mk,
                hash,
                device_id,
            });
        }
        self.login_continue(None, None, false).await
    }

    /// Шаг 2 (если нужен): код двухфакторной аутентификации.
    pub async fn login_two_factor(&self, provider: u8, code: &str) -> VResult<LoginResult> {
        self.login_continue(Some((provider, code.trim().to_string())), None, true)
            .await
    }

    /// Подтверждение нового устройства кодом из письма (bitwarden.com).
    pub async fn login_new_device(&self, otp: &str) -> VResult<LoginResult> {
        self.login_continue(None, Some(otp.trim().to_string()), false)
            .await
    }

    /// Отправить код 2FA на почту.
    pub async fn send_email_code(&self) -> VResult<()> {
        let g = self.inner.lock().await;
        let p = g.pending.as_ref().ok_or("Сначала введите почту и пароль")?;
        let ep = endpoints(&p.server);
        let r = self
            .http
            .post(format!("{}/two-factor/send-email-login", ep.api))
            .json(&json!({
                "email": p.email,
                "masterPasswordHash": p.hash,
                "deviceIdentifier": p.device_id,
            }))
            .send()
            .await
            .map_err(net_err)?;
        if r.status().is_success() {
            Ok(())
        } else {
            let st = r.status().as_u16();
            let v = camelize(r.json::<Value>().await.unwrap_or(Value::Null));
            Err(server_error(&v, st))
        }
    }

    async fn token_request(
        &self,
        identity: &str,
        email: &str,
        mut form: Vec<(&str, String)>,
    ) -> VResult<(u16, Value)> {
        form.push(("client_id", CLIENT_ID.into()));
        let auth_email = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(email.as_bytes());
        let r = self
            .http
            .post(format!("{}/connect/token", identity))
            .header("Device-Type", DEVICE_TYPE)
            .header("Bitwarden-Client-Name", CLIENT_ID)
            .header("Bitwarden-Client-Version", CLIENT_VERSION)
            .header("Auth-Email", auth_email)
            .form(&form)
            .send()
            .await
            .map_err(net_err)?;
        let st = r.status().as_u16();
        let v = camelize(r.json::<Value>().await.unwrap_or(Value::Null));
        Ok((st, v))
    }

    async fn login_continue(
        &self,
        two_factor: Option<(u8, String)>,
        new_device_otp: Option<String>,
        remember: bool,
    ) -> VResult<LoginResult> {
        let (server, email, hash, device_id) = {
            let g = self.inner.lock().await;
            let p = g.pending.as_ref().ok_or("Вход не начат")?;
            (p.server.clone(), p.email.clone(), p.hash.clone(), p.device_id.clone())
        };
        let ep = endpoints(&server);
        let mut form: Vec<(&str, String)> = vec![
            ("grant_type", "password".into()),
            ("username", email.clone()),
            ("password", hash),
            ("scope", "api offline_access".into()),
            ("deviceType", DEVICE_TYPE.into()),
            ("deviceIdentifier", device_id),
            ("deviceName", "connmgr".into()),
        ];
        if let Some((prov, code)) = two_factor {
            form.push(("twoFactorProvider", prov.to_string()));
            form.push(("twoFactorToken", code));
            form.push(("twoFactorRemember", if remember { "1" } else { "0" }.into()));
        }
        if let Some(otp) = new_device_otp {
            form.push(("newDeviceOtp", otp));
        }
        let (st, v) = self.token_request(&ep.identity, &email, form).await?;

        if st == 200 && v["access_token"].is_string() {
            self.finish_login(v).await?;
            return Ok(LoginResult { status: "ok".into(), providers: vec![] });
        }

        // Нужна двухфакторная аутентификация
        let providers: Vec<u8> = if let Some(obj) = v["twoFactorProviders2"].as_object() {
            obj.keys().filter_map(|k| k.parse().ok()).collect()
        } else if let Some(arr) = v["twoFactorProviders"].as_array() {
            arr.iter()
                .filter_map(|x| x.as_str().and_then(|t| t.parse().ok()).or(x.as_u64().map(|n| n as u8)))
                .collect()
        } else {
            vec![]
        };
        if !providers.is_empty() {
            return Ok(LoginResult { status: "twoFactor".into(), providers });
        }

        let msg = server_error(&v, st);
        let low = msg.to_lowercase();
        if low.contains("new device") || low.contains("device verification") {
            return Ok(LoginResult { status: "newDevice".into(), providers: vec![] });
        }
        if low.contains("username or password is incorrect") || low.contains("invalid_grant") {
            return Err("Неверная почта или мастер-пароль".into());
        }
        Err(msg)
    }

    async fn finish_login(&self, tok: Value) -> VResult<()> {
        let pending = {
            let mut g = self.inner.lock().await;
            g.pending.take().ok_or("Вход не начат")?
        };
        let protected = s(&tok["key"]).ok_or("Сервер не вернул ключ шифрования")?.to_string();
        let user_key = crypto::decrypt_user_key(&pending.master_key, &protected)?;

        let refresh = s(&tok["refresh_token"]).map(|x| x.to_string());
        let remember = s(&tok["twoFactorToken"]).map(|x| user_key.encrypt_str(x));
        let account = Account {
            server: pending.server.clone(),
            email: pending.email.clone(),
            device_id: pending.device_id.clone(),
            kdf: Some(pending.kdf.clone()),
            protected_key: protected,
            refresh_token: refresh.as_ref().map(|r| user_key.encrypt_str(r)),
            remember_token: remember,
            last_sync: 0,
            local: false,
        };
        self.save_account(&account)?;
        let expires = tok["expires_in"].as_u64().unwrap_or(3600);
        let unlocked = Unlocked {
            user_key,
            org_keys: HashMap::new(),
            access_token: s(&tok["access_token"]).map(|x| x.to_string()),
            token_expiry: Some(Instant::now() + Duration::from_secs(expires)),
            refresh_token: refresh,
            master_hash: pending.hash.clone(),
            sync: json!({}),
        };
        // Старый кеш принадлежит другой учётной записи. Локальное хранилище не стираем никогда.
        if self.dir.join("local.flag").exists() {
            self.stash_local()?;
        }
        let _ = std::fs::remove_file(self.dir.join("cache.json"));
        let mut g = self.inner.lock().await;
        g.account = Some(account);
        g.unlocked = Some(unlocked);
        Ok(())
    }

    /// Забыть учётную запись на этом компьютере.
    pub async fn logout(&self) {
        let mut g = self.inner.lock().await;
        g.unlocked = None;
        g.pending = None;
        // Локальное хранилище не удаляем — убираем в папку резервной копии
        if g.account.as_ref().is_some_and(|a| a.local) {
            let device_id = g.account.take().map(|a| a.device_id).unwrap_or_default();
            if self.stash_local().is_ok() {
                let _ = self.save_account(&Account { device_id, ..Default::default() });
            }
            return;
        }
        // device_id сохраняем, чтобы сервер не считал компьютер новым устройством
        if let Some(a) = g.account.take() {
            let keep = Account {
                device_id: a.device_id,
                ..Default::default()
            };
            let _ = self.save_account(&keep);
        }
        let _ = std::fs::remove_file(self.dir.join("cache.json"));
    }

    // ---------- Разблокировка ----------

    /// Разблокировать локально; данные берутся из кеша, если он есть.
    pub async fn unlock(&self, password: &str) -> VResult<bool> {
        let account = {
            let g = self.inner.lock().await;
            g.account.clone().ok_or("Учётная запись не настроена")?
        };
        let kdf = match account.kdf.clone() {
            Some(k) => k,
            None => self.prelogin(&account.server, &account.email).await?,
        };
        let pw = password.to_string();
        let email = account.email.clone();
        let protected = account.protected_key.clone();
        let (user_key, hash) = tokio::task::spawn_blocking(move || -> VResult<(SymKey, String)> {
            let mut mk = crypto::master_key(&pw, &email, &kdf)?;
            let uk = crypto::decrypt_user_key(&mk, &protected);
            let h = crypto::master_password_hash(&mk, &pw);
            mk.zeroize();
            Ok((uk?, h))
        })
        .await
        .map_err(|e| e.to_string())??;

        let refresh = account
            .refresh_token
            .as_deref()
            .and_then(|t| user_key.decrypt_str(t).ok());
        let cache = self.load_cache(&user_key, account.local)?;
        let has_cache = cache.is_some();
        if account.local && !has_cache {
            return Err(format!(
                "Не найден файл данных локального хранилища ({}). Восстановите его из резервной копии.",
                self.dir.join("cache.json").display()
            ));
        }
        let mut u = Unlocked {
            user_key,
            org_keys: HashMap::new(),
            access_token: None,
            token_expiry: None,
            refresh_token: refresh,
            master_hash: hash,
            sync: cache.unwrap_or_else(|| json!({})),
        };
        if has_cache {
            compute_org_keys(&mut u);
        }
        let mut g = self.inner.lock().await;
        g.unlocked = Some(u);
        Ok(has_cache)
    }

    pub async fn lock(&self) {
        let mut g = self.inner.lock().await;
        g.unlocked = None;
        g.pending = None;
    }

    // ---------- Токены и синхронизация ----------

    async fn access_token(&self) -> VResult<String> {
        let (server, email, device_id, refresh, hash, remember, valid) = {
            let g = self.inner.lock().await;
            let a = g.account.as_ref().ok_or("Учётная запись не настроена")?;
            let u = g.unlocked.as_ref().ok_or("Хранилище заблокировано")?;
            if let (Some(t), Some(exp)) = (&u.access_token, u.token_expiry) {
                if exp > Instant::now() + Duration::from_secs(60) {
                    return Ok(t.clone());
                }
            }
            let remember = a
                .remember_token
                .as_deref()
                .and_then(|t| u.user_key.decrypt_str(t).ok());
            (
                a.server.clone(),
                a.email.clone(),
                a.device_id.clone(),
                u.refresh_token.clone(),
                u.master_hash.clone(),
                remember,
                false,
            )
        };
        let _ = valid;
        let ep = endpoints(&server);

        let mut result: Option<Value> = None;
        if let Some(rt) = refresh {
            let (st, v) = self
                .token_request(
                    &ep.identity,
                    &email,
                    vec![("grant_type", "refresh_token".into()), ("refresh_token", rt)],
                )
                .await?;
            if st == 200 && v["access_token"].is_string() {
                result = Some(v);
            }
        }
        if result.is_none() {
            // Refresh-токен истёк: входим заново по паролю (и «запомненному» 2FA)
            let mut form: Vec<(&str, String)> = vec![
                ("grant_type", "password".into()),
                ("username", email.clone()),
                ("password", hash),
                ("scope", "api offline_access".into()),
                ("deviceType", DEVICE_TYPE.into()),
                ("deviceIdentifier", device_id),
                ("deviceName", "connmgr".into()),
            ];
            if let Some(r) = remember {
                form.push(("twoFactorProvider", "5".into()));
                form.push(("twoFactorToken", r));
            }
            let (st, v) = self.token_request(&ep.identity, &email, form).await?;
            if st == 200 && v["access_token"].is_string() {
                result = Some(v);
            } else if v["twoFactorProviders2"].is_object() || v["twoFactorProviders"].is_array() {
                return Err("Сессия на сервере истекла. Нажмите «Сменить аккаунт» и войдите заново с кодом 2FA.".into());
            } else {
                return Err(server_error(&v, st));
            }
        }
        let v = result.unwrap();
        let token = s(&v["access_token"]).unwrap_or("").to_string();
        let expires = v["expires_in"].as_u64().unwrap_or(3600);
        let mut g = self.inner.lock().await;
        let g = &mut *g;
        let (Some(a), Some(u)) = (g.account.as_mut(), g.unlocked.as_mut()) else {
            return Err("Хранилище заблокировано".into());
        };
        u.access_token = Some(token.clone());
        u.token_expiry = Some(Instant::now() + Duration::from_secs(expires));
        if let Some(rt) = s(&v["refresh_token"]) {
            u.refresh_token = Some(rt.to_string());
            a.refresh_token = Some(u.user_key.encrypt_str(rt));
        }
        if let Some(rt) = s(&v["twoFactorToken"]) {
            a.remember_token = Some(u.user_key.encrypt_str(rt));
        }
        let acc = a.clone();
        self.save_account(&acc)?;
        Ok(token)
    }

    async fn api(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> VResult<Value> {
        let token = self.access_token().await?;
        let server = {
            let g = self.inner.lock().await;
            g.account.as_ref().map(|a| a.server.clone()).unwrap_or_default()
        };
        let url = format!("{}{}", endpoints(&server).api, path);
        let mut rb = self.http.request(method, &url).bearer_auth(token);
        if let Some(b) = body {
            rb = rb.json(&b);
        }
        let r = rb.send().await.map_err(net_err)?;
        let st = r.status().as_u16();
        let text = r.text().await.map_err(net_err)?;
        let v = if text.trim().is_empty() {
            Value::Null
        } else {
            camelize(serde_json::from_str(&text).unwrap_or(Value::Null))
        };
        if (200..300).contains(&st) {
            Ok(v)
        } else {
            let mut e = server_error(&v, st);
            if v.is_null() && !text.trim().is_empty() && !text.contains('<') {
                e = format!("{}: {}", e, text.trim().chars().take(200).collect::<String>());
            }
            Err(e)
        }
    }

    /// Синхронизация с сервером. Без force — только если на сервере есть изменения.
    pub async fn sync(&self, force: bool) -> VResult<bool> {
        if self.is_local().await {
            return Ok(false);
        }
        if !force {
            let last = {
                let g = self.inner.lock().await;
                let empty = g
                    .unlocked
                    .as_ref()
                    .map(|u| u.sync["ciphers"].is_null())
                    .unwrap_or(true);
                if empty { 0 } else { g.account.as_ref().map(|a| a.last_sync).unwrap_or(0) }
            };
            if last > 0 {
                let rev = self
                    .api(reqwest::Method::GET, "/accounts/revision-date", None)
                    .await?;
                let rev = rev.as_i64().unwrap_or(i64::MAX);
                if rev <= last {
                    return Ok(false);
                }
            }
        }
        let started = now_ms();
        let data = self
            .api(reqwest::Method::GET, "/sync?excludeDomains=true", None)
            .await?;
        if !data["ciphers"].is_array() {
            return Err("Сервер вернул неожиданный ответ на синхронизацию".into());
        }
        let mut g = self.inner.lock().await;
        let g = &mut *g;
        if let Some(u) = g.unlocked.as_mut() {
            self.save_cache(&data, &u.user_key);
            u.sync = data;
            compute_org_keys(u);
        }
        if let Some(a) = g.account.as_mut() {
            a.last_sync = started;
            let acc = a.clone();
            self.save_account(&acc)?;
        }
        Ok(true)
    }

    // ---------- Чтение ----------

    pub async fn view(&self) -> VResult<VaultView> {
        let g = self.inner.lock().await;
        let u = g.unlocked.as_ref().ok_or("Хранилище заблокировано")?;
        Ok(build_view(u))
    }

    pub async fn secret(&self, id: &str) -> VResult<Secret> {
        let g = self.inner.lock().await;
        let u = g.unlocked.as_ref().ok_or("Хранилище заблокировано")?;
        let raw = find_cipher(&u.sync, id)?;
        let key = cipher_key(u, raw)?;
        let d = |v: &Value| dec(&key, v);
        Ok(Secret {
            password: d(&raw["login"]["password"]),
            totp: d(&raw["login"]["totp"]),
            fields: decrypt_fields(&key, raw, true),
            ssh_private_key: d(&raw["sshKey"]["privateKey"]),
        })
    }

    /// Всё, что нужно для подключения: запись, пароль, все поля.
    pub async fn conn_secret(&self, id: &str) -> VResult<ConnSecret> {
        let g = self.inner.lock().await;
        let u = g.unlocked.as_ref().ok_or("Хранилище заблокировано")?;
        let raw = find_cipher(&u.sync, id)?;
        let key = cipher_key(u, raw)?;
        let item = decrypt_item(u, raw).ok_or("Не удалось расшифровать запись")?;
        Ok(ConnSecret {
            item,
            password: dec(&key, &raw["login"]["password"]),
            fields: decrypt_fields(&key, raw, true),
            ssh_private_key: dec(&key, &raw["sshKey"]["privateKey"]),
        })
    }

    /// Найти запись по id или точному имени (для ссылок вида gateway=Имя).
    pub async fn resolve_ref(&self, r: &str) -> VResult<String> {
        let g = self.inner.lock().await;
        let u = g.unlocked.as_ref().ok_or("Хранилище заблокировано")?;
        let ciphers = u.sync["ciphers"].as_array().cloned().unwrap_or_default();
        if ciphers.iter().any(|c| c["id"].as_str() == Some(r)) {
            return Ok(r.to_string());
        }
        let found: Vec<String> = ciphers
            .iter()
            .filter(|c| c["deletedDate"].is_null())
            .filter_map(|c| {
                let k = cipher_key(u, c).ok()?;
                (dec(&k, &c["name"]) == r).then(|| c["id"].as_str().unwrap_or("").to_string())
            })
            .collect();
        match found.len() {
            0 => Err(format!("Запись «{}» не найдена в хранилище", r)),
            1 => Ok(found[0].clone()),
            _ => Err(format!("Несколько записей называются «{}», оставьте одну", r)),
        }
    }

    // ---------- Изменение ----------

    pub async fn save_item(&self, input: ItemInput) -> VResult<ItemV> {
        if self.is_local().await {
            return self.local_save(input).await;
        }
        let (body, path, method, create_org, profile_id) = {
            let g = self.inner.lock().await;
            let u = g.unlocked.as_ref().ok_or("Хранилище заблокировано")?;
            let profile_id = s(&u.sync["profile"]["id"]).map(String::from);
            match &input.id {
                Some(id) => {
                    let raw = find_cipher(&u.sync, id)?.clone();
                    if raw["edit"] == json!(false) {
                        return Err("Нет прав на изменение этой записи".into());
                    }
                    let key = cipher_key(u, &raw)?;
                    let body = build_cipher(&key, Some(&raw), &input);
                    (body, format!("/ciphers/{}", id), reqwest::Method::PUT, false, profile_id)
                }
                None => {
                    let key = match &input.org_id {
                        Some(o) => u
                            .org_keys
                            .get(o)
                            .cloned()
                            .ok_or("Нет ключа организации")?,
                        None => u.user_key.clone(),
                    };
                    let mut body = build_cipher(&key, None, &input);
                    if let Some(o) = &input.org_id {
                        body["organizationId"] = json!(o);
                        if input.collection_ids.is_empty() {
                            return Err("Для записи организации выберите коллекцию".into());
                        }
                        (body, "/ciphers/create".to_string(), reqwest::Method::POST, true, profile_id)
                    } else {
                        (body, "/ciphers".to_string(), reqwest::Method::POST, false, profile_id)
                    }
                }
            }
        };
        // Vaultwarden >= 1.34 и Bitwarden требуют id владельца ключа, иначе 422
        let mut body = body;
        if let Some(uid) = profile_id {
            body["encryptedFor"] = json!(uid);
        }
        let payload = if create_org {
            json!({ "cipher": body, "collectionIds": input.collection_ids })
        } else {
            body
        };
        let resp = self.api(method, &path, Some(payload)).await?;
        self.store_cipher(resp).await
    }

    /// Переместить запись в папку и/или поменять «избранное» (без перешифрования).
    pub async fn move_item(&self, id: &str, folder_id: Option<String>, favorite: Option<bool>) -> VResult<ItemV> {
        let (cur_folder, cur_fav) = {
            let g = self.inner.lock().await;
            let u = g.unlocked.as_ref().ok_or("Хранилище заблокировано")?;
            let c = find_cipher(&u.sync, id)?;
            (
                c["folderId"].as_str().map(String::from),
                c["favorite"].as_bool().unwrap_or(false),
            )
        };
        // Some("") — в корень, None — оставить как есть
        let folder = match folder_id {
            Some(f) if f.is_empty() => None,
            Some(f) => Some(f),
            None => cur_folder,
        };
        if self.is_local().await {
            let mut c = {
                let g = self.inner.lock().await;
                let u = g.unlocked.as_ref().ok_or("Хранилище заблокировано")?;
                find_cipher(&u.sync, id)?.clone()
            };
            c["folderId"] = json!(folder);
            c["favorite"] = json!(favorite.unwrap_or(cur_fav));
            return self.local_store(c).await;
        }
        let resp = self
            .api(
                reqwest::Method::PUT,
                &format!("/ciphers/{}/partial", id),
                Some(json!({ "folderId": folder, "favorite": favorite.unwrap_or(cur_fav) })),
            )
            .await?;
        self.store_cipher(resp).await
    }

    /// Удалить в корзину.
    pub async fn delete_item(&self, id: &str) -> VResult<()> {
        if !self.is_local().await {
            self.api(reqwest::Method::PUT, &format!("/ciphers/{}/delete", id), None)
                .await?;
        }
        let mut g = self.inner.lock().await;
        if let Some(u) = g.unlocked.as_mut() {
            if let Some(arr) = u.sync["ciphers"].as_array_mut() {
                arr.retain(|c| c["id"].as_str() != Some(id));
            }
            self.save_cache(&u.sync, &u.user_key);
        }
        Ok(())
    }

    async fn store_cipher(&self, resp: Value) -> VResult<ItemV> {
        let id = s(&resp["id"]).ok_or("Сервер не вернул запись")?.to_string();
        let mut g = self.inner.lock().await;
        let u = g.unlocked.as_mut().ok_or("Хранилище заблокировано")?;
        if !u.sync["ciphers"].is_array() {
            u.sync["ciphers"] = json!([]);
        }
        let arr = u.sync["ciphers"].as_array_mut().unwrap();
        match arr.iter_mut().find(|c| c["id"].as_str() == Some(&id)) {
            Some(c) => *c = resp.clone(),
            None => arr.push(resp.clone()),
        }
        self.save_cache(&u.sync, &u.user_key);
        decrypt_item(u, &resp).ok_or_else(|| "Не удалось расшифровать сохранённую запись".into())
    }

    pub async fn save_folder(&self, id: Option<String>, name: &str) -> VResult<FolderV> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Укажите имя папки".into());
        }
        let enc = {
            let g = self.inner.lock().await;
            let u = g.unlocked.as_ref().ok_or("Хранилище заблокировано")?;
            u.user_key.encrypt_str(name)
        };
        let resp = if self.is_local().await {
            json!({
                "id": id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                "name": enc,
                "revisionDate": now_iso(),
                "object": "folder",
            })
        } else {
            match &id {
                Some(id) => {
                    self.api(reqwest::Method::PUT, &format!("/folders/{}", id), Some(json!({ "name": enc })))
                        .await?
                }
                None => self.api(reqwest::Method::POST, "/folders", Some(json!({ "name": enc }))).await?,
            }
        };
        let fid = s(&resp["id"]).ok_or("Сервер не вернул папку")?.to_string();
        let mut g = self.inner.lock().await;
        let u = g.unlocked.as_mut().ok_or("Хранилище заблокировано")?;
        if !u.sync["folders"].is_array() {
            u.sync["folders"] = json!([]);
        }
        let arr = u.sync["folders"].as_array_mut().unwrap();
        match arr.iter_mut().find(|f| f["id"].as_str() == Some(&fid)) {
            Some(f) => *f = resp,
            None => arr.push(resp),
        }
        self.save_cache(&u.sync, &u.user_key);
        Ok(FolderV { id: fid, name: name.to_string() })
    }

    pub async fn delete_folder(&self, id: &str) -> VResult<()> {
        if !self.is_local().await {
            self.api(reqwest::Method::DELETE, &format!("/folders/{}", id), None)
                .await?;
        }
        let mut g = self.inner.lock().await;
        if let Some(u) = g.unlocked.as_mut() {
            if let Some(arr) = u.sync["folders"].as_array_mut() {
                arr.retain(|f| f["id"].as_str() != Some(id));
            }
            // Записи из удалённой папки сервер переносит в корень
            if let Some(arr) = u.sync["ciphers"].as_array_mut() {
                for c in arr.iter_mut() {
                    if c["folderId"].as_str() == Some(id) {
                        c["folderId"] = Value::Null;
                    }
                }
            }
            self.save_cache(&u.sync, &u.user_key);
        }
        Ok(())
    }
}

// ---------- Расшифровка ----------

fn compute_org_keys(u: &mut Unlocked) {
    u.org_keys.clear();
    let Some(pk) = s(&u.sync["profile"]["privateKey"]) else { return };
    let Ok(rsa) = crypto::decrypt_private_key(&u.user_key, pk) else { return };
    if let Some(orgs) = u.sync["profile"]["organizations"].as_array() {
        for o in orgs {
            if let (Some(id), Some(k)) = (s(&o["id"]), s(&o["key"])) {
                if let Ok(bytes) = crypto::rsa_decrypt(&rsa, k) {
                    if let Ok(key) = SymKey::from_bytes(&bytes) {
                        u.org_keys.insert(id.to_string(), key);
                    }
                }
            }
        }
    }
}

fn find_cipher<'a>(sync: &'a Value, id: &str) -> VResult<&'a Value> {
    sync["ciphers"]
        .as_array()
        .and_then(|a| a.iter().find(|c| c["id"].as_str() == Some(id)))
        .ok_or_else(|| "Запись не найдена (возможно, удалена). Обновите список.".to_string())
}

/// Ключ, которым зашифрованы поля записи.
fn cipher_key(u: &Unlocked, raw: &Value) -> VResult<SymKey> {
    let base = match s(&raw["organizationId"]) {
        Some(o) => u
            .org_keys
            .get(o)
            .ok_or("Нет ключа организации для этой записи")?,
        None => &u.user_key,
    };
    match s(&raw["key"]) {
        Some(k) => SymKey::from_bytes(&base.decrypt(k)?),
        None => Ok(base.clone()),
    }
}

fn dec(key: &SymKey, v: &Value) -> String {
    match s(v) {
        Some(t) => key.decrypt_str(t).unwrap_or_else(|_| "[ошибка расшифровки]".into()),
        None => String::new(),
    }
}

fn decrypt_fields(key: &SymKey, raw: &Value, with_hidden: bool) -> Vec<FieldV> {
    raw["fields"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|f| f["type"].as_u64().unwrap_or(0) <= 2)
                .map(|f| {
                    let kind = f["type"].as_u64().unwrap_or(0) as u8;
                    FieldV {
                        name: dec(key, &f["name"]),
                        value: if kind == 1 && !with_hidden {
                            None
                        } else {
                            Some(dec(key, &f["value"]))
                        },
                        kind,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn decrypt_item(u: &Unlocked, raw: &Value) -> Option<ItemV> {
    if !raw["deletedDate"].is_null() {
        return None;
    }
    let key = cipher_key(u, raw).ok()?;
    let d = |v: &Value| dec(&key, v);
    Some(ItemV {
        id: raw["id"].as_str()?.to_string(),
        org_id: s(&raw["organizationId"]).map(String::from),
        folder_id: s(&raw["folderId"]).map(String::from),
        collection_ids: raw["collectionIds"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        kind: raw["type"].as_u64().unwrap_or(0) as u8,
        name: d(&raw["name"]),
        notes: d(&raw["notes"]),
        username: d(&raw["login"]["username"]),
        has_password: s(&raw["login"]["password"]).is_some(),
        has_totp: s(&raw["login"]["totp"]).is_some(),
        uris: raw["login"]["uris"]
            .as_array()
            .map(|a| a.iter().map(|x| d(&x["uri"])).filter(|x| !x.is_empty()).collect())
            .unwrap_or_default(),
        fields: decrypt_fields(&key, raw, false),
        favorite: raw["favorite"].as_bool().unwrap_or(false),
        edit: raw["edit"].as_bool().unwrap_or(true),
        revision: raw["revisionDate"].as_str().unwrap_or("").to_string(),
    })
}

fn build_view(u: &Unlocked) -> VaultView {
    let folders = u.sync["folders"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|f| {
                    Some(FolderV {
                        id: f["id"].as_str()?.to_string(),
                        name: dec(&u.user_key, &f["name"]),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let orgs: Vec<OrgV> = u.sync["profile"]["organizations"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|o| {
                    Some(OrgV {
                        id: o["id"].as_str()?.to_string(),
                        name: o["name"].as_str().unwrap_or("").to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let collections = u.sync["collections"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|c| {
                    let org = c["organizationId"].as_str()?;
                    let key = u.org_keys.get(org)?;
                    Some(CollectionV {
                        id: c["id"].as_str()?.to_string(),
                        org_id: org.to_string(),
                        name: dec(key, &c["name"]),
                        read_only: c["readOnly"].as_bool().unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let items = u.sync["ciphers"]
        .as_array()
        .map(|a| a.iter().filter_map(|c| decrypt_item(u, c)).collect())
        .unwrap_or_default();
    VaultView { folders, collections, orgs, items }
}

/// Собрать тело запроса для создания/изменения записи.
/// Для существующей записи сохраняем всё, что мы не меняем (вложения, passkeys, историю).
fn build_cipher(key: &SymKey, raw: Option<&Value>, input: &ItemInput) -> Value {
    let enc = |t: &str| -> Value {
        if t.is_empty() {
            Value::Null
        } else {
            json!(key.encrypt_str(t))
        }
    };
    const KEEP: [&str; 17] = [
        "archivedDate",
        "type",
        "organizationId",
        "folderId",
        "favorite",
        "reprompt",
        "key",
        "name",
        "notes",
        "fields",
        "passwordHistory",
        "login",
        "card",
        "identity",
        "secureNote",
        "sshKey",
        "revisionDate",
    ];
    let mut body = Map::new();
    if let Some(r) = raw {
        for k in KEEP {
            if !r[k].is_null() {
                body.insert(k.to_string(), r[k].clone());
            }
        }
        if let Some(rd) = body.remove("revisionDate") {
            body.insert("lastKnownRevisionDate".into(), rd);
        }
    } else {
        let kind = input.kind.unwrap_or(1);
        body.insert("type".into(), json!(kind));
        body.insert("reprompt".into(), json!(0));
        if kind == 2 {
            body.insert("secureNote".into(), json!({ "type": 0 }));
        }
    }
    let kind = body["type"].as_u64().unwrap_or(1);

    body.insert("name".into(), enc(input.name.trim()));
    body.insert("notes".into(), enc(&input.notes));
    body.insert("folderId".into(), json!(input.folder_id));
    body.insert("favorite".into(), json!(input.favorite));

    if kind == 1 {
        let mut login = body
            .get("login")
            .cloned()
            .filter(|v| v.is_object())
            .unwrap_or_else(|| json!({}));
        let old_uris = login["uris"].as_array().cloned().unwrap_or_default();
        login["username"] = enc(input.username.trim());
        if let Some(pw) = &input.password {
            let old_pw = raw.map(|r| r["login"]["password"].clone()).unwrap_or(Value::Null);
            let changed = old_pw.is_string()
                && key.decrypt_str(old_pw.as_str().unwrap()).ok().as_deref() != Some(pw.as_str());
            if changed {
                // Старый пароль — в историю, как это делают клиенты Bitwarden
                let mut hist = body
                    .get("passwordHistory")
                    .and_then(|h| h.as_array().cloned())
                    .unwrap_or_default();
                hist.insert(0, json!({ "password": old_pw, "lastUsedDate": now_iso() }));
                hist.truncate(5);
                body.insert("passwordHistory".into(), Value::Array(hist));
                login["passwordRevisionDate"] = json!(now_iso());
            }
            login["password"] = enc(pw);
        }
        let uris: Vec<Value> = input
            .uris
            .iter()
            .map(|u| u.trim())
            .filter(|u| !u.is_empty())
            .enumerate()
            .map(|(i, u)| {
                let m = old_uris.get(i).map(|o| o["match"].clone()).unwrap_or(Value::Null);
                json!({ "uri": key.encrypt_str(u), "match": m })
            })
            .collect();
        login["uris"] = if uris.is_empty() { Value::Null } else { Value::Array(uris) };
        body.insert("login".into(), login);
    }

    // Поля: если значение скрытого поля не передано — оставляем зашифрованное старое
    let old_fields: Vec<(String, Value)> = raw
        .and_then(|r| r["fields"].as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .map(|f| (dec(key, &f["name"]), f["value"].clone()))
        .collect();
    let fields: Vec<Value> = input
        .fields
        .iter()
        .filter(|f| !f.name.trim().is_empty())
        .map(|f| {
            let value = match &f.value {
                Some(v) => enc(v),
                None => old_fields
                    .iter()
                    .find(|(n, _)| n == &f.name)
                    .map(|(_, v)| v.clone())
                    .unwrap_or(Value::Null),
            };
            json!({ "type": f.kind, "name": key.encrypt_str(f.name.trim()), "value": value, "linkedId": null })
        })
        .collect();
    body.insert(
        "fields".into(),
        if fields.is_empty() { Value::Null } else { Value::Array(fields) },
    );
    Value::Object(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unlocked_with(sync: Value) -> Unlocked {
        Unlocked {
            user_key: SymKey::generate(),
            org_keys: HashMap::new(),
            access_token: None,
            token_expiry: None,
            refresh_token: None,
            master_hash: String::new(),
            sync,
        }
    }

    #[test]
    fn camelize_keys() {
        let v = camelize(json!({"Object":"x","Ciphers":[{"Login":{"Uris":[]}}]}));
        assert!(v["ciphers"][0]["login"]["uris"].is_array());
    }

    #[test]
    fn iso_date_format() {
        let d = now_iso();
        assert_eq!(d.len(), 24);
        assert!(d.ends_with('Z'));
    }

    #[test]
    fn build_and_read_back() {
        let mut u = unlocked_with(json!({}));
        let input = ItemInput {
            id: None,
            kind: Some(1),
            org_id: None,
            collection_ids: vec![],
            folder_id: None,
            name: "srv".into(),
            notes: String::new(),
            username: "admin".into(),
            password: Some("p@ss".into()),
            uris: vec!["rdp://10.0.0.1".into()],
            fields: vec![
                FieldV { name: "domain".into(), value: Some("CORP".into()), kind: 0 },
                FieldV { name: "pin".into(), value: Some("1234".into()), kind: 1 },
            ],
            favorite: false,
        };
        let mut body = build_cipher(&u.user_key, None, &input);
        body["id"] = json!("1");
        body["revisionDate"] = json!("2026-01-01T00:00:00Z");
        u.sync = json!({ "ciphers": [body.clone()] });
        let item = decrypt_item(&u, &body).unwrap();
        assert_eq!(item.name, "srv");
        assert_eq!(item.username, "admin");
        assert_eq!(item.uris, vec!["rdp://10.0.0.1"]);
        assert!(item.has_password);
        assert_eq!(item.fields[0].value.as_deref(), Some("CORP"));
        assert_eq!(item.fields[1].value, None);

        // Правка без пароля и без значения скрытого поля сохраняет их
        let edit = ItemInput {
            id: Some("1".into()),
            password: None,
            fields: vec![FieldV { name: "pin".into(), value: None, kind: 1 }],
            name: "srv2".into(),
            ..input
        };
        let body2 = build_cipher(&u.user_key, Some(&body), &edit);
        assert_eq!(body2["login"]["password"], body["login"]["password"]);
        assert_eq!(body2["fields"][0]["value"], body["fields"][1]["value"]);
        assert!(body2["lastKnownRevisionDate"].is_string());

        // Смена пароля кладёт старый в историю
        let edit2 = ItemInput {
            id: Some("1".into()),
            kind: None,
            org_id: None,
            collection_ids: vec![],
            folder_id: None,
            name: "srv".into(),
            notes: String::new(),
            username: "admin".into(),
            password: Some("new".into()),
            uris: vec![],
            fields: vec![],
            favorite: false,
        };
        let body3 = build_cipher(&u.user_key, Some(&body), &edit2);
        assert_eq!(body3["passwordHistory"].as_array().unwrap().len(), 1);
        assert_eq!(
            u.user_key.decrypt_str(body3["login"]["password"].as_str().unwrap()).unwrap(),
            "new"
        );
    }
}

#[cfg(test)]
mod local_tests {
    use super::*;

    fn input(name: &str, pw: &str, folder: Option<String>) -> ItemInput {
        ItemInput {
            id: None,
            kind: Some(1),
            org_id: None,
            collection_ids: vec![],
            folder_id: folder,
            name: name.into(),
            notes: "заметка-секрет".into(),
            username: "CORP\\admin".into(),
            password: Some(pw.into()),
            uris: vec!["rdp://10.9.8.7".into()],
            fields: vec![FieldV { name: "domain".into(), value: Some("CORP".into()), kind: 0 }],
            favorite: false,
        }
    }

    #[tokio::test]
    async fn local_vault_lifecycle() {
        let dir = std::env::temp_dir().join(format!("rdpmgr-local-{}", uuid::Uuid::new_v4()));
        let v = Vault::new(dir.clone());
        assert!(v.create_local("short").await.is_err());
        v.create_local("correct horse battery").await.unwrap();
        assert!(v.status().await.local);

        let f = v.save_folder(None, "Офис-папка").await.unwrap();
        let it = v.save_item(input("Сервер-тайный", "P@ss-Секрет-42", Some(f.id.clone()))).await.unwrap();
        assert_eq!(v.secret(&it.id).await.unwrap().password, "P@ss-Секрет-42");
        let it = v.move_item(&it.id, None, Some(true)).await.unwrap();
        assert!(it.favorite);
        assert!(!v.sync(true).await.unwrap());

        // На диске — только шифротекст
        let disk = std::fs::read_to_string(dir.join("cache.json")).unwrap()
            + &std::fs::read_to_string(dir.join("account.json")).unwrap();
        for plain in ["P@ss-Секрет-42", "Сервер-тайный", "Офис-папка", "заметка-секрет", "admin", "10.9.8.7", "CORP"] {
            assert!(!disk.contains(plain), "на диске открытый текст: {}", plain);
        }

        v.lock().await;
        assert!(v.unlock("wrong password!!").await.is_err());
        assert!(v.unlock("correct horse battery").await.unwrap());
        assert_eq!(v.view().await.unwrap().items.len(), 1);

        assert!(v.change_local_password("wrong password!!", "new strong password").await.is_err());
        v.change_local_password("correct horse battery", "new strong password").await.unwrap();
        v.lock().await;
        assert!(v.unlock("correct horse battery").await.is_err());
        v.unlock("new strong password").await.unwrap();

        let backup = v.local_backup().await.unwrap();
        assert!(!backup.contains("P@ss-Секрет-42"));
        // «Сменить хранилище» — файлы уходят в папку копии, а не удаляются
        v.logout().await;
        assert!(!dir.join("cache.json").exists());
        assert!(std::fs::read_dir(&dir).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with("local-backup-")));

        v.local_restore(&backup).await.unwrap();
        v.unlock("new strong password").await.unwrap();
        let view = v.view().await.unwrap();
        assert_eq!(view.items[0].name, "Сервер-тайный");
        assert_eq!(v.secret(&view.items[0].id).await.unwrap().password, "P@ss-Секрет-42");
        v.delete_item(&view.items[0].id).await.unwrap();
        v.delete_folder(&f.id).await.unwrap();
        assert!(v.view().await.unwrap().items.is_empty());

        // Подмена файла хранилища вне программы обнаруживается
        v.save_folder(None, "ещё").await.unwrap();
        v.lock().await;
        let mut t = std::fs::read_to_string(dir.join("cache.json")).unwrap();
        t = t.replacen("\"folders\"", "\"folders\" ", 1);
        std::fs::write(dir.join("cache.json"), t).unwrap();
        assert!(v.unlock("new strong password").await.unwrap_err().contains("изменён"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
