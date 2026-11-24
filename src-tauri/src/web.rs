//! Веб-вкладки: встроенная страница (веб-интерфейс роутера, камеры, панели управления)
//! — отдельный WebView поверх области вкладки главного окна.
//!
//! Безопасность: страница чужая, поэтому
//!  * у веб-вкладок нет прав на команды программы (capabilities выданы только webview «main»
//!    и окнам инструментов, а не всему окну);
//!  * переходы только на http/https и никогда на адреса самой программы (tauri.localhost,
//!    ipc.localhost, сервер разработки) — иначе страница могла бы загрузить интерфейс программы;
//!  * новые окна (window.open, target=_blank) открываются в этой же вкладке.

use tauri::webview::{NewWindowResponse, PageLoadEvent};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Url, WebviewBuilder, WebviewUrl};

fn allowed(u: &Url) -> bool {
    if !matches!(u.scheme(), "http" | "https") {
        return false;
    }
    let host = u.host_str().unwrap_or("").to_ascii_lowercase();
    if host == "tauri.localhost" || host == "ipc.localhost" || host.ends_with(".tauri.localhost") {
        return false;
    }
    // Сервер разработки интерфейса: на dev-порту блокируется весь loopback-диапазон,
    // а не два написания — [::1], localhost. и 127.0.0.2 раньше проходили фильтр
    if cfg!(debug_assertions) && u.port() == Some(1420) {
        let h = host.trim_end_matches('.');
        let v6 = h.trim_matches(['[', ']']).parse::<std::net::Ipv6Addr>().ok();
        let loopback = h == "localhost"
            || h.ends_with(".localhost")
            || h.parse::<std::net::Ipv4Addr>().map(|ip| ip.is_loopback() || ip.is_unspecified()).unwrap_or(false)
            || v6.map(|ip| ip.is_loopback() || ip.is_unspecified() || ip.to_ipv4_mapped().map(|v4| v4.is_loopback()).unwrap_or(false)).unwrap_or(false);
        if loopback {
            return false;
        }
    }
    true
}

fn check_label(label: &str) -> Result<(), String> {
    if label.starts_with("web-") && label.len() < 64 && label[4..].chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        Ok(())
    } else {
        Err("Неверный идентификатор веб-вкладки".into())
    }
}

#[derive(Clone, serde::Serialize)]
struct NavEvent {
    label: String,
    url: String,
    loading: bool,
}

/// Открыть страницу поверх области вкладки (логические пиксели окна).
pub fn open(app: &AppHandle, label: String, url: String, x: f64, y: f64, w: f64, h: f64) -> Result<(), String> {
    check_label(&label)?;
    let u = Url::parse(url.trim()).map_err(|_| "Неверный адрес страницы".to_string())?;
    if !allowed(&u) {
        return Err("Открывать можно только адреса http:// и https://".into());
    }
    if let Some(old) = app.get_webview(&label) {
        let _ = old.close();
    }
    let win = app.get_window("main").ok_or("Главное окно не найдено")?;
    let (a1, a2, l1, l2) = (app.clone(), app.clone(), label.clone(), label.clone());
    let builder = WebviewBuilder::new(&label, WebviewUrl::External(u))
        .on_navigation(allowed)
        .on_new_window(move |u, _| {
            if allowed(&u) {
                if let Some(wv) = a1.get_webview(&l1) {
                    let _ = wv.navigate(u);
                }
            }
            NewWindowResponse::Deny
        })
        .on_page_load(move |wv, p| {
            let _ = a2.emit(
                "web-nav",
                NavEvent { label: l2.clone(), url: p.url().to_string(), loading: matches!(p.event(), PageLoadEvent::Started) },
            );
            let _ = wv;
        })
        .disable_drag_drop_handler();
    win.add_child(builder, LogicalPosition::new(x, y), LogicalSize::new(w.max(1.0), h.max(1.0)))
        .map_err(|e| format!("Не удалось открыть страницу: {}", e))?;
    // Хранилище могли заблокировать, пока страница открывалась (гонка с vault_lock):
    // новый webview сразу прячем — place() покажет его после разблокировки
    if *app.state::<crate::rdp::Rdp>().locked.lock().unwrap() {
        if let Some(wv) = app.get_webview(&label) {
            let _ = wv.hide();
        }
    }
    Ok(())
}

