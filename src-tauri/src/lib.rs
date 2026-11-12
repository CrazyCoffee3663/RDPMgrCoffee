mod bw;
mod clip;
mod conn;
mod mremoteng;
mod net;
mod tools;
mod rdp;
mod sessions;

use bw::{FolderV, ItemInput, ItemV, LoginResult, Secret, Status, Vault, VaultView};
use serde::Serialize;
use sessions::{Sessions, TermCmd};
use std::path::PathBuf;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Manager, State};

type CmdResult<T> = Result<T, String>;

// ---------- Хранилище ----------

#[tauri::command]
async fn vault_status(vault: State<'_, Vault>) -> CmdResult<Status> {
    Ok(vault.status().await)
}

#[tauri::command]
async fn vault_login(vault: State<'_, Vault>, server: String, email: String, password: String) -> CmdResult<LoginResult> {
    vault.login_start(&server, &email, &password).await
}

#[tauri::command]
async fn vault_login_2fa(vault: State<'_, Vault>, provider: u8, code: String) -> CmdResult<LoginResult> {
    vault.login_two_factor(provider, &code).await
}

#[tauri::command]
async fn vault_login_new_device(vault: State<'_, Vault>, code: String) -> CmdResult<LoginResult> {
    vault.login_new_device(&code).await
}

#[tauri::command]
async fn vault_send_email_code(vault: State<'_, Vault>) -> CmdResult<()> {
    vault.send_email_code().await
}

#[tauri::command]
async fn vault_unlock(vault: State<'_, Vault>, password: String) -> CmdResult<bool> {
    vault.unlock(&password).await
}

#[tauri::command]
async fn vault_lock(app: AppHandle, vault: State<'_, Vault>) -> CmdResult<()> {
    vault.lock().await;
    // Сессии продолжают работать: пароли им уже переданы.
    let _ = app;
    Ok(())
}

#[tauri::command]
async fn vault_logout(vault: State<'_, Vault>) -> CmdResult<()> {
    vault.logout().await;
    Ok(())
}

#[tauri::command]
async fn vault_sync(vault: State<'_, Vault>, force: bool) -> CmdResult<bool> {
    vault.sync(force).await
}

#[tauri::command]
async fn vault_view(vault: State<'_, Vault>) -> CmdResult<VaultView> {
    vault.view().await
}

#[tauri::command]
async fn item_secret(vault: State<'_, Vault>, id: String) -> CmdResult<Secret> {
    vault.secret(&id).await
}

#[tauri::command]
async fn item_save(vault: State<'_, Vault>, item: ItemInput) -> CmdResult<ItemV> {
    vault.save_item(item).await
}

#[tauri::command]
async fn item_move(vault: State<'_, Vault>, id: String, folder_id: Option<String>, to_root: Option<bool>, favorite: Option<bool>) -> CmdResult<ItemV> {
    // to_root = переместить в корень; folder_id = None без to_root — папку не менять
    let folder = if to_root.unwrap_or(false) { Some(String::new()) } else { folder_id };
    vault.move_item(&id, folder, favorite).await
}

#[tauri::command]
async fn item_delete(vault: State<'_, Vault>, id: String) -> CmdResult<()> {
    vault.delete_item(&id).await
}

#[tauri::command]
async fn folder_save(vault: State<'_, Vault>, id: Option<String>, name: String) -> CmdResult<FolderV> {
    vault.save_folder(id, &name).await
}

#[tauri::command]
async fn folder_delete(vault: State<'_, Vault>, id: String) -> CmdResult<()> {
    vault.delete_folder(&id).await
}

#[tauri::command]
fn generate_password(length: usize, upper: bool, lower: bool, digits: bool, symbols: bool) -> String {
    bw::crypto::generate_password(length, upper, lower, digits, symbols)
}

/// Скопировать секрет записи в буфер обмена, не передавая его в интерфейс.
#[tauri::command]
async fn item_copy(vault: State<'_, Vault>, id: String, what: String, clear_after: Option<u64>) -> CmdResult<()> {
    let sec = vault.secret(&id).await?;
    let value = match what.as_str() {
        "password" => sec.password,
        "sshkey" => sec.ssh_private_key,
        other => sec
            .fields
            .into_iter()
            .find(|f| f.name == other)
            .and_then(|f| f.value)
            .ok_or("Поле не найдено")?,
    };
    if value.is_empty() {
        return Err("Значение пустое".into());
    }
    clip::set(&value, clear_after)
}

