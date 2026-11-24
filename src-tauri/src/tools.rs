//! Внешние инструменты (ping, tracert, WinBox, браузер…) и установка/обновление WinBox.
//!
//! Подстановки в аргументах: %HOST% %PORT% %ADDR% %USER% %DOMAIN% %NAME% %PROTOCOL%.
//! %PASSWORD% в командную строку не подставляется никогда (её видят другие программы
//! и журналы аудита Windows): аргумент с ним отбрасывается, а пароль кладётся в буфер обмена.
//! Логин и пароль берутся из хранилища здесь, в Rust, — интерфейс их не видит.

use crate::conn::ConnParams;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

const WINBOX_LATEST: &str = "https://upgrade.mikrotik.com/routeros/winbox/LATEST.4";

/// Разбить строку аргументов: пробелы — разделители, "в кавычках" — один аргумент.
pub fn split_args(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    let mut has = false;
    for ch in s.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                has = true;
            }
            c if c.is_whitespace() && !quoted => {
                if has {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => {
                cur.push(c);
                has = true;
            }
        }
    }
    if has {
        out.push(cur);
    }
    out
}

/// Подставить параметры подключения в аргумент.
pub fn expand(arg: &str, p: &ConnParams) -> String {
    let port = p.port.map(|x| x.to_string()).unwrap_or_else(|| {
        match p.protocol.as_str() {
            "rdp" => "3389",
            "ssh" => "22",
            "telnet" => "23",
            "winbox" => "8291",
            _ => "",
        }
        .to_string()
    });
    // %ADDR% — хост, а порт только если он задан явно (для WinBox: «10.0.0.1:8292»)
    let host = if p.host.contains(':') && !p.host.starts_with('[') { format!("[{}]", p.host) } else { p.host.clone() };
    let addr = match p.port {
        Some(port) => format!("{}:{}", host, port),
        None => p.host.clone(),
    };
    arg.replace("%ADDR%", &addr)
        .replace("%HOST%", &p.host)
        .replace("%PORT%", &port)
        .replace("%USER%", &p.username)
        .replace("%DOMAIN%", &p.domain)
        .replace("%NAME%", &p.name)
        .replace("%PROTOCOL%", &p.protocol)
}

pub fn winbox_dir(tools: &Path) -> PathBuf {
    tools.join("winbox")
}

fn winbox_exe(tools: &Path) -> Option<PathBuf> {
    let dir = winbox_dir(tools);
    ["WinBox.exe", "winbox64.exe", "winbox.exe"]
        .iter()
        .map(|n| dir.join(n))
        .find(|p| p.is_file())
}

/// Разобрать и подставить аргументы. Аргументы с %PASSWORD% не передаются —
/// возвращается true, если пароль надо положить в буфер обмена.
pub fn prepare(args: &str, p: &ConnParams) -> (Vec<String>, bool) {
    let mut dropped = false;
    let out = split_args(args)
        .into_iter()
        .filter(|a| {
            let hide = a.to_ascii_uppercase().contains("%PASSWORD%");
            dropped |= hide;
            !hide
        })
        .map(|a| expand(&a, p))
        .collect();
    (out, dropped && !p.password.is_empty())
}

/// Путь к программе инструмента
pub fn resolve(tools: &Path, command: &str, p: &ConnParams) -> Result<PathBuf, String> {
    if command.eq_ignore_ascii_case("winbox") {
        let exe = winbox_exe(tools).ok_or_else(|| "WINBOX_MISSING".to_string())?;
        verify_winbox(&exe)?;
        Ok(exe)
    } else {
        find_exe(&expand(command, p))
    }
}

/// Найти программу по имени. Windows при запуске по одному имени сначала ищет в текущей
/// папке — подложенный туда ping.exe запустился бы вместо системного. Поэтому ищем сами:
/// System32, папка Windows, затем только абсолютные пути из PATH.
pub fn find_exe(cmd: &str) -> Result<PathBuf, String> {
    let cmd = cmd.trim().trim_matches('"');
    if cmd.is_empty() {
        return Err("Не указана программа".into());
    }
    let p = Path::new(cmd);
    if p.is_absolute() {
        return Ok(p.to_path_buf());
    }
    if cmd.contains(['\\', '/']) {
        return Err(format!("Укажите полный путь к программе: «{}»", cmd));
    }
    let name = if p.extension().is_some() { cmd.to_string() } else { format!("{}.exe", cmd) };
    let mut dirs: Vec<PathBuf> = vec![];
    if let Some(root) = std::env::var_os("SystemRoot") {
        let root = PathBuf::from(root);
        dirs.push(root.join("System32"));
        dirs.push(root.clone());
        dirs.push(root.join("System32").join("WindowsPowerShell").join("v1.0"));
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path).filter(|d| d.is_absolute()));
    }
    dirs.into_iter()
        .map(|d| d.join(&name))
        .find(|f| f.is_file())
        .ok_or_else(|| format!("Программа «{}» не найдена", cmd))
}

/// WinBox запускается, только если он подписан MikroTik (SIA «Mikrotīkls») и подпись верна.
#[cfg(windows)]
pub fn verify_winbox(exe: &Path) -> Result<(), String> {
    let signer = authenticode_signer(exe)
        .map_err(|e| format!("WinBox не прошёл проверку подписи ({}): {}. Переустановите его в настройках.", exe.display(), e))?;
    if !signer.to_lowercase().contains("mikrot") {
        return Err(format!("WinBox подписан не MikroTik, а «{}» — запуск отменён", signer));
    }
    Ok(())
}
#[cfg(not(windows))]
pub fn verify_winbox(_: &Path) -> Result<(), String> {
    Ok(())
}

