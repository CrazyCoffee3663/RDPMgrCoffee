use serde::Serialize;
use serde_json::Value;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Manager, State};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

// tofu = запомнить сертификат при первом подключении и сообщать, если он изменится
const CERT_MODE: &str = "tofu";
const CREATE_NO_WINDOW: u32 = 0x08000000;

static BW_EXE: OnceLock<PathBuf> = OnceLock::new();
static BW_DATA: OnceLock<PathBuf> = OnceLock::new();
static FREERDP_EXE: OnceLock<PathBuf> = OnceLock::new();

fn bw_exe() -> PathBuf {
    BW_EXE.get().cloned().unwrap_or_else(|| PathBuf::from("bw.exe"))
}

fn freerdp_exe() -> PathBuf {
    FREERDP_EXE
        .get()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("wfreerdp.exe"))
}

// Убираем префикс \\?\, с которым некоторые программы работают плохо
fn clean(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy().to_string();
    PathBuf::from(s.trim_start_matches(r"\\?\"))
}

// Ищем файл среди ресурсов установленной программы,
// а в режиме разработки берём из папки проекта
fn locate(app: &AppHandle, rel: &str) -> PathBuf {
    if let Ok(dir) = app.path().resource_dir() {
        let p = dir.join("resources").join(rel);
        if p.exists() {
            return clean(p);
        }
    }
    clean(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources").join(rel))
}

#[derive(Default)]
struct Vault {
    session: Mutex<Option<String>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Folder {
    id: String,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Conn {
    id: String,
    name: String,
    folder_id: Option<String>,
    uri: String,
    username: String,
    protocol: String,
}

#[derive(Serialize)]
struct VaultData {
    folders: Vec<Folder>,
    items: Vec<Conn>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VaultStatus {
    status: String,
    server_url: String,
    email: String,
}

fn run_bw(args: &[&str], envs: &[(&str, &str)]) -> Result<String, String> {
    let exe = bw_exe();
    let mut cmd = Command::new(&exe);
    cmd.args(args)
        .env("BW_NOINTERACTION", "true")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Своя папка данных, независимая от других установок Bitwarden CLI
    if let Some(dir) = BW_DATA.get() {
        cmd.env("BITWARDENCLI_APPDATA_DIR", dir);
    }
    for (k, v) in envs {
        cmd.env(k, v);
    }
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let out = cmd
        .output()
        .map_err(|e| format!("Не удалось запустить bw.exe ({}): {}", exe.display(), e))?;

    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() {
            "bw вернул ошибку без описания".to_string()
        } else {
            err
        })
    }
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

// Значение пользовательского поля записи по имени
fn field(item: &Value, name: &str) -> String {
    item["fields"]
        .as_array()
        .and_then(|arr| {
            arr.iter().find(|f| {
                f["name"]
                    .as_str()
                    .map(|n| n.trim().eq_ignore_ascii_case(name))
                    .unwrap_or(false)
            })
        })
        .map(|f| text(&f["value"]).trim().to_string())
        .unwrap_or_default()
}

// "rdp://host:3389/path" -> "host:3389"
fn hostport(uri: &str) -> String {
    let rest = uri.split_once("://").map(|(_, r)| r).unwrap_or(uri);
    rest.split('/').next().unwrap_or("").trim().to_string()
}

// "домен\логин" -> (домен, логин); иначе домен берётся из запасного поля
fn split_user(raw: &str, fallback_domain: &str) -> (String, String) {
    match raw.trim().split_once('\\') {
        Some((d, u)) => (d.to_string(), u.to_string()),
        None => (fallback_domain.to_string(), raw.trim().to_string()),
    }
}

fn load_vault(session: &str) -> Result<VaultData, String> {
    let env = [("BW_SESSION", session)];
    run_bw(&["sync"], &env)?;
    let folders_raw = run_bw(&["list", "folders"], &env)?;
    let items_raw = run_bw(&["list", "items"], &env)?;

    let folders_json: Vec<Value> = serde_json::from_str(&folders_raw)
        .map_err(|e| format!("Ошибка разбора списка папок: {}", e))?;
    let items_json: Vec<Value> = serde_json::from_str(&items_raw)
        .map_err(|e| format!("Ошибка разбора списка записей: {}", e))?;

    let folders = folders_json
        .iter()
        .filter_map(|f| {
            f["id"].as_str().map(|id| Folder {
                id: id.to_string(),
                name: text(&f["name"]),
            })
        })
        .collect();

    let mut items = Vec::new();
    for it in &items_json {
        if it["type"].as_i64() != Some(1) {
            continue;
        }
        let uri = text(&it["login"]["uris"][0]["uri"]);
        let protocol = match uri.split_once("://") {
            Some((scheme, _)) => scheme.to_lowercase(),
            None => String::new(),
        };
        items.push(Conn {
            id: text(&it["id"]),
            name: text(&it["name"]),
            folder_id: it["folderId"].as_str().map(|s| s.to_string()),
            uri,
            username: text(&it["login"]["username"]),
            protocol,
        });
    }

    Ok(VaultData { folders, items })
}

fn find_gateway(session: &str, gw_name: &str) -> Result<Value, String> {
    let env = [("BW_SESSION", session)];
    let raw = run_bw(&["list", "items", "--search", gw_name], &env)?;
    let items: Vec<Value> = serde_json::from_str(&raw)
        .map_err(|e| format!("Ошибка разбора записи шлюза: {}", e))?;
    let found: Vec<&Value> = items
        .iter()
        .filter(|i| i["type"].as_i64() == Some(1) && text(&i["name"]) == gw_name)
        .collect();
    match found.len() {
        0 => Err(format!("Запись шлюза «{}» не найдена в Bitwarden", gw_name)),
        1 => Ok(found[0].clone()),
        _ => Err(format!(
            "В Bitwarden несколько записей с названием «{}», оставьте одну",
            gw_name
        )),
    }
}

fn launch(session: &str, id: &str) -> Result<(), String> {
    let env = [("BW_SESSION", session)];
    let raw = run_bw(&["get", "item", id], &env)?;
    let item: Value =
        serde_json::from_str(&raw).map_err(|e| format!("Ошибка разбора записи: {}", e))?;

    let name = text(&item["name"]);
    let uri = text(&item["login"]["uris"][0]["uri"]);
    let scheme = uri
        .split_once("://")
        .map(|(s, _)| s.to_lowercase())
        .ok_or("У записи нет адреса вида rdp://хост")?;
    if scheme != "rdp" {
        return Err(format!(
            "Протокол {} пока не поддерживается, сейчас доступен только rdp",
            scheme
        ));
    }

    let host = hostport(&uri);
    let (domain, user) = split_user(&text(&item["login"]["username"]), &field(&item, "domain"));
    let password = text(&item["login"]["password"]);

    let mut args: Vec<String> = vec![
        format!("/v:{}", host),
        format!("/title:{}", name),
        format!("/cert:{}", CERT_MODE),
        "/size:1600x900".to_string(),
        "/dynamic-resolution".to_string(),
        "/clipboard:direction-to:all,files-to:all".to_string(),
    ];
    if !user.is_empty() {
        args.push(format!("/u:{}", user));
    }
    if !domain.is_empty() {
        args.push(format!("/d:{}", domain));
    }
    if !password.is_empty() {
        args.push(format!("/p:{}", password));
    }

    let gw_name = field(&item, "gateway");
    if !gw_name.is_empty() {
        let gw = find_gateway(session, &gw_name)?;
        let gw_host = hostport(&text(&gw["login"]["uris"][0]["uri"]));
        if gw_host.is_empty() {
            return Err(format!("У записи шлюза «{}» не указан адрес (URI)", gw_name));
        }
        let (gd, gu) = split_user(&text(&gw["login"]["username"]), &field(&gw, "domain"));
        let gp = text(&gw["login"]["password"]);

        // Запятая ломает разбор параметров шлюза
        for (label, v) in [("адрес", &gw_host), ("логин", &gu), ("домен", &gd), ("пароль", &gp)] {
            if v.contains(',') {
                return Err(format!(
                    "{} шлюза содержит запятую, такое пока не поддерживается",
                    label
                ));
            }
        }

        let mut g = format!("/gateway:g:{}", gw_host);
        if !gu.is_empty() {
            g.push_str(&format!(",u:{}", gu));
        }
        if !gd.is_empty() {
            g.push_str(&format!(",d:{}", gd));
        }
        if !gp.is_empty() {
            g.push_str(&format!(",p:{}", gp));
        }
        args.push(g);
    }

    // Лог FreeRDP (без паролей) для разбора проблем
    let log_path = std::env::temp_dir().join("connmgr-freerdp.log");
    let log = File::create(&log_path).map_err(|e| format!("Не удалось создать лог: {}", e))?;
    let log2 = log
        .try_clone()
        .map_err(|e| format!("Не удалось создать лог: {}", e))?;

    let exe = freerdp_exe();
    let mut cmd = Command::new(&exe);
    cmd.arg("/args-from:stdin")
        .stdin(Stdio::piped())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log2));
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Не удалось запустить FreeRDP ({}): {}", exe.display(), e))?;

    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or("Не удалось открыть ввод FreeRDP")?;
        stdin
            .write_all(args.join("\n").as_bytes())
            .map_err(|e| format!("Не удалось передать параметры FreeRDP: {}", e))?;
    } // здесь ввод закрывается

    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[tauri::command]
async fn vault_status() -> Result<VaultStatus, String> {
    tauri::async_runtime::spawn_blocking(|| -> Result<VaultStatus, String> {
        let raw = run_bw(&["status"], &[])?;
        let v: Value = serde_json::from_str(&raw)
            .map_err(|e| format!("Ошибка разбора статуса: {}", e))?;
        Ok(VaultStatus {
            status: text(&v["status"]),
            server_url: text(&v["serverUrl"]),
            email: text(&v["userEmail"]),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn vault_setup(
    state: State<'_, Vault>,
    server: String,
    email: String,
    password: String,
    code: String,
) -> Result<VaultData, String> {
    let (session, data) =
        tauri::async_runtime::spawn_blocking(move || -> Result<(String, VaultData), String> {
            let server = server.trim().trim_end_matches('/').to_string();
            let email = email.trim().to_string();
            let code = code.trim().to_string();
            if !server.starts_with("https://") && !server.starts_with("http://") {
                return Err("Адрес сервера должен начинаться с https://".to_string());
            }
            if email.is_empty() {
                return Err("Укажите почту".to_string());
            }

            // Чистое состояние: выходим, если вход уже был (ошибку игнорируем)
            let _ = run_bw(&["logout"], &[]);
            run_bw(&["config", "server", server.as_str()], &[])?;

            let mut args: Vec<&str> = vec![
                "login",
                email.as_str(),
                "--passwordenv",
                "BW_PASSWORD",
                "--raw",
            ];
            if !code.is_empty() {
                // метод 0 = приложение-аутентификатор
                args.extend(["--method", "0", "--code", code.as_str()]);
            }
            let session = run_bw(&args, &[("BW_PASSWORD", password.as_str())])?;
            let data = load_vault(&session)?;
            Ok((session, data))
        })
        .await
        .map_err(|e| e.to_string())??;

    *state.session.lock().unwrap() = Some(session);
    Ok(data)
}

#[tauri::command]
async fn vault_reset(state: State<'_, Vault>) -> Result<(), String> {
    *state.session.lock().unwrap() = None;
    tauri::async_runtime::spawn_blocking(|| {
        let _ = run_bw(&["logout"], &[]);
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn vault_unlock(state: State<'_, Vault>, password: String) -> Result<VaultData, String> {
    let (session, data) =
        tauri::async_runtime::spawn_blocking(move || -> Result<(String, VaultData), String> {
            let session = run_bw(
                &["unlock", "--raw", "--passwordenv", "BW_PASSWORD"],
                &[("BW_PASSWORD", password.as_str())],
            )?;
            let data = load_vault(&session)?;
            Ok((session, data))
        })
        .await
        .map_err(|e| e.to_string())??;

    *state.session.lock().unwrap() = Some(session);
    Ok(data)
}

#[tauri::command]
async fn vault_lock(state: State<'_, Vault>) -> Result<(), String> {
    *state.session.lock().unwrap() = None;
    Ok(())
}

#[tauri::command]
async fn connect(state: State<'_, Vault>, id: String) -> Result<(), String> {
    let session = {
        let guard = state.session.lock().unwrap();
        guard.clone()
    };
    let session = session.ok_or_else(|| "Хранилище заблокировано".to_string())?;
    tauri::async_runtime::spawn_blocking(move || launch(&session, &id))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Vault::default())
        .setup(|app| {
            let handle = app.handle();
            let _ = BW_EXE.set(locate(handle, r"bw\bw.exe"));
            let _ = FREERDP_EXE.set(locate(handle, r"freerdp\wfreerdp.exe"));
            if let Ok(dir) = handle.path().app_data_dir() {
                let dir = clean(dir.join("bw-data"));
                let _ = std::fs::create_dir_all(&dir);
                let _ = BW_DATA.set(dir);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            vault_status,
            vault_setup,
            vault_reset,
            vault_unlock,
            vault_lock,
            connect
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}