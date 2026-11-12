//! RDP во вкладке: создаём окно-хост поверх области вкладки и встраиваем в него
//! FreeRDP (/parent-window). Параметры, включая пароль, передаются через stdin.
//!
//! Хост — отдельное окно без рамки, принадлежащее главному (owned popup), а не дочернее:
//! WebView2 рисует через DirectComposition поверх всех дочерних окон, поэтому дочернее
//! окно оказывалось под страницей и сеанс не было видно.

use crate::conn::ConnParams;
use crate::sessions::emit_status;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Manager};

pub static FREERDP_EXE: OnceLock<PathBuf> = OnceLock::new();

struct RdpSession {
    hwnd: isize,
    child: Arc<Mutex<Child>>,
    /// Последнее положение в клиентских координатах главного окна и видимость
    rect: (i32, i32, i32, i32, bool),
}

#[derive(Default)]
pub struct Rdp {
    sessions: Mutex<HashMap<u32, RdpSession>>,
}

#[cfg(windows)]
mod win {
    use windows::core::w;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::Graphics::Gdi::{GetStockObject, BLACK_BRUSH, HBRUSH};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::*;

    extern "system" fn host_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        unsafe {
            if msg == WM_SIZE {
                // Окно FreeRDP всегда занимает всю область вкладки
                let w = (lp.0 & 0xffff) as i32;
                let h = ((lp.0 >> 16) & 0xffff) as i32;
                if let Ok(child) = GetWindow(hwnd, GW_CHILD) {
                    let _ = MoveWindow(child, 0, 0, w, h, true);
                }
                // Когда размер устоится — ещё раз сообщить FreeRDP, иначе он
                // пропускает частые изменения и разрешение сервера не обновляется
                SetTimer(Some(hwnd), RESIZE_TIMER, 200, None);
            }
            // Фокус хоста отдаём окну FreeRDP: пока у него нет фокуса, FreeRDP
            // держит ввод приостановленным (SuspendInput) и не шлёт мышь/клавиатуру.
            // Хост получает фокус при щелчке по сеансу (активация окна).
            if msg == WM_SETFOCUS {
                if let Ok(child) = GetWindow(hwnd, GW_CHILD) {
                    let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(child));
                }
                return LRESULT(0);
            }
            if msg == WM_TIMER && wp.0 == RESIZE_TIMER {
                let _ = KillTimer(Some(hwnd), RESIZE_TIMER);
                notify_resized(hwnd);
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
    }

    const RESIZE_TIMER: usize = 1;

    /// Повторно отправить окну FreeRDP WM_SIZE + WM_EXITSIZEMOVE —
    /// по ним он пересчитывает разрешение (/dynamic-resolution).
    unsafe fn notify_resized(host: HWND) {
        let mut rc = Default::default();
        if GetClientRect(host, &mut rc).is_err() {
            return;
        }
        if let Ok(child) = GetWindow(host, GW_CHILD) {
            let lp = LPARAM((((rc.bottom as u32 & 0xffff) << 16) | (rc.right as u32 & 0xffff)) as isize);
            let _ = PostMessageW(Some(child), WM_SIZE, WPARAM(0), lp);
            let _ = PostMessageW(Some(child), WM_EXITSIZEMOVE, WPARAM(0), LPARAM(0));
        }
    }