/// Проверить цифровую подпись файла (Authenticode) и вернуть имя подписавшего.
#[cfg(windows)]
pub fn authenticode_signer(file: &Path) -> Result<String, String> {
    use windows::core::{GUID, PCWSTR, PWSTR};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Security::Cryptography::{CertGetNameStringW, CERT_NAME_SIMPLE_DISPLAY_TYPE};
    use windows::Win32::Security::WinTrust::*;
    let path: Vec<u16> = file.as_os_str().encode_wide_z();
    let mut fi = WINTRUST_FILE_INFO {
        cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: PCWSTR(path.as_ptr()),
        ..Default::default()
    };
    let mut wd = WINTRUST_DATA {
        cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 { pFile: &mut fi },
        dwStateAction: WTD_STATEACTION_VERIFY,
        ..Default::default()
    };
    let mut action: GUID = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    let st = unsafe { WinVerifyTrust(HWND(std::ptr::null_mut()), &mut action, &mut wd as *mut _ as *mut _) };
    let result = (|| {
        if st != 0 {
            return Err(format!("подпись недействительна (код 0x{:08X})", st as u32));
        }
        unsafe {
            let prov = WTHelperProvDataFromStateData(wd.hWVTStateData);
            let signer = WTHelperGetProvSignerFromChain(prov, 0, false, 0);
            if signer.is_null() || (*signer).pasCertChain.is_null() {
                return Err("нет данных о подписавшем".to_string());
            }
            let cert = (*(*signer).pasCertChain).pCert;
            let mut buf = [0u16; 256];
            let n = CertGetNameStringW(cert, CERT_NAME_SIMPLE_DISPLAY_TYPE, 0, None, Some(&mut buf));
            Ok(String::from_utf16_lossy(&buf[..(n as usize).saturating_sub(1)]))
        }
    })();
    wd.dwStateAction = WTD_STATEACTION_CLOSE;
    unsafe {
        let _ = WinVerifyTrust(HWND(std::ptr::null_mut()), &mut action, &mut wd as *mut _ as *mut _);
    }
    let _ = PWSTR::null();
    result
}

trait WideZ {
    fn encode_wide_z(&self) -> Vec<u16>;
}
impl WideZ for std::ffi::OsStr {
    fn encode_wide_z(&self) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        self.encode_wide().chain(std::iter::once(0)).collect()
    }
}

pub const PW_CLIP_SECS: u64 = 45;

/// Запустить инструмент. command: путь к программе, "winbox" (встроенный) или "url" (открыть ссылку).
/// Возвращает подсказку для пользователя, если пароль положен в буфер обмена.
pub fn run(tools: &Path, command: &str, args: &str, p: &ConnParams) -> Result<Option<String>, String> {
    let (args, clip) = prepare(args, p);
    if command.eq_ignore_ascii_case("url") {
        let url = args.first().ok_or("Не указан адрес")?;
        open_url(url)?;
        return Ok(None);
    }
    let exe = resolve(tools, command, p)?;
    spawn_console(&exe, &args).map_err(|e| format!("Не удалось запустить {}: {}", exe.display(), e))?;
    if clip {
        crate::clip::set(&p.password, Some(PW_CLIP_SECS), true)?;
        return Ok(Some(format!(
            "Пароль скопирован — вставьте его (Ctrl+V). Буфер очистится через {} с",
            PW_CLIP_SECS
        )));
    }
    Ok(None)
}

// ---------- Запуск с выводом в окно приложения ----------

/// Вывод консольных программ Windows — в OEM-кодировке (cp866 у русской Windows).
#[derive(Default)]
pub struct OemDecoder {
    pending: Vec<u8>,
}

impl OemDecoder {
    pub fn push(&mut self, b: &[u8]) -> String {
        self.pending.extend_from_slice(b);
        // Для UTF-8 не режем многобайтовый символ на границе чтения
        let cut = if oem_cp() == 65001 {
            match std::str::from_utf8(&self.pending) {
                Ok(_) => self.pending.len(),
                Err(e) if e.error_len().is_none() => e.valid_up_to(),
                Err(_) => self.pending.len(),
            }
        } else {
            self.pending.len()
        };
        let chunk: Vec<u8> = self.pending.drain(..cut).collect();
        decode_oem(&chunk)
    }
    pub fn finish(&mut self) -> String {
        let rest = std::mem::take(&mut self.pending);
        decode_oem(&rest)
    }
}

#[cfg(windows)]
fn oem_cp() -> u32 {
    unsafe { windows::Win32::Globalization::GetOEMCP() }
}
#[cfg(not(windows))]
fn oem_cp() -> u32 {
    65001
}