#[tauri::command]
fn clipboard_set(text: String, clear_after: Option<u64>) -> CmdResult<()> {
    clip::set(&text, clear_after)
}

#[tauri::command]
fn clipboard_get() -> CmdResult<String> {
    clip::get()
}

// ---------- Сессии ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Opened {
    id: u32,
    /// term | rdp
    kind: String,
    protocol: String,
    title: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenArgs {
    item_id: Option<String>,
    quick: Option<String>,
    quick_protocol: Option<String>,
    quick_username: Option<String>,
    quick_password: Option<String>,
    cols: u32,
    rows: u32,
    /// Область вкладки в физических пикселях (для RDP)
    rect: Option<[i32; 4]>,
}

#[tauri::command]
async fn session_open(
    app: AppHandle,
    vault: State<'_, Vault>,
    sessions: State<'_, Sessions>,
    args: OpenArgs,
    on_data: Channel<InvokeResponseBody>,
) -> CmdResult<Opened> {
    let mut p = match (&args.item_id, &args.quick) {
        (Some(id), _) => conn::from_vault(&vault, id).await?,
        (None, Some(q)) => conn::from_quick(q, args.quick_protocol.as_deref().unwrap_or("ssh"))?,
        _ => return Err("Не указано, куда подключаться".into()),
    };
    if let Some(u) = args.quick_username.filter(|u| !u.is_empty()) {
        let (d, u) = conn::split_user(&u);
        p.username = u;
        if !d.is_empty() {
            p.domain = d;
        }
    }
    if let Some(pw) = args.quick_password.filter(|p| !p.is_empty()) {
        p.password = pw;
    }
    let id = sessions.alloc_id();
    let title = p.name.clone();
    let protocol = p.protocol.clone();
    if p.protocol == "rdp" {
        let [x, y, w, h] = args.rect.unwrap_or([0, 0, 1024, 768]);
        rdp::open(&app, id, p, x, y, w, h).await?;
        Ok(Opened { id, kind: "rdp".into(), protocol, title })
    } else {
        sessions::start_term(app.clone(), id, p, args.cols.max(10), args.rows.max(5), on_data);
        Ok(Opened { id, kind: "term".into(), protocol, title })
    }
}

#[tauri::command]
fn session_input(sessions: State<'_, Sessions>, id: u32, data: String) {
    sessions.send(id, TermCmd::Data(data.into_bytes()));
}

#[tauri::command]
fn session_resize(sessions: State<'_, Sessions>, id: u32, cols: u32, rows: u32) {
    sessions.send(id, TermCmd::Resize(cols, rows));
}

#[tauri::command]
fn session_close(app: AppHandle, sessions: State<'_, Sessions>, id: u32) {
    if !rdp::close(&app, id) {
        sessions.send(id, TermCmd::Close);
    }
}

#[tauri::command]
fn rdp_place(app: AppHandle, id: u32, x: i32, y: i32, w: i32, h: i32, visible: bool) {
    rdp::place(&app, id, x, y, w, h, visible);
}

#[tauri::command]
fn rdp_detach(app: AppHandle, id: u32, title: String) -> Result<(), String> {
    rdp::detach(&app, id, title)
}

#[tauri::command]
fn rdp_attach(app: AppHandle, id: u32) {
    rdp::attach(&app, id);
}

#[tauri::command]
fn prompt_answer(sessions: State<'_, Sessions>, id: u64, value: Option<String>) {
    sessions.answer(id, value);
}

#[tauri::command]
fn serial_ports() -> Vec<String> {
    sessions::serial_ports()
}

// ---------- Запуск ----------