    fn register() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| unsafe {
            let hinst = GetModuleHandleW(None).unwrap_or_default();
            let wc = WNDCLASSW {
                lpfnWndProc: Some(host_proc),
                hInstance: hinst.into(),
                lpszClassName: w!("connmgr_rdp_host"),
                hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                ..Default::default()
            };
            RegisterClassW(&wc);
        });
    }

    /// Вызывать в главном потоке. owner — главное окно приложения.
    pub fn create(parent: isize) -> Result<isize, String> {
        register();
        unsafe {
            let hinst = GetModuleHandleW(None).map_err(|e| e.to_string())?;
            let h = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                w!("connmgr_rdp_host"),
                w!(""),
                WS_POPUP | WS_CLIPCHILDREN,
                0,
                0,
                10,
                10,
                Some(HWND(parent as _)),
                None,
                Some(hinst.into()),
                None,
            )
            .map_err(|e| format!("Не удалось создать окно RDP: {}", e))?;
            Ok(h.0 as isize)
        }
    }

    /// x, y — в клиентских координатах главного окна (физические пиксели).
    pub fn place(hwnd: isize, owner: isize, x: i32, y: i32, w: i32, h: i32, visible: bool) {
        unsafe {
            let owner = HWND(owner as _);
            // Главное окно свёрнуто — не показываем (Windows сама вернёт при восстановлении)
            let visible = visible && !IsIconic(owner).as_bool();
            let mut pt = windows::Win32::Foundation::POINT { x, y };
            let _ = windows::Win32::Graphics::Gdi::ClientToScreen(owner, &mut pt);
            let flags = if visible { SWP_SHOWWINDOW } else { SWP_HIDEWINDOW };
            let _ = SetWindowPos(
                HWND(hwnd as _),
                None,
                pt.x,
                pt.y,
                w.max(1),
                h.max(1),
                flags | SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER,
            );
        }
    }

    /// Страховка: окно FreeRDP должно быть видимым и занимать всю область хоста.
    /// FreeRDP иногда создаёт/пересоздаёт своё окно (после переподключения, через шлюз)
    /// уже после нашего последнего WM_SIZE, и оно остаётся крошечным или скрытым.
    pub fn fixup(hwnd: isize, pid: u32) {
        unsafe {
            let host = HWND(hwnd as _);
            adopt(host, pid);
            if !IsWindowVisible(host).as_bool() {
                return;
            }
            let mut rc = Default::default();
            if GetClientRect(host, &mut rc).is_err() {
                return;
            }
            if let Ok(child) = GetWindow(host, GW_CHILD) {
                let mut cr = Default::default();
                let _ = GetWindowRect(child, &mut cr);
                let (cw, ch) = (cr.right - cr.left, cr.bottom - cr.top);
                // Фокус у пустого хоста (окно FreeRDP появилось позже) — передать его сеансу
                use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
                if GetFocus() == host {
                    let _ = SetFocus(Some(child));
                }
                if cw != rc.right || ch != rc.bottom || !IsWindowVisible(child).as_bool() {
                    let _ = MoveWindow(child, 0, 0, rc.right, rc.bottom, true);
                    let _ = ShowWindow(child, SW_SHOW);
                    SetTimer(Some(host), RESIZE_TIMER, 200, None);
                }
            }
        }
    }

    /// Окна процесса FreeRDP вне вкладки: главное окно встраиваем,
    /// диалоги (запрос пароля, сертификат) поднимаем поверх приложения.
    unsafe fn adopt(host: HWND, pid: u32) {
        struct Ctx {
            pid: u32,
            found: Vec<HWND>,
        }
        extern "system" fn cb(h: HWND, lp: LPARAM) -> windows::core::BOOL {
            unsafe {
                let ctx = &mut *(lp.0 as *mut Ctx);
                let mut p = 0u32;
                GetWindowThreadProcessId(h, Some(&mut p));
                if p == ctx.pid {
                    ctx.found.push(h);
                }
            }
            true.into()
        }
        let mut ctx = Ctx { pid, found: vec![] };
        let _ = EnumWindows(Some(cb), LPARAM(&mut ctx as *mut Ctx as isize));
        for h in ctx.found {
            if !IsWindowVisible(h).as_bool() {
                continue;
            }
            let mut buf = [0u16; 64];
            let n = GetClassNameW(h, &mut buf) as usize;
            let class = String::from_utf16_lossy(&buf[..n]);
            if class == "FreeRDP" {
                // Главное окно сеанса оказалось отдельным — встраиваем во вкладку
                let style = GetWindowLongPtrW(h, GWL_STYLE) as u32;
                let style = (style & !(WS_POPUP.0 | WS_CAPTION.0 | WS_THICKFRAME.0 | WS_SYSMENU.0)) | WS_CHILD.0;
                SetWindowLongPtrW(h, GWL_STYLE, style as isize);
                let _ = SetParent(h, Some(host));
                let mut rc = Default::default();
                let _ = GetClientRect(host, &mut rc);
                let _ = MoveWindow(h, 0, 0, rc.right, rc.bottom, true);
            } else {
                let ex = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
                if ex & WS_EX_TOPMOST.0 == 0 {
                    let _ = SetWindowPos(h, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
                    let _ = SetForegroundWindow(h);
                }
            }
        }
    }

    /// Сделать сеанс активным: фокус хосту, он передаст его окну FreeRDP.
    pub fn focus(hwnd: isize) {
        unsafe {
            use windows::Win32::UI::Input::KeyboardAndMouse::{SetActiveWindow, SetFocus};
            let h = HWND(hwnd as _);
            let _ = SetActiveWindow(h);
            let _ = SetFocus(Some(h));
        }
    }

    pub fn destroy(hwnd: isize) {
        unsafe {
            let _ = DestroyWindow(HWND(hwnd as _));
        }
    }
}