#[cfg(windows)]
fn decode_oem(b: &[u8]) -> String {
    use windows::Win32::Globalization::{MultiByteToWideChar, CP_OEMCP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS};
    if b.is_empty() {
        return String::new();
    }
    let mut buf = vec![0u16; b.len() * 2 + 4];
    let n = unsafe { MultiByteToWideChar(CP_OEMCP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), b, Some(&mut buf)) };
    if n <= 0 {
        return String::from_utf8_lossy(b).into_owned();
    }
    String::from_utf16_lossy(&buf[..n as usize])
}
#[cfg(not(windows))]
fn decode_oem(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Job Object: закрытие дескриптора завершает все процессы задания (всё дерево).
/// kill_on_drop/kill убивает только прямого потомка — cmd /k ping -t оставлял ping сиротой.
#[cfg(windows)]
struct Job(windows::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Job {
    fn assign(child: &tokio::process::Child) -> Result<Job, String> {
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        use windows::Win32::System::JobObjects::*;
        unsafe {
            let h = CreateJobObjectW(None, None).map_err(|e| e.to_string())?;
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let r = SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if let Err(e) = r {
                let _ = CloseHandle(h);
                return Err(e.to_string());
            }
            let proc = child.raw_handle().ok_or("Нет дескриптора процесса")?;
            if let Err(e) = AssignProcessToJobObject(h, HANDLE(proc)) {
                let _ = CloseHandle(h);
                return Err(e.to_string());
            }
            Ok(Job(h))
        }
    }
}

// Дескриптор задания — общепроцессный; CloseHandle потокобезопасен, задание живёт
// в одном экземпляре exec и закрывается при его завершении на любом потоке
#[cfg(windows)]
unsafe impl Send for Job {}

#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

/// Выполнить программу без окна консоли, отдавая вывод в `out`. Остановка — через `stop`
/// (сигнал или закрытие отправителя). Возвращает код выхода (None — остановлено).
pub async fn exec<F: Fn(String)>(
    exe: &Path,
    args: &[String],
    out: F,
    mut stop: tokio::sync::oneshot::Receiver<()>,
) -> Result<Option<i32>, String> {
    use std::process::Stdio;
    use tokio::io::AsyncReadExt;
    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let mut child = cmd.spawn().map_err(|e| format!("Не удалось запустить {}: {}", exe.display(), e))?;
    // Отказ изоляции — отказ запуска: иначе «Стоп» оставлял бы дочерние процессы
    #[cfg(windows)]
    let _job = match Job::assign(&child) {
        Ok(j) => j,
        Err(e) => {
            let _ = child.kill().await;
            return Err(format!("Не удалось изолировать процесс инструмента: {}", e));
        }
    };
    let mut so = child.stdout.take().ok_or("нет stdout")?;
    let mut se = child.stderr.take().ok_or("нет stderr")?;
    let (mut d1, mut d2) = (OemDecoder::default(), OemDecoder::default());
    let (mut b1, mut b2) = ([0u8; 4096], [0u8; 4096]);
    let (mut o_done, mut e_done) = (false, false);
    while !(o_done && e_done) {
        tokio::select! {
            r = so.read(&mut b1), if !o_done => match r {
                Ok(n) if n > 0 => { let s = d1.push(&b1[..n]); if !s.is_empty() { out(s) } }
                _ => o_done = true,
            },
            r = se.read(&mut b2), if !e_done => match r {
                Ok(n) if n > 0 => { let s = d2.push(&b2[..n]); if !s.is_empty() { out(s) } }
                _ => e_done = true,
            },
            _ = &mut stop => {
                let _ = child.kill().await;
                return Ok(None);
            }
        }
    }
    for s in [d1.finish(), d2.finish()] {
        if !s.is_empty() {
            out(s);
        }
    }
    let st = child.wait().await.map_err(|e| e.to_string())?;
    Ok(st.code())
}

/// Склеить аргументы в командную строку Windows (правила CommandLineToArgvW).
pub fn quote_args(exe: &str, args: &[String]) -> String {
    let mut line = format!("\"{}\"", exe);
    for a in args {
        line.push(' ');
        if !a.is_empty() && !a.chars().any(|c| c == ' ' || c == '\t' || c == '"') {
            line.push_str(a);
            continue;
        }
        line.push('"');
        let mut bs = 0;
        for c in a.chars() {
            match c {
                '\\' => bs += 1,
                '"' => {
                    line.extend(std::iter::repeat_n('\\', bs * 2 + 1));
                    line.push('"');
                    bs = 0;
                }
                c => {
                    line.extend(std::iter::repeat_n('\\', bs));
                    line.push(c);
                    bs = 0;
                }
            }
        }
        line.extend(std::iter::repeat_n('\\', bs * 2));
        line.push('"');
    }
    line
}

/// Запуск в собственном окне консоли. Через CreateProcessW, а не std::process::Command:
/// Command всегда передаёт дочернему процессу наши stdin/stdout (STARTF_USESTDHANDLES),
/// и консольная программа пишет не в своё окно, а в никуда; cmd /k при этом сразу закрывается.
#[cfg(windows)]
fn spawn_console(exe: &Path, args: &[String]) -> Result<(), String> {
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{CreateProcessW, CREATE_NEW_CONSOLE, PROCESS_INFORMATION, STARTUPINFOW};
    let mut line: Vec<u16> = quote_args(&exe.display().to_string(), args).encode_utf16().chain([0]).collect();
    let si = STARTUPINFOW { cb: std::mem::size_of::<STARTUPINFOW>() as u32, ..Default::default() };
    let mut pi = PROCESS_INFORMATION::default();
    unsafe {
        CreateProcessW(
            PCWSTR::null(),
            Some(PWSTR(line.as_mut_ptr())),
            None,
            None,
            false,
            CREATE_NEW_CONSOLE,
            None,
            PCWSTR::null(),
            &si,
            &mut pi,
        )
        .map_err(|e| e.message().to_string())?;
        let _ = CloseHandle(pi.hThread);
        let _ = CloseHandle(pi.hProcess);
    }
    Ok(())
}

#[cfg(not(windows))]
fn spawn_console(exe: &Path, args: &[String]) -> Result<(), String> {
    Command::new(exe).args(args).spawn().map(|_| ()).map_err(|e| e.to_string())
}

fn open_url(url: &str) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Открывать можно только http(s)-ссылки".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Версия встроенного FreeRDP и последняя версия FreeRDP на GitHub.
#[derive(Serialize)]
pub struct FreerdpStatus {
    pub installed: Option<String>,
    pub latest: Option<String>,
    pub url: String,
}

/// Наш репозиторий: релизы сборок FreeRDP (freerdp-…) и установщики приложения (v…).
/// Всё через github.com: api.github.com блокируют hosts-файлы некоторых антивирусов.
const GH_REPO: &str = "CrazyCoffee3663/RDPMgrCoffee";

/// Тег последнего релиза: github.com/releases/latest перенаправляет на /releases/tag/<tag>.
pub fn tag_from_release_url(url: &str) -> Option<String> {
    let path = url.split('?').next()?;
    let tag = path.rsplit("/tag/").next()?.trim_end_matches('/');
    if tag.is_empty() || tag.contains('/') || tag.len() > 32 {
        return None;
    }
    Some(tag.to_string())
}

pub async fn latest_release(http: &reqwest::Client, repo: &str) -> Option<(String, String)> {
    let r = http.get(format!("https://github.com/{repo}/releases/latest")).send().await.ok()?;
    if !r.status().is_success() {
        return None;
    }
    let tag = tag_from_release_url(r.url().as_str())?;
    Some((tag.clone(), format!("https://github.com/{repo}/releases/tag/{tag}")))
}

/// Ссылки на файлы релиза со страницы expanded_assets — без GitHub API.
pub fn parse_asset_urls(html: &str, repo: &str, tag: &str) -> Vec<String> {
    let needle = format!("/{repo}/releases/download/{tag}/");
    let mut out: Vec<String> = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find(&needle) {
        rest = &rest[i + needle.len()..];
        let end = rest.find('"').unwrap_or(rest.len());
        let name = &rest[..end];
        if name.is_empty() || name.contains('/') || name.contains("..") {
            continue;
        }
        let url = format!("https://github.com/{repo}/releases/download/{tag}/{name}");
        if !out.contains(&url) {
            out.push(url);
        }
    }
    out
}

/// Версии релизов приложения (теги v<версия>) со страницы releases.
/// В репозитории живут и freerdp-релизы, поэтому releases/latest не подходит.
pub fn parse_app_versions(html: &str, repo: &str) -> Vec<String> {
    let needle = format!("/{repo}/releases/tag/v");
    let mut out: Vec<String> = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find(&needle) {
        rest = &rest[i + needle.len()..];
        let end = rest.find('"').unwrap_or(rest.len());
        let v = &rest[..end];
        if !v.is_empty() && v.len() <= 16 && v.chars().all(|c| c.is_ascii_digit() || c == '.') && !out.iter().any(|x| x == v) {
            out.push(v.to_string());
        }
    }
    out
}

/// Сравнение версий вида 0.1.1 / 0.10.0 по числовым компонентам.
pub fn cmp_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let mut ia = a.split('.');
    let mut ib = b.split('.');
    loop {
        let (x, y) = (ia.next(), ib.next());
        if x.is_none() && y.is_none() {
            return std::cmp::Ordering::Equal;
        }
        let x = x.unwrap_or("0").parse::<u64>().unwrap_or(0);
        let y = y.unwrap_or("0").parse::<u64>().unwrap_or(0);
        if x != y {
            return x.cmp(&y);
        }
    }
}

/// Последний релиз приложения: максимум среди v-тегов страницы releases.
pub async fn app_latest(http: &reqwest::Client, repo: &str) -> Option<(String, String)> {
    let html = http.get(format!("https://github.com/{repo}/releases")).send().await.ok()?.text().await.ok()?;
    let v = parse_app_versions(&html, repo).into_iter().max_by(|a, b| cmp_versions(a, b))?;
    Some((v.clone(), format!("https://github.com/{repo}/releases/tag/v{v}")))
}

#[derive(Serialize)]
pub struct AppUpdateStatus {
    pub current: String,
    pub latest: Option<String>,
    pub url: String,
}

pub async fn app_update_status(http: &reqwest::Client) -> AppUpdateStatus {
    let (latest, url) = match app_latest(http, GH_REPO).await {
        Some((v, u)) => (Some(v), u),
        None => (None, format!("https://github.com/{GH_REPO}/releases")),
    };
    AppUpdateStatus { current: env!("CARGO_PKG_VERSION").to_string(), latest, url }
}

pub async fn freerdp_status(http: &reqwest::Client, freerdp_dir: Option<&Path>) -> FreerdpStatus {
    let installed = freerdp_dir
        .and_then(|d| std::fs::read_to_string(d.join("version.txt")).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let mut url = "https://github.com/FreeRDP/FreeRDP/releases".to_string();
    let latest = match latest_release(http, "FreeRDP/FreeRDP").await {
        Some((tag, u)) => {
            url = u;
            Some(tag)
        }
        None => None,
    }
    .filter(|s| !s.is_empty() && s.len() < 16 && s.chars().all(|c| c.is_ascii_digit() || c == '.'));
    FreerdpStatus { installed, latest, url }
}

#[derive(Serialize)]
pub struct FreerdpUpdateResult {
    pub version: String,
    pub already: bool,
}

static UPDATING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Заменить каталог подготовленным .new, прежний сохранить в .old; при сбое — откат.
pub fn swap_dirs(dir: &Path) -> Result<(), String> {
    let new = dir.with_file_name("freerdp.new");
    let old = dir.with_file_name("freerdp.old");
    if !new.is_dir() {
        return Err("Нет подготовленного каталога обновления".into());
    }
    let _ = std::fs::remove_dir_all(&old);
    std::fs::rename(dir, &old).map_err(|e| format!("Не удалось сдвинуть текущий FreeRDP (открыты вкладки RDP?): {}", e))?;
    if let Err(e) = std::fs::rename(&new, dir) {
        let _ = std::fs::rename(&old, dir);
        return Err(format!("Не удалось установить обновление: {}", e));
    }
    Ok(())
}

/// Новая сборка обязана запускаться: набор с недостающей DLL не должен попасть к пользователю.
pub async fn verify_freerdp_exe(exe: &Path) -> Result<(), String> {
    let mut cmd = tokio::process::Command::new(exe);
    cmd.arg("/version").creation_flags(0x08000000).kill_on_drop(true);
    let out = tokio::time::timeout(std::time::Duration::from_secs(20), cmd.output())
        .await
        .map_err(|_| "Новый FreeRDP не ответил за 20 секунд".to_string())?
        .map_err(|e| format!("Не удалось запустить новый FreeRDP: {}", e))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if !out.status.success() || !text.contains("This is FreeRDP version") {
        return Err(format!("Новый FreeRDP не запускается (не хватает DLL?): {}", text.lines().last().unwrap_or("")));
    }
    Ok(())
}

/// Обновление FreeRDP одной кнопкой: релиз со сборкой из FREERDP_REPO, SHA-256,
/// проверка запуском и замена каталога с откатом. report — этап и процент для интерфейса.
pub async fn freerdp_update(
    report: &(dyn Fn(&str, u8) + Sync),
    http: &reqwest::Client,
    dir: &Path,
) -> Result<FreerdpUpdateResult, String> {
    use std::sync::atomic::Ordering;
    if UPDATING.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return Err("Обновление уже выполняется".into());
    }
    let res = freerdp_update_inner(report, http, dir).await;
    UPDATING.store(false, Ordering::SeqCst);
    res
}

#[derive(Serialize)]
pub struct AppUpdateResult {
    pub version: String,
    pub already: bool,
}

static APP_UPDATING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Обновление приложения одной кнопкой: установщик с GitHub-релиза, SHA-256, запуск.
/// Запущенную программу установщик закроет сам (RestartManager) после подтверждения.
pub async fn app_update_install(
    report: &(dyn Fn(&str, u8) + Sync),
    http: &reqwest::Client,
) -> Result<AppUpdateResult, String> {
    use std::sync::atomic::Ordering;
    if APP_UPDATING.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return Err("Обновление уже выполняется".into());
    }
    let res = app_update_inner(report, http).await;
    APP_UPDATING.store(false, Ordering::SeqCst);
    res
}

