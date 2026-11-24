//! Буфер обмена (текст). Скопированный пароль очищается через заданное время,
//! если пользователь не скопировал за это время что-то другое.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

static GENERATION: AtomicU64 = AtomicU64::new(0);
/// Последний положенный нами секрет: очистка при блокировке/выходе сравнивает с ним,
/// чтобы не стереть чужое содержимое буфера
static LAST_SECRET: Mutex<Option<String>> = Mutex::new(None);
/// Каталог маркера «в буфере наш секрет» (переживает падение процесса)
static MARKER_DIR: OnceLock<PathBuf> = OnceLock::new();

fn forget_secret() {
    if let Some(mut s) = LAST_SECRET.lock().unwrap().take() {
        zeroize::Zeroize::zeroize(&mut s);
    }
}

fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(s.as_bytes()).iter().map(|b| format!("{:02x}", b)).collect()
}

fn marker_path() -> Option<PathBuf> {
    MARKER_DIR.get().map(|d| d.join("clip-pending"))
}

fn write_marker(text: &str) {
    if let Some(p) = marker_path() {
        // На диск — только хеш, не сам секрет
        let _ = std::fs::write(p, sha256_hex(text));
    }
}

fn remove_marker() {
    if let Some(p) = marker_path() {
        let _ = std::fs::remove_file(p);
    }
}

/// Вызывать при старте: где хранить маркер «в буфере наш секрет».
pub fn init(dir: &std::path::Path) {
    let _ = MARKER_DIR.set(dir.to_path_buf());
}

/// Очистить буфер, если в нём всё ещё наш последний секрет (блокировка хранилища, выход).
pub fn clear_if_secret() {
    #[cfg(windows)]
    {
        let last = LAST_SECRET.lock().unwrap().clone();
        if let Some(secret) = last {
            if win::get().map(|t| t == secret).unwrap_or(false) {
                if win::set("", false).is_ok() {
                    forget_secret();
                    remove_marker();
                }
            } else {
                forget_secret();
                remove_marker();
            }
        }
    }
}

/// При старте: если прошлый процесс оставил наш секрет в буфере (падение или выход
/// до истечения TTL), очистить его. Маркер хранит только хеш — сравниваем содержимое.
pub fn startup_cleanup() {
    #[cfg(windows)]
    {
        let Some(p) = marker_path() else { return };
        let Ok(hash) = std::fs::read_to_string(&p) else { return };
        let hash = hash.trim();
        if !hash.is_empty() {
            if let Ok(cur) = win::get() {
                if sha256_hex(&cur) == hash {
                    let _ = win::set("", false);
                }
            }
        }
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(windows)]
mod win {
    use windows::Win32::Foundation::{HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::*;
    use windows::Win32::System::Memory::*;
    use windows::Win32::System::Ole::CF_UNICODETEXT;

    fn open() -> Result<(), String> {
        // Буфер может быть занят другой программой — несколько попыток
        for _ in 0..10 {
            if unsafe { OpenClipboard(None) }.is_ok() {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        Err("Буфер обмена занят другой программой".into())
    }

    /// Положить блок данных в буфер (буфер уже открыт).
    unsafe fn put(format: u32, data: &[u8]) -> Result<(), String> {
        let h = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1)).map_err(|e| e.to_string())?;
        let p = GlobalLock(h) as *mut u8;
        std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
        let _ = GlobalUnlock(h);
        SetClipboardData(format, Some(HANDLE(h.0))).map(|_| ()).map_err(|e| e.to_string())
    }

    fn format(name: &str) -> u32 {
        let w: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe { RegisterClipboardFormatW(windows::core::PCWSTR(w.as_ptr())) }
    }

    /// secret: пометки Windows, чтобы значение не попало в журнал буфера (Win+V),
    /// облачный буфер обмена и программы-менеджеры буфера.
    pub fn set(text: &str, secret: bool) -> Result<(), String> {
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes: Vec<u8> = wide.iter().flat_map(|c| c.to_le_bytes()).collect();
        open()?;
        let r = unsafe {
            let _ = EmptyClipboard();
            let mut r = Ok(());
            if secret {
                let zero = 0u32.to_le_bytes();
                for (name, data) in [
                    ("ExcludeClipboardContentFromMonitorProcessing", &zero[..]),
                    ("CanIncludeInClipboardHistory", &zero[..]),
                    ("CanUploadToCloudClipboard", &zero[..]),
                ] {
                    let f = format(name);
                    if f != 0 {
                        r = r.and(put(f, data));
                    }
                }
            }
            r.and(put(CF_UNICODETEXT.0 as u32, &bytes))
        };
        unsafe {
            let _ = CloseClipboard();
        }
        r
    }

    pub fn get() -> Result<String, String> {
        open()?;
        let r = unsafe {
            match GetClipboardData(CF_UNICODETEXT.0 as u32) {
                Ok(h) => {
                    let hg = HGLOBAL(h.0);
                    let p = GlobalLock(hg) as *const u16;
                    if p.is_null() {
                        Ok(String::new())
                    } else {
                        // Скан ограничен размером блока: чужой буфер без нуля не
                        // должен уводить чтение за пределы выделенной памяти
                        let cap = GlobalSize(hg) / 2;
                        let mut len = 0;
                        while len < cap && *p.add(len) != 0 {
                            len += 1;
                        }
                        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
                        let _ = GlobalUnlock(hg);
                        Ok(s)
                    }
                }
                Err(_) => Ok(String::new()),
            }
        };
        unsafe {
            let _ = CloseClipboard();
        }
        r
    }
}

pub fn set(text: &str, clear_after: Option<u64>, secret: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        win::set(text, secret)?;
        let gen = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        // Прежнего секрета в буфере больше нет
        forget_secret();
        remove_marker();
        if secret {
            if let Some(secs) = clear_after.filter(|s| *s > 0) {
                *LAST_SECRET.lock().unwrap() = Some(text.to_string());
                write_marker(text);
                let mut copied = text.to_string();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(secs));
                    // Буфер может быть занят: повторяем в окне ~30 с, пока поколение
                    // не сменилось и содержимое всё ещё наше (одна попытка теряла пароль навсегда)
                    for _ in 0..10 {
                        if GENERATION.load(Ordering::SeqCst) != gen {
                            break;
                        }
                        if win::get().map(|t| t == copied).unwrap_or(false) {
                            if win::set("", false).is_ok() {
                                forget_secret();
                                remove_marker();
                                break;
                            }
                        } else {
                            // Содержимое сменила другая программа — очищать нечего
                            forget_secret();
                            remove_marker();
                            break;
                        }
                        std::thread::sleep(Duration::from_secs(3));
                    }
                    zeroize::Zeroize::zeroize(&mut copied);
                });
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (text, clear_after, secret);
        Err("Буфер обмена пока поддерживается только в Windows".into())
    }
}