pub fn place(app: &AppHandle, label: &str, x: f64, y: f64, w: f64, h: f64, visible: bool) -> Result<(), String> {
    check_label(label)?;
    let locked = *app.state::<crate::rdp::Rdp>().locked.lock().unwrap();
    let Some(wv) = app.get_webview(label) else { return Ok(()) };
    let _ = wv.set_position(LogicalPosition::new(x, y));
    let _ = wv.set_size(LogicalSize::new(w.max(1.0), h.max(1.0)));
    let _ = if visible && !locked { wv.show() } else { wv.hide() };
    Ok(())
}

pub fn close(app: &AppHandle, label: &str) {
    if check_label(label).is_ok() {
        if let Some(wv) = app.get_webview(label) {
            let _ = wv.close();
        }
    }
}

/// back | forward | reload | stop
pub fn nav(app: &AppHandle, label: &str, op: &str) -> Result<(), String> {
    check_label(label)?;
    let wv = app.get_webview(label).ok_or("Страница закрыта")?;
    match op {
        "back" => wv.eval("history.back()"),
        "forward" => wv.eval("history.forward()"),
        "reload" => wv.reload(),
        "stop" => wv.eval("window.stop()"),
        _ => return Err("Неизвестное действие".into()),
    }
    .map_err(|e| e.to_string())
}

/// Блокировка хранилища: спрятать все веб-вкладки (интерфейс покажет их снова после разблокировки)
pub fn hide_all(app: &AppHandle) {
    for (label, wv) in app.webviews() {
        if label.starts_with("web-") {
            let _ = wv.hide();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_filter() {
        let ok = |s: &str| allowed(&Url::parse(s).unwrap());
        assert!(ok("https://192.168.88.1/webfig/"));
        assert!(ok("http://router.local"));
        assert!(!ok("http://tauri.localhost/"));
        assert!(!ok("https://ipc.localhost/plugin:opener|open_url"));
        assert!(!ok("file:///C:/Windows/win.ini"));
        assert!(!ok("javascript:alert(1)"));
        assert!(!ok("tauri://localhost"));
        assert!(check_label("web-abc-1").is_ok());
        assert!(check_label("main").is_err());
    }

    /// v61/v79д: dev-фильтр закрывает весь loopback на порту 1420, внешние адреса — нет
    #[test]
    fn dev_filter_covers_loopback_forms() {
        let ok = |s: &str| allowed(&Url::parse(s).unwrap());
        // В debug-сборке (cargo test) сервер разработки недоступен веб-вкладке
        if cfg!(debug_assertions) {
            assert!(!ok("http://localhost:1420/"));
            assert!(!ok("http://127.0.0.1:1420/"));
            assert!(!ok("http://[::1]:1420/"));
            assert!(!ok("http://localhost.:1420/"));
            assert!(!ok("http://127.0.0.2:1420/"));
            assert!(!ok("http://127.1:1420/"));
            assert!(!ok("http://2130706433:1420/"));
            assert!(!ok("http://sub.localhost:1420/"));
            assert!(!ok("http://0.0.0.0:1420/"));
            assert!(!ok("http://[::]:1420/"));
            assert!(!ok("http://[::ffff:127.0.0.1]:1420/"));
            // Другой порт и внешние адреса не затрагиваются
            assert!(ok("http://localhost:8080/"));
            assert!(ok("http://192.168.1.1:1420/"));
            assert!(ok("https://example.com:1420/"));
        }
    }
}