async fn app_update_inner(
    report: &(dyn Fn(&str, u8) + Sync),
    http: &reqwest::Client,
) -> Result<AppUpdateResult, String> {
    let (version, _url) = app_latest(http, GH_REPO).await.ok_or("Не удалось получить последний релиз с GitHub")?;
    if cmp_versions(&version, env!("CARGO_PKG_VERSION")) != std::cmp::Ordering::Greater {
        return Ok(AppUpdateResult { version, already: true });
    }
    let tag = format!("v{version}");
    report("list", 0);
    let html = http
        .get(format!("https://github.com/{GH_REPO}/releases/expanded_assets/{tag}"))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    let urls = parse_asset_urls(&html, GH_REPO, &tag);
    let exe_url = urls.iter().find(|u| u.ends_with("_x64-setup.exe")).ok_or("В релизе нет установщика RDPMgrCoffee_…_x64-setup.exe")?;
    let sha_url = urls.iter().find(|u| u.ends_with(".sha256")).cloned().unwrap_or_else(|| format!("{exe_url}.sha256"));
    let expected = http
        .get(&sha_url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if expected.len() != 64 || !expected.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("В релизе нет корректной SHA-256 установщика".into());
    }
    report("download", 0);
    let exe_path = std::env::temp_dir().join(format!("connmgr-app-update-{version}.exe"));
    download_verified(report, http, exe_url, &expected, &exe_path).await?;
    report("install", 100);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS: установщик переживает закрытие приложения
        // /UPDATE /P /R: бесшовно — только прогресс, без мастера и без удаления
        // старой версии, после установки приложение запускается само
        std::process::Command::new(&exe_path)
            .args(["/UPDATE", "/P", "/R"])
            .creation_flags(0x00000008)
            .spawn()
            .map_err(|e| format!("Не удалось запустить установщик: {}", e))?;
    }
    #[cfg(not(windows))]
    return Err("Обновление приложения поддерживается только в Windows".into());
    Ok(AppUpdateResult { version, already: false })
}

