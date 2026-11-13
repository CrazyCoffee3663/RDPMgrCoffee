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

pub async fn freerdp_status(http: &reqwest::Client, freerdp_dir: Option<&Path>) -> FreerdpStatus {
    let installed = freerdp_dir
        .and_then(|d| std::fs::read_to_string(d.join("version.txt")).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let mut url = "https://github.com/FreeRDP/FreeRDP/releases".to_string();
    let latest = match http
        .get("https://api.github.com/repos/FreeRDP/FreeRDP/releases/latest")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r.json::<serde_json::Value>().await.ok().and_then(|v| {
            if let Some(u) = v["html_url"].as_str().filter(|u| u.starts_with("https://github.com/FreeRDP/FreeRDP/")) {
                url = u.to_string();
            }
            v["tag_name"].as_str().map(|t| t.trim_start_matches('v').to_string())
        }),
        _ => None,
    }
    .filter(|s| !s.is_empty() && s.len() < 16 && s.chars().all(|c| c.is_ascii_digit() || c == '.'));
    FreerdpStatus { installed, latest, url }
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
}

#[cfg(all(test, windows))]
mod exec_tests {
    use super::*;
    #[tokio::test]
    async fn ping_output_decoded() {
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