fn main_hwnd(app: &AppHandle) -> Result<isize, String> {
    let w = app
        .get_webview_window("main")
        .ok_or("Главное окно не найдено")?;
    #[cfg(windows)]
    {
        Ok(w.hwnd().map_err(|e| e.to_string())?.0 as isize)
    }
    #[cfg(not(windows))]
    {
        let _ = w;
        Err("RDP во вкладке пока поддерживается только в Windows".into())
    }
}

async fn on_main<T: Send + 'static>(
    app: &AppHandle,
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(f());
    })
    .map_err(|e| e.to_string())?;
    rx.await.map_err(|e| e.to_string())
}

/// auth=kerberos — оставить системный Negotiate (Kerberos → NTLM), как у mstsc.
fn use_kerberos(p: &ConnParams) -> bool {
    p.opt("auth").eq_ignore_ascii_case("kerberos")
}

fn build_args(p: &ConnParams, hwnd: isize, w: i32, h: i32) -> Result<Vec<String>, String> {
    let host = match p.port {
        Some(port) => format!("{}:{}", p.host, port),
        None => p.host.clone(),
    };
    let cert = match p.opt("cert") {
        "" => "tofu",
        c => c,
    };
    let mut a: Vec<String> = vec![
        format!("/v:{}", host),
        format!("/title:{}", p.name),
        format!("/cert:{}", cert),
        format!("/parent-window:{}", hwnd),
        "+auto-reconnect".into(),
        // Панель подключения FreeRDP нужна только в полноэкранном режиме
        "/floatbar:sticky:off,default:hidden,show:fullscreen".into(),
    ];
    // Проверка подлинности без Kerberos: вне домена Windows ищет контроллер домена
    // десятки секунд, прежде чем перейти на NTLM. auth=kerberos — как в mstsc.
    if !use_kerberos(p) {
        a.push("/auth-pkg-list:!kerberos".into());
    }
    // Буфер обмена включён по умолчанию (как в mstsc), clipboard=off — выключить
    if p.opt("clipboard").eq_ignore_ascii_case("off") {
        a.push("-clipboard".into());
    } else {
        a.push("/clipboard:direction-to:all,files-to:all".into());
    }
    match p.opt("resolution") {
        "" | "auto" => {
            a.push(format!("/size:{}x{}", w.max(640), h.max(480)));
            a.push("/dynamic-resolution".into());
        }
        r => {
            a.push(format!("/size:{}", r));
            a.push("/smart-sizing".into());
        }
    }
    if !p.username.is_empty() {
        a.push(format!("/u:{}", p.username));
    }
    if !p.domain.is_empty() {
        a.push(format!("/d:{}", p.domain));
    }
    if !p.password.is_empty() {
        a.push(format!("/p:{}", p.password));
    }
    if p.flag("admin") {
        a.push("/admin".into());
    }
    if p.flag("sound") {
        a.push("/sound:sys:winmm".into());
    }
    if p.flag("microphone") {
        a.push("/microphone:sys:winmm".into());
    }
    match p.opt("drives") {
        "" | "0" | "false" | "no" => {}
        "1" | "true" | "yes" | "all" | "да" => a.push("/drives".into()),
        path => a.push(format!("/drive:share,{}", path)),
    }
    if p.flag("printers") {
        a.push("/printer".into());
    }
    if p.flag("smartcard") {
        a.push("/smartcard".into());
    }
    if let Some((addr, gu, gd, gp)) = &p.gateway {
        for (label, v) in [("адрес", addr), ("логин", gu), ("домен", gd), ("пароль", gp)] {
            if v.contains(',') {
                return Err(format!("{} шлюза содержит запятую — FreeRDP так не умеет", label));
            }
        }
        let mut g = format!("/gateway:g:{}", addr);
        if !gu.is_empty() {
            g.push_str(&format!(",u:{}", gu));
        }
        if !gd.is_empty() {
            g.push_str(&format!(",d:{}", gd));
        }
        if !gp.is_empty() {
            g.push_str(&format!(",p:{}", gp));
        }
        // Классический HTTP-транспорт, как у mstsc. С WebSocket FreeRDP на части
        // шлюзов ждёт ~2 минуты, прежде чем откатиться. gwtransport=auto — как в FreeRDP.
        match p.opt("gwtransport") {
            "auto" => {}
            "rpc" => g.push_str(",type:rpc"),
            "websocket" => g.push_str(",type:http"),
            _ => g.push_str(",type:http,no-websockets"),
        }
        a.push(g);
    }
    // Дополнительные параметры FreeRDP для опытных пользователей
    for extra in p.opt("freerdp").split_whitespace() {
        a.push(extra.to_string());
    }
    Ok(a)
}