/// Скачать файл с потоковой SHA-256 и прогрессом; несовпадение — файл удаляется.
async fn download_verified(
    report: &(dyn Fn(&str, u8) + Sync),
    http: &reqwest::Client,
    url: &str,
    expected: &str,
    dest: &Path,
) -> Result<(), String> {
    use sha2::Digest;
    let mut resp = http.get(url).send().await.map_err(|e| format!("Не удалось скачать файл: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("GitHub вернул {}", resp.status()));
    }
    let total = resp.content_length().unwrap_or(0);
    let mut file = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut done: u64 = 0;
    let mut hasher = sha2::Sha256::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        hasher.update(&chunk);
        std::io::Write::write_all(&mut file, &chunk).map_err(|e| e.to_string())?;
        done += chunk.len() as u64;
        if total > 0 {
            report("download", (done * 100 / total).min(100) as u8);
        }
    }
    drop(file);
    if format!("{:x}", hasher.finalize()) != expected {
        let _ = std::fs::remove_file(dest);
        return Err("SHA-256 файла не совпала с релизом — обновление отменено".into());
    }
    Ok(())
}

async fn freerdp_update_inner(
    report: &(dyn Fn(&str, u8) + Sync),
    http: &reqwest::Client,
    dir: &Path,
) -> Result<FreerdpUpdateResult, String> {
    let (tag, _url) = latest_release(http, GH_REPO).await.ok_or("Не удалось получить последний релиз с GitHub")?;
    let version = tag.strip_prefix("freerdp-").unwrap_or(&tag).to_string();
    if version.is_empty() || version.len() > 16 || !version.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return Err(format!("Неожиданный тег релиза: {}", tag));
    }
    let installed = std::fs::read_to_string(dir.join("version.txt")).ok().map(|s| s.trim().to_string());
    if installed.as_deref() == Some(version.as_str()) {
        return Ok(FreerdpUpdateResult { version, already: true });
    }
    report("list", 0);
    let html = http
        .get(format!("https://github.com/{GH_REPO}/releases/expanded_assets/{tag}"))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    let urls = parse_asset_urls(&html, GH_REPO, &tag);
    let zip_url = urls.iter().find(|u| u.ends_with("-win-x64.zip")).ok_or("В релизе нет архива freerdp-…-win-x64.zip")?;
    let sha_url = urls.iter().find(|u| u.ends_with(".sha256")).cloned().unwrap_or_else(|| format!("{zip_url}.sha256"));
    let expected = http
        .get(&sha_url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if expected.len() != 64 || !expected.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("В релизе нет корректной SHA-256 архива".into());
    }
    report("download", 0);
    let zip_path = std::env::temp_dir().join("connmgr-freerdp-update.zip");
    download_verified(report, http, zip_url, &expected, &zip_path).await?;
    report("install", 100);
    let new = dir.with_file_name("freerdp.new");
    let _ = std::fs::remove_dir_all(&new);
    std::fs::create_dir_all(&new).map_err(|e| e.to_string())?;
    {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&zip_path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("Архив повреждён: {}", e))?;
        for i in 0..zip.len() {
            let mut f = zip.by_index(i).map_err(|e| e.to_string())?;
            // enclosed_name защищает от путей вида ../../
            let Some(rel) = f.enclosed_name() else { continue };
            let out = new.join(rel);
            if f.is_dir() {
                std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
                continue;
            }
            if let Some(p) = out.parent() {
                std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            let mut w = std::fs::File::create(&out).map_err(|e| e.to_string())?;
            std::io::copy(&mut f, &mut w).map_err(|e| e.to_string())?;
        }
    }
    let _ = std::fs::remove_file(&zip_path);
    let exe = new.join("wfreerdp.exe");
    if !exe.is_file() {
        let _ = std::fs::remove_dir_all(&new);
        return Err("В архиве нет wfreerdp.exe".into());
    }
    if !new.join("version.txt").is_file() {
        std::fs::write(new.join("version.txt"), &version).map_err(|e| e.to_string())?;
    }
    if let Err(e) = verify_freerdp_exe(&exe).await {
        let _ = std::fs::remove_dir_all(&new);
        return Err(e);
    }
    swap_dirs(dir)?;
    report("done", 100);
    Ok(FreerdpUpdateResult { version, already: false })
}