fn clean(p: PathBuf) -> PathBuf {
    PathBuf::from(p.to_string_lossy().trim_start_matches(r"\\?\").to_string())
}

/// Файл среди ресурсов установленной программы; в режиме разработки — из папки проекта.
fn locate(app: &AppHandle, rel: &str) -> PathBuf {
    if let Ok(dir) = app.path().resource_dir() {
        let p = dir.join("resources").join(rel);
        if p.exists() {
            return clean(p);
        }
    }
    clean(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources").join(rel))
}

fn cleanup_stale_files() {
    if let Ok(rd) = std::fs::read_dir(std::env::temp_dir()) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with("connmgr-") && (n.ends_with(".pw") || n.ends_with(".log")) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}


// ---------- SFTP ----------

#[tauri::command]
async fn sftp_list(sessions: State<'_, Sessions>, id: u32, path: String) -> Result<sessions::SftpListing, String> {
    sessions::sftp_list(&sessions, id, &path).await
}

/// Скачать файл в папку «Загрузки». Возвращает локальный путь.
#[tauri::command]
async fn sftp_download(app: AppHandle, sessions: State<'_, Sessions>, id: u32, path: String) -> Result<String, String> {
    let dir = app.path().download_dir().map_err(|e| e.to_string())?;
    let p = sessions::sftp_download(&sessions, id, &path, &dir).await?;
    Ok(p.display().to_string())
}

/// Загрузка файла: тело запроса — содержимое, заголовки x-sid и x-path (urlencoded).
#[tauri::command]
async fn sftp_upload(sessions: State<'_, Sessions>, request: tauri::ipc::Request<'_>) -> Result<(), String> {
    let tauri::ipc::InvokeBody::Raw(data) = request.body() else {
        return Err("Ожидалось содержимое файла".into());
    };
    let h = |k: &str| request.headers().get(k).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let id: u32 = h("x-sid").parse().map_err(|_| "Нет id сеанса")?;
    let path = urlencoding_decode(&h("x-path"));
    if path.is_empty() {
        return Err("Не указан путь".into());
    }
    sessions::sftp_upload(&sessions, id, &path, data).await
}

fn urlencoding_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("");
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[tauri::command]
async fn sftp_op(sessions: State<'_, Sessions>, id: u32, op: String, path: String, to: Option<String>) -> Result<(), String> {
    sessions::sftp_op(&sessions, id, &op, &path, to.as_deref().unwrap_or("")).await
}

/// Папка внешних инструментов (WinBox) и HTTP-клиент для их загрузки
pub struct Tools {
    dir: PathBuf,
    http: reqwest::Client,
    /// Запущенные во всплывающих окнах программы: метка окна → сигнал остановки
    runs: std::sync::Mutex<std::collections::HashMap<String, tokio::sync::oneshot::Sender<()>>>,
}

/// Запустить внешний инструмент для подключения (подстановки — в tools.rs).
#[tauri::command]
async fn tool_run(
    vault: State<'_, Vault>,
    tools: State<'_, Tools>,
    item_id: String,
    command: String,
    args: String,
    pw_clip: bool,
) -> Result<Option<String>, String> {
    let p = conn::from_vault(&vault, &item_id).await?;
    tools::run(&tools.dir, &command, &args, &p, pw_clip)
}

/// Выполнить инструмент с выводом в окно приложения (вызывается из самого окна).
#[tauri::command]
async fn tool_exec(
    window: tauri::Window,
    vault: State<'_, Vault>,
    tools: State<'_, Tools>,
    item_id: String,
    command: String,
    args: String,
    pw_clip: bool,
    on_data: tauri::ipc::Channel<String>,
) -> Result<Option<i32>, String> {
    let p = conn::from_vault(&vault, &item_id).await?;
    let (args, clip) = tools::prepare(&args, &p, pw_clip);
    let exe = tools::resolve(&tools.dir, &command, &p)?;
    if clip {
        clip::set(&p.password, Some(tools::PW_CLIP_SECS))?;
    }
    let label = window.label().to_string();
    let (tx, rx) = tokio::sync::oneshot::channel();
    // Прежний запуск в этом окне (перезапуск) останавливается: его отправитель закрывается
    tools.runs.lock().unwrap().insert(label.clone(), tx);
    let r = tools::exec(&exe, &args, |s| { let _ = on_data.send(s); }, rx).await;
    let mut runs = tools.runs.lock().unwrap();
    if runs.get(&label).is_some_and(|t| t.is_closed()) {
        runs.remove(&label);
    }
    r
}

#[tauri::command]
fn tool_stop(window: tauri::Window, tools: State<'_, Tools>) {
    if let Some(tx) = tools.runs.lock().unwrap().remove(window.label()) {
        let _ = tx.send(());
    }
}

#[tauri::command]
async fn winbox_status(tools: State<'_, Tools>) -> Result<tools::WinboxStatus, String> {
    Ok(tools::status(&tools.http, &tools.dir).await)
}

/// Установить/обновить WinBox (только после подтверждения пользователя в интерфейсе).
#[tauri::command]
async fn winbox_install(tools: State<'_, Tools>, version: String) -> Result<(), String> {
    std::fs::create_dir_all(&tools.dir).map_err(|e| e.to_string())?;
    tools::install(&tools.http, &tools.dir, &version).await
}

/// Проверка доступности хостов (индикатор в дереве).
#[tauri::command]
async fn probe_hosts(targets: Vec<net::Target>, timeout_ms: u64, parallel: usize) -> Vec<net::Reach> {
    net::probe(targets, timeout_ms, parallel).await
}

/// Файл подключений mRemoteNG по умолчанию (%APPDATA%\mRemoteNG\confCons.xml), если он есть.
#[tauri::command]
fn mremoteng_default_file() -> Option<String> {
    let p = std::path::PathBuf::from(std::env::var_os("APPDATA")?).join("mRemoteNG").join("confCons.xml");
    p.is_file().then(|| p.to_string_lossy().into_owned())
}

/// Прочитать и разобрать файл mRemoteNG по пути.
#[tauri::command]
fn import_mremoteng_file(path: String, password: Option<String>) -> Result<mremoteng::Import, String> {
    let xml = std::fs::read_to_string(&path).map_err(|e| format!("Не удалось прочитать {}: {}", path, e))?;
    mremoteng::parse(&xml, password.as_deref())
}

/// Разобрать confCons.xml mRemoteNG (записи создаёт интерфейс).
#[tauri::command]
fn import_mremoteng(xml: String, password: Option<String>) -> Result<mremoteng::Import, String> {
    mremoteng::parse(&xml, password.as_deref())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Sessions::default())
        .manage(rdp::Rdp::default())
        .setup(|app| {
            let handle = app.handle();
            let _ = rdp::FREERDP_EXE.set(locate(handle, r"freerdp\wfreerdp.exe"));
            let dir = clean(
                handle
                    .path()
                    .app_data_dir()
                    .unwrap_or_else(|_| std::env::temp_dir().join("connmgr")),
            );
            let _ = std::fs::create_dir_all(&dir);
            *app.state::<Sessions>().known_hosts.lock().unwrap() = dir.join("known_hosts");
            app.manage(Vault::new(dir.join("vault")));
            app.manage(Tools {
                dir: dir.join("tools"),
                http: reqwest::Client::builder()
                    .user_agent("RDPMgrCoffee")
                    .timeout(std::time::Duration::from_secs(120))
                    .build()
                    .unwrap_or_default(),
                runs: Default::default(),
            });
            cleanup_stale_files();
            Ok(())
        })
        .on_window_event(|w, e| match e {
            // Окна инструментов: закрытие останавливает запущенную программу
            tauri::WindowEvent::Destroyed if w.label() != "main" => {
                w.app_handle().state::<Tools>().runs.lock().unwrap().remove(w.label());
            }
            tauri::WindowEvent::Destroyed => {
                let app = w.app_handle();
                app.state::<Sessions>().close_all();
                rdp::close_all(app);
                for (label, win) in app.webview_windows() {
                    if label != "main" {
                        let _ = win.destroy();
                    }
                }
            }
            tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) if w.label() == "main" => {
                rdp::reposition_all(w.app_handle())
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            vault_status,
            vault_login,
            vault_login_2fa,
            vault_login_new_device,
            vault_send_email_code,
            vault_unlock,
            vault_lock,
            vault_logout,
            vault_sync,
            vault_view,
            item_secret,
            item_save,
            item_move,
            item_delete,
            folder_save,
            folder_delete,
            generate_password,
            item_copy,
            clipboard_set,
            clipboard_get,
            session_open,
            session_input,
            session_resize,
            session_close,
            rdp_place,
            rdp_detach,
            rdp_attach,
            prompt_answer,
            serial_ports,
            import_mremoteng,
            mremoteng_default_file,
            probe_hosts,
            tool_run,
            tool_exec,
            tool_stop,
            sftp_list,
            sftp_download,
            sftp_upload,
            sftp_op,
            winbox_status,
            winbox_install,
            import_mremoteng_file
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    #[test]
    fn urldecode() {
        assert_eq!(super::urlencoding_decode("%2Fhome%2F%D1%84%20x"), "/home/ф x");
        assert_eq!(super::urlencoding_decode("a%2"), "a%2");
    }
}