/// Последние строки ошибок из лога FreeRDP.
fn log_errors(path: &PathBuf) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let errs: Vec<&str> = text
        .lines()
        .filter(|l| l.contains("[ERROR]"))
        .map(|l| l.rsplit(" - ").next().unwrap_or(l).trim())
        .collect();
    let tail: Vec<&str> = errs.iter().rev().take(3).rev().copied().collect();
    let joined = tail.join("\n");
    if joined.contains("AUTHENTICATION_FAILED") || joined.contains("LOGON_FAILURE") {
        return "Неверный логин или пароль".into();
    }
    if joined.contains("CONNECT_TRANSPORT_FAILED") || joined.contains("DNS_NAME_NOT_FOUND") {
        return format!("Сервер недоступен\n{}", joined);
    }
    joined
}

pub async fn open(app: &AppHandle, id: u32, p: ConnParams, x: i32, y: i32, w: i32, h: i32) -> Result<(), String> {
    let parent = main_hwnd(app)?;
    #[cfg(not(windows))]
    {
        let _ = (parent, id, p, x, y, w, h);
        return Err("RDP пока поддерживается только в Windows".into());
    }
    #[cfg(windows)]
    {
        let hwnd = on_main(app, move || win::create(parent)).await??;
        on_main(app, move || {
            win::place(hwnd, parent, x, y, w, h, true);
            win::focus(hwnd);
        })
        .await?;
        let args = build_args(&p, hwnd, w, h)?;

        let log_path = std::env::temp_dir().join(format!("connmgr-rdp-{}-{}.log", std::process::id(), id));
        let log = std::fs::File::create(&log_path).map_err(|e| e.to_string())?;
        let log2 = log.try_clone().map_err(|e| e.to_string())?;
        let exe = FREERDP_EXE.get().cloned().unwrap_or_else(|| PathBuf::from("wfreerdp.exe"));
        let mut cmd = Command::new(&exe);
        // Провайдер OpenSSL legacy (MD4) нужен для NTLM — без него не проходит
        // вход на RD-шлюз и NLA по паролю
        if let Some(dir) = exe.parent().map(|d| d.join("ossl-modules")).filter(|d| d.is_dir()) {
            cmd.env("OPENSSL_MODULES", dir);
        }
        // Этапы подключения в лог — чтобы было видно, где задержка
        cmd.env("WLOG_LEVEL", "INFO");
        // Встроенный NTLM FreeRDP вместо системного SSPI — без задержек на поиск KDC
        if !use_kerberos(&p) {
            cmd.env("WINPR_NATIVE_SSPI", "0");
        }
        cmd.arg("/args-from:stdin")
            .stdin(Stdio::piped())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log2));
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let _ = on_main(app, move || win::destroy(hwnd)).await;
                return Err(format!("Не удалось запустить FreeRDP ({}): {}", exe.display(), e));
            }
        };
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(args.join("\n").as_bytes())
                .map_err(|e| format!("Не удалось передать параметры FreeRDP: {}", e))?;
        }
        let pid = child.id();
        let child = Arc::new(Mutex::new(child));
        app.state::<Rdp>()
            .sessions
            .lock()
            .unwrap()
            .insert(id, RdpSession { hwnd, child: child.clone(), rect: (x, y, w, h, true) });
        emit_status(app, id, "connected", "");

        let app2 = app.clone();
        std::thread::spawn(move || {
            let code = loop {
                if let Ok(Some(st)) = child.lock().unwrap().try_wait() {
                    break st.code().unwrap_or(-1);
                }
                let _ = app2.run_on_main_thread(move || win::fixup(hwnd, pid));
                std::thread::sleep(Duration::from_millis(400));
            };
            let removed = app2.state::<Rdp>().sessions.lock().unwrap().remove(&id);
            let killed = removed.is_none();
            let errors = log_errors(&log_path);
            // Лог последнего сеанса оставляем для диагностики
            let _ = std::fs::rename(&log_path, std::env::temp_dir().join("connmgr-rdp-last.log"));
            let _ = app2.run_on_main_thread(move || win::destroy(hwnd));
            if killed || code == 0 || errors.is_empty() {
                emit_status(&app2, id, "closed", "");
            } else {
                emit_status(&app2, id, "error", errors);
            }
        });
        Ok(())
    }
}