#[derive(Serialize)]
pub struct WinboxStatus {
    pub installed: Option<String>,
    pub latest: Option<String>,
    pub path: String,
}

fn installed_version(tools: &Path) -> Option<String> {
    winbox_exe(tools)?;
    std::fs::read_to_string(winbox_dir(tools).join("version.txt"))
        .ok()
        .map(|s| s.trim().to_string())
        .or(Some("?".into()))
}

pub async fn status(http: &reqwest::Client, tools: &Path) -> WinboxStatus {
    let latest = match http.get(WINBOX_LATEST).send().await {
        Ok(r) if r.status().is_success() => r.text().await.ok().map(|s| s.trim().to_string()).filter(|s| {
            !s.is_empty() && s.len() < 16 && s.chars().all(|c| c.is_ascii_digit() || c == '.')
        }),
        _ => None,
    };
    WinboxStatus { installed: installed_version(tools), latest, path: winbox_dir(tools).display().to_string() }
}

/// Скачать WinBox указанной версии с download.mikrotik.com и распаковать.
pub async fn install(http: &reqwest::Client, tools: &Path, version: &str) -> Result<(), String> {
    if version.is_empty() || !version.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return Err("Неверная версия".into());
    }
    let url = format!("https://download.mikrotik.com/routeros/winbox/{}/WinBox_Windows.zip", version);
    let resp = http.get(&url).send().await.map_err(|e| format!("Не удалось скачать WinBox: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("Сервер MikroTik вернул {}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    let dir = winbox_dir(tools);
    let tmp = tools.join("winbox.new");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("Архив повреждён: {}", e))?;
        for i in 0..zip.len() {
            let mut f = zip.by_index(i).map_err(|e| e.to_string())?;
            // enclosed_name защищает от путей вида ../../
            let Some(rel) = f.enclosed_name() else { continue };
            let out = tmp.join(rel);
            if f.is_dir() {
                std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
                continue;
            }
            if let Some(p) = out.parent() {
                std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            let mut w = std::fs::File::create(&out).map_err(|e| e.to_string())?;
            std::io::copy(&mut f, &mut w).map_err(|e| e.to_string())?;
        }
    }
    std::fs::write(tmp.join("version.txt"), version).map_err(|e| e.to_string())?;
    // Скачанный WinBox должен быть подписан MikroTik — иначе не устанавливаем
    let new_exe = ["WinBox.exe", "winbox64.exe", "winbox.exe"].iter().map(|n| tmp.join(n)).find(|p| p.is_file());
    match new_exe {
        Some(e) => {
            if let Err(err) = verify_winbox(&e) {
                let _ = std::fs::remove_dir_all(&tmp);
                return Err(err);
            }
        }
        None => {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err("В архиве не найден WinBox.exe".into());
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::rename(&tmp, &dir).map_err(|e| format!("Не удалось заменить WinBox (он запущен?): {}", e))?;
    if winbox_exe(tools).is_none() {
        return Err("В архиве не найден WinBox.exe".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_and_expand() {
        assert_eq!(split_args(r#"/k ping -t "my host" x"#), vec!["/k", "ping", "-t", "my host", "x"]);
        let p = ConnParams {
            name: "r1".into(),
            protocol: "ssh".into(),
            host: "10.0.0.1".into(),
            username: "admin".into(),
            password: "p w".into(),
            ..Default::default()
        };
        assert_eq!(expand("%USER%@%HOST%:%PORT%", &p), "admin@10.0.0.1:22");
        // Пароль не подставляется ни в каком виде
        assert_eq!(expand("%PASSWORD%", &p), "%PASSWORD%");
        let (a, clip) = prepare("%ADDR% %USER% %PASSWORD% -p=%password% --pass:%PASSWORD%x", &p);
        assert_eq!(a, vec!["10.0.0.1", "admin"]);
        assert!(clip);
        assert!(!a.iter().any(|x| x.contains("p w")));
        assert_eq!(
            quote_args("cmd", &["/k".into(), "ping".into(), "a b".into(), r#"x"y"#.into(), r"c:\d e\".into()]),
            r#""cmd" /k ping "a b" "x\"y" "c:\d e\\""#
        );
    }

    #[test]
    fn release_tag_from_redirect_url() {
        assert_eq!(
            tag_from_release_url("https://github.com/o/r/releases/tag/freerdp-3.33.1"),
            Some("freerdp-3.33.1".into())
        );
        assert_eq!(tag_from_release_url("https://github.com/o/r/releases/tag/3.33.1/"), Some("3.33.1".into()));
        assert_eq!(tag_from_release_url("https://github.com/o/r/releases"), None);
        assert_eq!(tag_from_release_url("https://github.com/o/r/releases/tag/"), None);
        assert_eq!(tag_from_release_url("https://github.com/o/r/releases/tag/a/b"), None);
    }

    #[test]
    fn asset_urls_from_expanded_assets_html() {
        let html = r#"<a href="/o/r/releases/download/freerdp-3.33.1/freerdp-3.33.1-win-x64.zip">zip</a>
            <a href="/o/r/releases/download/freerdp-3.33.1/freerdp-3.33.1-win-x64.zip.sha256">sha</a>
            <a href="/o/r/releases/download/freerdp-3.33.1/freerdp-3.33.1-win-x64.zip">dup</a>
            <a href="/o/r/releases/download/freerdp-3.33.1/../../evil">bad</a>
            <a href="/other/r/releases/download/freerdp-3.33.1/x.zip">other</a>"#;
        let urls = parse_asset_urls(html, "o/r", "freerdp-3.33.1");
        assert_eq!(
            urls,
            vec![
                "https://github.com/o/r/releases/download/freerdp-3.33.1/freerdp-3.33.1-win-x64.zip".to_string(),
                "https://github.com/o/r/releases/download/freerdp-3.33.1/freerdp-3.33.1-win-x64.zip.sha256".to_string(),
            ]
        );
    }

    #[test]
    fn app_versions_from_releases_html() {
        let html = r#"<a href="/o/r/releases/tag/v0.1.1">x</a><a href="/o/r/releases/tag/freerdp-3.33.1">y</a>
            <a href="/o/r/releases/tag/v0.1.0">z</a><a href="/o/r/releases/tag/v0.1.1">dup</a>
            <a href="/o/r/releases/tag/v1.0.0-beta">bad</a><a href="/o/r/releases/tag/v">empty</a>"#;
        assert_eq!(parse_app_versions(html, "o/r"), vec!["0.1.1".to_string(), "0.1.0".to_string()]);
        assert_eq!(parse_app_versions(html, "other/repo"), Vec::<String>::new());
    }

    #[test]
    fn version_compare_numeric_components() {
        use std::cmp::Ordering::*;
        assert_eq!(cmp_versions("0.1.1", "0.1.0"), Greater);
        assert_eq!(cmp_versions("0.10.0", "0.9.0"), Greater);
        assert_eq!(cmp_versions("0.1", "0.1.0"), Equal);
        assert_eq!(cmp_versions("0.1.0", "0.1.1"), Less);
        assert_eq!(cmp_versions("1.0.0", "0.99.99"), Greater);
    }

    #[test]
    fn swap_dirs_roundtrip_and_rollback() {
        let base = std::env::temp_dir().join(format!("connmgr-swap-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("freerdp");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("version.txt"), "1").unwrap();
        // без .new — отказ, текущий каталог не тронут
        assert!(swap_dirs(&dir).is_err());
        assert_eq!(std::fs::read_to_string(dir.join("version.txt")).unwrap(), "1");
        let new = base.join("freerdp.new");
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("version.txt"), "2").unwrap();
        swap_dirs(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("version.txt")).unwrap(), "2");
        assert_eq!(std::fs::read_to_string(base.join("freerdp.old").join("version.txt")).unwrap(), "1");
        assert!(!new.exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}

/// Оба ping-теста считают процессы ping.exe в системе — без сериализации чужой ping
/// попадает в базовый счётчик и ломает сравнение
#[cfg(all(test, windows))]
static PING_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(all(test, windows))]
mod exec_tests {
    use super::*;
    #[tokio::test]
    async fn ping_output_decoded() {
        let _g = PING_LOCK.lock().unwrap();
        let buf = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let b = buf.clone();
        let (_tx, rx) = tokio::sync::oneshot::channel();
        let code = exec(Path::new("ping"), &["-n".into(), "1".into(), "127.0.0.1".into()], move |s| b.lock().unwrap().push_str(&s), rx)
            .await
            .unwrap();
        let out = buf.lock().unwrap().clone();
        eprintln!("{}", out);
        assert_eq!(code, Some(0));
        assert!(out.contains("127.0.0.1"));
        assert!(!out.contains('\u{FFFD}'));
    }
}

#[cfg(all(test, windows))]
mod trust_tests {
    use super::*;

    #[test]
    fn find_exe_never_uses_current_dir() {
        let p = find_exe("ping").unwrap();
        assert!(p.to_string_lossy().to_lowercase().contains("system32"), "{}", p.display());
        assert!(find_exe(r".\ping.exe").is_err());
        assert!(find_exe("no-such-program-xyz").is_err());
    }

    #[test]
    fn unsigned_file_rejected() {
        let f = std::env::temp_dir().join(format!("unsigned-{}.exe", std::process::id()));
        std::fs::write(&f, b"MZ not really").unwrap();
        assert!(verify_winbox(&f).is_err());
        let _ = std::fs::remove_file(&f);
    }

    /// Установленный WinBox: cargo test winbox_signed -- --ignored --nocapture
    #[test]
    #[ignore]
    fn winbox_signed() {
        let f = std::path::PathBuf::from(std::env::var("APPDATA").unwrap()).join(r"io.github.connmgr\tools\winbox\WinBox.exe");
        let s = authenticode_signer(&f).unwrap();
        eprintln!("signer: {}", s);
        verify_winbox(&f).unwrap();
    }
}

/// v55/v79е: остановка инструмента убивает всё дерево процессов, а не только cmd
#[cfg(all(test, windows))]
mod exec_tree_tests {
    use super::*;

    fn ping_count() -> u32 {
        use windows::Win32::System::Diagnostics::ToolHelp::*;
        let mut n = 0;
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return 0 };
            let mut e: PROCESSENTRY32W = std::mem::zeroed();
            e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            if Process32FirstW(snap, &mut e).is_ok() {
                loop {
                    let name = String::from_utf16_lossy(&e.szExeFile);
                    if name.trim_end_matches('\0').eq_ignore_ascii_case("ping.exe") {
                        n += 1;
                    }
                    if Process32NextW(snap, &mut e).is_err() {
                        break;
                    }
                }
            }
            let _ = windows::Win32::Foundation::CloseHandle(snap);
        }
        n
    }

    #[tokio::test]
    async fn stop_kills_process_tree() {
        let _g = super::PING_LOCK.lock().unwrap();
        let before = ping_count();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let exe = find_exe("cmd").unwrap();
        // Аргументы раздельно: cmd /k "ping -t …" в кавычках не запускает ping
        let args = vec!["/k".to_string(), "ping".to_string(), "-t".to_string(), "127.0.0.1".to_string()];
        let h = tokio::spawn(async move { exec(&exe, &args, |_| {}, rx).await });
        let mut spawned = false;
        for _ in 0..60 {
            if h.is_finished() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if ping_count() > before {
                spawned = true;
                break;
            }
        }
        if !spawned {
            // Не ждём вечно живой exec: останавливаем и показываем его результат
            let _ = tx.send(());
            let r = tokio::time::timeout(std::time::Duration::from_secs(5), h).await;
            panic!("дочерний ping не появился (before={}): exec={:?}", before, r.map(|x| x.unwrap()));
        }
        let _ = tx.send(());
        let stopped = tokio::time::timeout(std::time::Duration::from_secs(10), h).await.expect("exec не завершился после остановки").unwrap();
        assert_eq!(stopped.unwrap(), None);
        let mut gone = false;
        for _ in 0..60 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if ping_count() <= before {
                gone = true;
                break;
            }
        }
        assert!(gone, "дочерний ping пережил остановку — дерево не убито");
    }
}
