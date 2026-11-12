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

    pub fn set(text: &str) -> Result<(), String> {
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        open()?;
        let r = unsafe {
            let _ = EmptyClipboard();
            let bytes = wide.len() * 2;
            match GlobalAlloc(GMEM_MOVEABLE, bytes) {
                Ok(h) => {
                    let p = GlobalLock(h) as *mut u16;
                    std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
                    let _ = GlobalUnlock(h);
                    SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(h.0)))
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                }
                Err(e) => Err(e.to_string()),
            }
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

pub fn set(text: &str, clear_after: Option<u64>) -> Result<(), String> {
    #[cfg(windows)]
    {
        win::set(text)?;
        let gen = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(secs) = clear_after.filter(|s| *s > 0) {
            let copied = text.to_string();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(secs));
                if GENERATION.load(Ordering::SeqCst) == gen
                    && win::get().map(|t| t == copied).unwrap_or(false)
                {
                    let _ = win::set("");
                }
            });
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (text, clear_after);
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