pub fn place(app: &AppHandle, id: u32, x: i32, y: i32, w: i32, h: i32, visible: bool) {
    let hwnd = {
        let st = app.state::<Rdp>();
        let mut g = st.sessions.lock().unwrap();
        g.get_mut(&id).map(|s| {
            let was_visible = s.rect.4;
            s.rect = (x, y, w, h, visible);
            (s.hwnd, visible && !was_visible)
        })
    };
    let owner = main_hwnd(app).unwrap_or(0);
    #[cfg(windows)]
    if let Some((hwnd, shown)) = hwnd {
        let _ = app.run_on_main_thread(move || {
            win::place(hwnd, owner, x, y, w, h, visible);
            // Вкладку RDP открыли/переключились на неё — сразу можно работать мышью и клавиатурой
            if shown {
                win::focus(hwnd);
            }
        });
    }
    #[cfg(not(windows))]
    let _ = (hwnd, owner, x, y, w, h, visible);
}

/// Главное окно сдвинули/развернули — переставить окна сеансов.
pub fn reposition_all(app: &AppHandle) {
    let all: Vec<(isize, (i32, i32, i32, i32, bool))> = app
        .state::<Rdp>()
        .sessions
        .lock()
        .unwrap()
        .values()
        .map(|s| (s.hwnd, s.rect))
        .collect();
    if all.is_empty() {
        return;
    }
    let owner = main_hwnd(app).unwrap_or(0);
    #[cfg(windows)]
    let _ = app.run_on_main_thread(move || {
        for (h, (x, y, w, hh, v)) in all {
            win::place(h, owner, x, y, w, hh, v);
        }
    });
    #[cfg(not(windows))]
    let _ = (all, owner);
}

pub fn close(app: &AppHandle, id: u32) -> bool {
    let s = app.state::<Rdp>().sessions.lock().unwrap().remove(&id);
    match s {
        Some(s) => {
            let _ = s.child.lock().unwrap().kill();
            true
        }
        None => false,
    }
}

pub fn close_all(app: &AppHandle) {
    let all: Vec<RdpSession> = app.state::<Rdp>().sessions.lock().unwrap().drain().map(|(_, s)| s).collect();
    for s in all {
        let _ = s.child.lock().unwrap().kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args() {
        let mut p = ConnParams {
            name: "srv".into(),
            protocol: "rdp".into(),
            host: "10.0.0.5".into(),
            port: Some(3390),
            username: "ivan".into(),
            password: "secret".into(),
            domain: "CORP".into(),
            ..Default::default()
        };
        p.opts.insert("admin".into(), "1".into());
        p.gateway = Some(("gw.corp:443".into(), "g".into(), "".into(), "gp".into()));
        let a = build_args(&p, 1234, 1000, 700).unwrap();
        assert!(a.contains(&"/v:10.0.0.5:3390".to_string()));
        assert!(a.contains(&"/parent-window:1234".to_string()));
        assert!(a.contains(&"/admin".to_string()));
        assert!(a.contains(&"/gateway:g:gw.corp:443,u:g,p:gp,type:http,no-websockets".to_string()));
        assert!(a.contains(&"/dynamic-resolution".to_string()));
    }
}
