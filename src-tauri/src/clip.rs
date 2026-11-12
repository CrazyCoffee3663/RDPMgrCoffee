//! Буфер обмена (текст). Скопированный пароль очищается через заданное время,
//! если пользователь не скопировал за это время что-то другое.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static GENERATION: AtomicU64 = AtomicU64::new(0);

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
                        let mut len = 0;
                        while *p.add(len) != 0 {
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
        if let Some(secs) = clear_after.filter(|s| *s > 0) {
            let copied = text.to_string();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(secs));
                if GENERATION.load(Ordering::SeqCst) == gen
                    && win::get().map(|t| t == copied).unwrap_or(false)
                {
                    let _ = win::set("", false);
                }
            });
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
}