pub fn get() -> Result<String, String> {
    #[cfg(windows)]
    {
        win::get()
    }
    #[cfg(not(windows))]
    {
        Err("Буфер обмена пока поддерживается только в Windows".into())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use windows::Win32::System::DataExchange::{IsClipboardFormatAvailable, RegisterClipboardFormatW};

    fn has(name: &str) -> bool {
        let w: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe { IsClipboardFormatAvailable(RegisterClipboardFormatW(windows::core::PCWSTR(w.as_ptr()))).is_ok() }
    }

    /// Трогает буфер обмена (прежний текст возвращается): cargo test secret_flags -- --ignored
    #[test]
    #[ignore]
    fn secret_flags() {
        let old = super::get().unwrap_or_default();
        super::set("секрет-тест", None, true).unwrap();
        let flags = (has("CanIncludeInClipboardHistory"), has("CanUploadToCloudClipboard"), has("ExcludeClipboardContentFromMonitorProcessing"));
        let text = super::get().unwrap();
        super::set(&old, None, false).unwrap();
        assert_eq!(text, "секрет-тест");
        assert_eq!(flags, (true, true, true));
    }

    /// v79г: смена поколения отменяет очистку прежнего секрета; clear_if_secret чистит только своё.
    /// Трогает буфер обмена: cargo test generation -- --ignored
    #[test]
    #[ignore]
    fn generation_guard_and_clear_if_secret() {
        let old = super::get().unwrap_or_default();
        super::set("секрет-поколение", Some(1), true).unwrap();
        // Пользователь скопировал другое — таймер прежнего секрета не должен его стереть
        super::set("не секрет", None, false).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1600));
        assert_eq!(super::get().unwrap(), "не секрет");
        // clear_if_secret не трогает чужое содержимое
        super::clear_if_secret();
        assert_eq!(super::get().unwrap(), "не секрет");
        // А свой секрет — очищает
        super::set("секрет-блокировка", Some(60), true).unwrap();
        super::clear_if_secret();
        assert_eq!(super::get().unwrap(), "");
        super::set(&old, None, false).unwrap();
    }
}
