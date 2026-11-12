//! Внешние инструменты (ping, tracert, WinBox, браузер…) и установка/обновление WinBox.
//!
//! Подстановки в аргументах: %HOST% %PORT% %USER% %PASSWORD% %DOMAIN% %NAME% %PROTOCOL%.
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
            _ => "",
        }
        .to_string()
    });
    arg.replace("%HOST%", &p.host)
        .replace("%PORT%", &port)
        .replace("%USER%", &p.username)
        .replace("%PASSWORD%", &p.password)
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

/// Запустить инструмент. command: путь к программе, "winbox" (встроенный) или "url" (открыть ссылку).
pub fn run(tools: &Path, command: &str, args: &str, p: &ConnParams) -> Result<(), String> {
    let args: Vec<String> = split_args(args).iter().map(|a| expand(a, p)).collect();
    if command.eq_ignore_ascii_case("url") {
        let url = args.first().ok_or("Не указан адрес")?;
        return open_url(url);
    }
    let exe = if command.eq_ignore_ascii_case("winbox") {
        winbox_exe(tools).ok_or("WINBOX_MISSING")?
    } else {
        PathBuf::from(expand(command, p))
    };
    let mut cmd = Command::new(&exe);
    cmd.args(&args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Консольные программы (ping, tracert) — в своём окне консоли
        cmd.creation_flags(0x00000010); // CREATE_NEW_CONSOLE
    }
    cmd.spawn().map_err(|e| format!("Не удалось запустить {}: {}", exe.display(), e))?;
    Ok(())
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
        assert_eq!(expand("%PASSWORD%", &p), "p w");
    }
}
