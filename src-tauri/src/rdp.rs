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
    pid: u32,
    /// Объект задания Windows с процессом FreeRDP: через него включается/выключается
    /// доступ к буферу обмена, а при закрытии программы FreeRDP завершается вместе с ней
    job: isize,
    /// Буфер обмена для этого подключения разрешён в настройках записи
    clip: bool,
    /// Сейчас доступ к буферу открыт
    clip_on: bool,
    /// Последнее положение в клиентских координатах главного окна и видимость
    rect: (i32, i32, i32, i32, bool),
    /// Сеанс открыт в отдельном окне (не следует за вкладкой)
    detached: bool,
}

/// Для оконной процедуры хоста: доступ к состоянию приложения
static APP: OnceLock<AppHandle> = OnceLock::new();

#[derive(Default)]
pub struct Rdp {
    sessions: Mutex<HashMap<u32, RdpSession>>,
    /// Хранилище заблокировано: окна сеансов скрыты, буфер обмена закрыт
    pub locked: Mutex<bool>,
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
            // Закрытие отдельного окна сеанса — вернуть его во вкладку
            if msg == WM_CLOSE {
                let id = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as u32;
                if let Some(app) = super::APP.get() {
                    super::attach_on_main(app, id);
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
    pub fn create(parent: isize, id: u32) -> Result<isize, String> {
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
            SetWindowLongPtrW(h, GWLP_USERDATA, id as isize);
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

    /// Превратить хост в обычное окно с рамкой (открепление вкладки).
    pub fn detach(hwnd: isize, title: &str) {
        unsafe {
            let h = HWND(hwnd as _);
            let mut wr = Default::default();
            let _ = GetWindowRect(h, &mut wr);
            SetWindowLongPtrW(h, GWLP_HWNDPARENT, 0);
            SetWindowLongPtrW(h, GWL_STYLE, (WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN).0 as isize);
            SetWindowLongPtrW(h, GWL_EXSTYLE, WS_EX_APPWINDOW.0 as isize);
            let t: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
            let _ = SetWindowTextW(h, windows::core::PCWSTR(t.as_ptr()));
            // Размер с учётом рамки: клиентская область остаётся как была
            let mut r = windows::Win32::Foundation::RECT { left: 0, top: 0, right: wr.right - wr.left, bottom: wr.bottom - wr.top };
            let _ = AdjustWindowRectEx(&mut r, WS_OVERLAPPEDWINDOW, false, WS_EX_APPWINDOW);
            let _ = SetWindowPos(
                h,
                Some(HWND_TOP),
                wr.left + 24,
                wr.top + 24,
                r.right - r.left,
                r.bottom - r.top,
                SWP_FRAMECHANGED | SWP_SHOWWINDOW,
            );
            let _ = SetForegroundWindow(h);
            focus(hwnd);
        }
    }

    /// Вернуть хост во вкладку: снова окно без рамки поверх главного.
    pub fn attach(hwnd: isize, owner: isize) {
        unsafe {
            let h = HWND(hwnd as _);
            let _ = ShowWindow(h, SW_HIDE);
            SetWindowLongPtrW(h, GWL_STYLE, (WS_POPUP | WS_CLIPCHILDREN).0 as isize);
            SetWindowLongPtrW(h, GWL_EXSTYLE, WS_EX_TOOLWINDOW.0 as isize);
            SetWindowLongPtrW(h, GWLP_HWNDPARENT, owner);
            let _ = SetWindowPos(h, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
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

    /// Объект задания для процесса FreeRDP (завершить вместе с программой).
    pub fn job_new(process: isize) -> isize {
        use windows::Win32::System::JobObjects::*;
        unsafe {
            let Ok(job) = CreateJobObjectW(None, windows::core::PCWSTR::null()) else { return 0 };
            let mut lim = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            lim.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let _ = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &lim as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if AssignProcessToJobObject(job, windows::Win32::Foundation::HANDLE(process as _)).is_err() {
                let _ = windows::Win32::Foundation::CloseHandle(job);
                return 0;
            }
            job.0 as isize
        }
    }

    /// Разрешить/запретить процессам задания читать и менять буфер обмена.
    pub fn job_clip(job: isize, allow: bool) -> bool {
        use windows::Win32::System::JobObjects::*;
        if job == 0 {
            return false;
        }
        let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
            UIRestrictionsClass: if allow {
                JOB_OBJECT_UILIMIT(0)
            } else {
                JOB_OBJECT_UILIMIT_READCLIPBOARD | JOB_OBJECT_UILIMIT_WRITECLIPBOARD
            },
        };
        unsafe {
            SetInformationJobObject(
                windows::Win32::Foundation::HANDLE(job as _),
                JobObjectBasicUIRestrictions,
                &ui as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32,
            )
            .is_ok()
        }
    }

    pub fn job_close(job: isize) {
        if job != 0 {
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(windows::Win32::Foundation::HANDLE(job as _));
            }
        }
    }

    /// Пока доступ был закрыт, FreeRDP не видел новых копирований — сообщаем ему,
    /// что буфер изменился, чтобы сервер получил текущее содержимое.
    pub fn nudge_clipboard(pid: u32) {
        unsafe {
            let msg_parent = HWND(-3isize as _); // HWND_MESSAGE
            let mut prev: Option<HWND> = None;
            loop {
                let Ok(h) = FindWindowExW(Some(msg_parent), prev, w!("ClipboardHiddenMessageProcessor"), windows::core::PCWSTR::null()) else { break };
                if h.0.is_null() {
                    break;
                }
                let mut wpid = 0u32;
                GetWindowThreadProcessId(h, Some(&mut wpid));
                if wpid == pid {
                    let _ = PostMessageW(Some(h), WM_CLIPBOARDUPDATE, WPARAM(0), LPARAM(0));
                }
                prev = Some(h);
            }
        }
    }

    /// Скрыть/показать окно сеанса (блокировка хранилища, открепленные окна).
    pub fn show(hwnd: isize, visible: bool) {
        unsafe {
            let _ = ShowWindow(HWND(hwnd as _), if visible { SW_SHOWNA } else { SW_HIDE });
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

/// По умолчанию — системный Negotiate (Kerberos → NTLM), как у mstsc.
/// auth=ntlm — только встроенный NTLM FreeRDP (быстрее вне домена, но слабее).
fn use_kerberos(p: &ConnParams) -> bool {
    !p.opt("auth").eq_ignore_ascii_case("ntlm")
}

/// Ключи FreeRDP, которые нельзя задать через поле «freerdp» записи: они ослабляют защиту
/// (отключают проверку сертификата или NLA/TLS, подключают диски, пишут учётные данные в лог)
/// или подменяют то, чем управляет программа. Запись может прийти из общей коллекции.
const DENY_EXTRA: [&str; 24] = [
    "cert", "drive", "drives", "home-drive", "redirect-prefer", "proxy", "log-level", "log-filters",
    "args-from", "p", "password", "u", "d", "v", "gateway", "g", "gp", "gu", "gd", "sec", "tls",
    "auth-pkg-list", "sspi-module", "pth",
];

/// Имя ключа FreeRDP: «/cert:ignore» → «cert», «+drives» → «drives», «-sec-nla» → «sec-nla»
fn extra_key(a: &str) -> String {
    a.trim_start_matches(['/', '+', '-']).split([':', '=']).next().unwrap_or("").to_ascii_lowercase()
}

fn check_extra(a: &str) -> Result<(), String> {
    let k = extra_key(a);
    let base = k.split('-').next().unwrap_or("");
    if DENY_EXTRA.contains(&k.as_str()) || ["sec", "tls", "cert", "auth"].contains(&base) || k.starts_with("drive") {
        return Err(format!(
            "Параметр FreeRDP «{}» нельзя задать в поле «Дополнительно»: он ослабляет защиту подключения или управляется программой",
            a
        ));
    }
    Ok(())
}

/// Параметры уходят в FreeRDP построчно — перевод строки в значении добавил бы чужой ключ.
fn no_newlines(label: &str, v: &str) -> Result<(), String> {
    if v.contains(['\r', '\n', '\0']) {
        return Err(format!("{} содержит перевод строки — такое подключение не запускается", label));
    }
    Ok(())
}

fn build_args(p: &ConnParams, hwnd: isize, w: i32, h: i32, pin: Option<&str>) -> Result<Vec<String>, String> {
    for (label, v) in [
        ("Адрес", &p.host),
        ("Логин", &p.username),
        ("Домен", &p.domain),
        ("Пароль", &p.password),
    ] {
        no_newlines(label, v)?;
    }
    if let Some((a, u, d, pw)) = &p.gateway {
        for (label, v) in [("Адрес шлюза", a), ("Логин шлюза", u), ("Домен шлюза", d), ("Пароль шлюза", pw)] {
            no_newlines(label, v)?;
        }
    }
    let host = match p.port {
        Some(port) => format!("{}:{}", p.host, port),
        None => p.host.clone(),
    };
    // Сертификат: отпечаток, подтверждённый пользователем (rdpcert), иначе запоминание при первом входе
    let cert = match p.opt("cert") {
        "ignore" => "ignore".to_string(),
        _ => pin.unwrap_or("").to_string(),
    };
    let cert = if cert.is_empty() { "tofu".to_string() } else { cert };
    let mut a: Vec<String> = vec![
        format!("/v:{}", host),
        format!("/title:{}", p.name.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>()),
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
        check_extra(extra)?;
        a.push(extra.to_string());
    }
    Ok(a)
}

/// Сеанс завершён пользователем или сервером штатно — это не ошибка.
fn normal_end(text: &str) -> bool {
    [
        "ERRINFO_LOGOFF_BY_USER",
        "ERRINFO_RPC_INITIATED_DISCONNECT_BY_USER",
        "ERRINFO_RPC_INITIATED_LOGOFF",
        "ERRINFO_RPC_INITIATED_DISCONNECT",
        "ERRINFO_DISCONNECTED_BY_OTHERCONNECTION",
    ]
    .iter()
    .any(|c| text.contains(c))
}

/// Последние строки ошибок из вывода FreeRDP.
fn log_errors(text: &str) -> String {
    if normal_end(text) {
        return String::new();
    }
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

/// Проверить сертификаты RD-шлюза и сервера до запуска FreeRDP (с подтверждением пользователя).
/// Возвращает значение /cert: для FreeRDP (None — оставить «tofu»).
async fn cert_check(app: &AppHandle, id: u32, p: &ConnParams) -> Result<Option<String>, String> {
    if p.opt("cert").eq_ignore_ascii_case("ignore") {
        return Ok(None);
    }
    let path = app
        .state::<crate::sessions::Sessions>()
        .known_hosts
        .lock()
        .unwrap()
        .with_file_name("known_rdp");
    // Подтвердить отпечаток: новый — спросить, изменился — предупредить
    let confirm = |what: String, host_id: String, fp: String| {
        let path = path.clone();
        async move {
            let known = crate::rdpcert::known(&path, &host_id);
            if known.as_deref() == Some(fp.as_str()) {
                return Ok(());
            }
            let (title, msg, danger) = match &known {
                Some(old) => (
                    "СЕРТИФИКАТ ИЗМЕНИЛСЯ",
                    format!(
                        "Сертификат {} {} не совпадает с запомненным!

Это бывает при замене сертификата, но может означать перехват соединения — тогда пароль попадёт злоумышленнику.

Был (SHA-256):
{}

Сейчас:
{}

Доверять новому сертификату?",
                        what, host_id, crate::rdpcert::pretty(old), crate::rdpcert::pretty(&fp)
                    ),
                    true,
                ),
                None => (
                    "Новый сервер",
                    format!(
                        "Подключение к {} {} впервые.

Отпечаток сертификата (SHA-256):
{}

Сверьте его с администратором сервера, если есть сомнения. Доверять этому серверу?",
                        what, host_id, crate::rdpcert::pretty(&fp)
                    ),
                    false,
                ),
            };
            if crate::sessions::ask(app, id, "confirm", title, &msg, danger).await.is_none() {
                return Err("Сертификат не принят — подключение отменено".to_string());
            }
            crate::rdpcert::remember(&path, &host_id, &fp);
            Ok::<(), String>(())
        }
    };
    if let Some((addr, ..)) = &p.gateway {
        // Шлюз проверяем сами; сервер за шлюзом напрямую недоступен — для него FreeRDP запомнит
        // сертификат при первом входе (tofu)
        let (h, port) = match addr.rsplit_once(':') {
            Some((h, pt)) if !h.ends_with(':') && pt.parse::<u16>().is_ok() => (h.to_string(), pt.parse().unwrap()),
            _ => (addr.clone(), 443u16),
        };
        let host_id = format!("{}:{}", h, port);
        match crate::rdpcert::fetch(&h, port, false).await {
            Ok(fp) => confirm("RD-шлюза".into(), host_id, fp).await?,
            Err(e) if e.starts_with(crate::rdpcert::TLS_UNSUPPORTED) => {
                unchecked(app, id, &path, "RD-шлюза", &host_id).await?
            }
            Err(e) => return Err(e),
        }
        return Ok(None);
    }
    let port = p.port.unwrap_or(3389);
    let host_id = format!("{}:{}", p.host, port);
    match crate::rdpcert::fetch(&p.host, port, true).await {
        Ok(fp) => {
            confirm("RDP-сервера".into(), host_id, fp.clone()).await?;
            Ok(Some(crate::rdpcert::freerdp_pin(&[&fp])))
        }
        Err(e) if e.starts_with(crate::rdpcert::TLS_UNSUPPORTED) => {
            unchecked(app, id, &path, "RDP-сервера", &host_id).await?;
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// Сервер говорит только на старых версиях TLS/шифрах, сертификат заранее не получить.
/// Один раз спросить и дальше доверить проверку FreeRDP (хранилище сертификатов Windows,
/// иначе — запоминание при первом входе).
async fn unchecked(app: &AppHandle, id: u32, path: &std::path::Path, what: &str, host_id: &str) -> Result<(), String> {
    if crate::rdpcert::known(path, host_id).as_deref() == Some("UNCHECKED") {
        return Ok(());
    }
    let msg = format!(
        "Не удалось заранее проверить сертификат {} {}: сервер использует устаревшие параметры TLS.\n\nСертификат проверит сам FreeRDP — по доверенным центрам сертификации Windows, а самоподписанный запомнит при первом входе.\n\nПодключиться? Вопрос больше не появится для этого адреса.",
        what, host_id
    );
    if crate::sessions::ask(app, id, "confirm", "Сертификат не проверен заранее", &msg, false).await.is_none() {
        return Err("Подключение отменено".into());
    }
    crate::rdpcert::remember(path, host_id, "UNCHECKED");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn open(app: &AppHandle, id: u32, p: ConnParams, x: i32, y: i32, w: i32, h: i32, log_enabled: bool) -> Result<(), String> {
    let pin = cert_check(app, id, &p).await?;
    let parent = main_hwnd(app)?;
    let _ = APP.set(app.clone());
    #[cfg(not(windows))]
    {
        let _ = (parent, id, p, x, y, w, h, log_enabled);
        return Err("RDP пока поддерживается только в Windows".into());
    }
    #[cfg(windows)]
    {
        let hwnd = on_main(app, move || win::create(parent, id)).await??;
        on_main(app, move || {
            win::place(hwnd, parent, x, y, w, h, true);
            win::focus(hwnd);
        })
        .await?;
        let args = build_args(&p, hwnd, w, h, pin.as_deref())?;

        // Журнал на диск — только если включён в настройках (там адреса и логины).
        // Без журнала вывод FreeRDP (только ошибки) держим в памяти — для сообщения об ошибке.
        let last_log = std::env::temp_dir().join("connmgr-rdp-last.log");
        let log_path = std::env::temp_dir().join(format!("connmgr-rdp-{}-{}.log", std::process::id(), id));
        if !log_enabled {
            let _ = std::fs::remove_file(&last_log);
        }
        let exe = FREERDP_EXE.get().cloned().unwrap_or_else(|| PathBuf::from("wfreerdp.exe"));
        let mut cmd = Command::new(&exe);
        // Провайдер OpenSSL legacy (MD4) нужен для NTLM — без него не проходит
        // вход на RD-шлюз и NLA по паролю
        if let Some(dir) = exe.parent().map(|d| d.join("ossl-modules")).filter(|d| d.is_dir()) {
            cmd.env("OPENSSL_MODULES", dir);
        }
        // С журналом — этапы подключения (видно, где задержка); без — только ошибки
        cmd.env("WLOG_LEVEL", if log_enabled { "INFO" } else { "ERROR" });
        // Встроенный NTLM FreeRDP вместо системного SSPI — без задержек на поиск KDC
        if !use_kerberos(&p) {
            cmd.env("WINPR_NATIVE_SSPI", "0");
        }
        cmd.arg("/args-from:stdin").stdin(Stdio::piped());
        if log_enabled {
            let log = std::fs::File::create(&log_path).map_err(|e| e.to_string())?;
            let log2 = log.try_clone().map_err(|e| e.to_string())?;
            cmd.stdout(Stdio::from(log)).stderr(Stdio::from(log2));
        } else {
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        }
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
        let job = {
            use std::os::windows::io::AsRawHandle;
            win::job_new(child.as_raw_handle() as isize)
        };
        // Вывод FreeRDP без журнала: последние строки — в памяти
        let mem_log: Arc<Mutex<std::collections::VecDeque<String>>> = Default::default();
        for pipe in [
            child.stdout.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
            child.stderr.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let buf = mem_log.clone();
            std::thread::spawn(move || {
                use std::io::BufRead;
                for line in std::io::BufReader::new(pipe).lines().map_while(Result::ok) {
                    let mut b = buf.lock().unwrap();
                    b.push_back(line);
                    if b.len() > 200 {
                        b.pop_front();
                    }
                }
            });
        }
        let clip = !p.opt("clipboard").eq_ignore_ascii_case("off");
        let locked = *app.state::<Rdp>().locked.lock().unwrap();
        let child = Arc::new(Mutex::new(child));
        app.state::<Rdp>().sessions.lock().unwrap().insert(
            id,
            RdpSession { hwnd, child: child.clone(), pid, job, clip, clip_on: true, rect: (x, y, w, h, !locked), detached: false },
        );
        // Новая вкладка активна; если хранилище уже заблокировано — сразу скрыть
        sync_clipboard(app);
        if locked {
            let _ = on_main(app, move || win::show(hwnd, false)).await;
        }
        emit_status(app, id, "connected", "");

        let app2 = app.clone();
        let p_auth = p.clone();
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
            if let Some(s) = &removed {
                win::job_close(s.job);
            }
            // Потоки чтения успевают дочитать вывод завершившегося процесса
            std::thread::sleep(Duration::from_millis(150));
            let text = if log_enabled {
                let t = std::fs::read_to_string(&log_path).unwrap_or_default();
                // Журнал последнего сеанса оставляем для диагностики (включён в настройках)
                let _ = std::fs::rename(&log_path, &last_log);
                t
            } else {
                mem_log.lock().unwrap().iter().cloned().collect::<Vec<_>>().join("\n")
            };
            let mut errors = log_errors(&text);
            // Kerberos не договорился (сервер вне домена, нет KDC) — интерфейс повторит через NTLM,
            // как делает mstsc. Только если Kerberos не выбран в записи явно.
            if use_kerberos(&p_auth) && !p_auth.opt("auth").eq_ignore_ascii_case("kerberos") && !errors.is_empty()
                && (errors.contains("SECURITY_NEGO_CONNECT_FAILED") || errors.contains("NEGO_FAILURE")
                    || errors.to_ascii_lowercase().contains("kerberos"))
            {
                errors = format!("RDP_RETRY_NTLM\n{}", errors);
            }
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
    let locked = *app.state::<Rdp>().locked.lock().unwrap();
    let visible = visible && !locked;
    let hwnd = {
        let st = app.state::<Rdp>();
        let mut g = st.sessions.lock().unwrap();
        g.get_mut(&id).and_then(|s| {
            let was_visible = s.rect.4;
            s.rect = (x, y, w, h, visible);
            // Открепленное окно живёт само по себе
            (!s.detached).then_some((s.hwnd, visible && !was_visible))
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
    sync_clipboard(app);
}

/// Буфер обмена открыт только активному сеансу: видимой вкладке или открепленному окну,
/// и только пока хранилище не заблокировано. Остальные сеансы буфер не видят и не меняют.
fn sync_clipboard(app: &AppHandle) {
    let locked = *app.state::<Rdp>().locked.lock().unwrap();
    let st = app.state::<Rdp>();
    let mut g = st.sessions.lock().unwrap();
    for s in g.values_mut() {
        let allow = s.clip && !locked && (s.detached || s.rect.4);
        if allow != s.clip_on {
            #[cfg(windows)]
            {
                win::job_clip(s.job, allow);
                if allow {
                    win::nudge_clipboard(s.pid);
                }
            }
            s.clip_on = allow;
        }
    }
}

/// Блокировка хранилища: скрыть все окна сеансов (и открепленные), закрыть им буфер обмена.
/// Разблокировка — вернуть как было.
pub fn set_locked(app: &AppHandle, locked: bool) {
    *app.state::<Rdp>().locked.lock().unwrap() = locked;
    if locked {
        crate::web::hide_all(app);
    }
    let all: Vec<(isize, bool, (i32, i32, i32, i32, bool))> = app
        .state::<Rdp>()
        .sessions
        .lock()
        .unwrap()
        .values()
        .map(|s| (s.hwnd, s.detached, s.rect))
        .collect();
    sync_clipboard(app);
    let owner = main_hwnd(app).unwrap_or(0);
    #[cfg(windows)]
    let _ = app.run_on_main_thread(move || {
        for (h, detached, (x, y, w, hh, v)) in all {
            if locked {
                win::show(h, false);
            } else if detached {
                win::show(h, true);
            } else {
                win::place(h, owner, x, y, w, hh, v);
            }
        }
    });
    #[cfg(not(windows))]
    let _ = (all, owner);
}

/// Открепить сеанс в отдельное окно.
pub fn detach(app: &AppHandle, id: u32, title: String) -> Result<(), String> {
    let hwnd = {
        let st = app.state::<Rdp>();
        let mut g = st.sessions.lock().unwrap();
        let s = g.get_mut(&id).ok_or("Сеанс не найден")?;
        s.detached = true;
        s.hwnd
    };
    #[cfg(windows)]
    let _ = app.run_on_main_thread(move || win::detach(hwnd, &title));
    #[cfg(not(windows))]
    let _ = (hwnd, title);
    sync_clipboard(app);
    Ok(())
}

/// Вернуть сеанс во вкладку (из команды интерфейса).
pub fn attach(app: &AppHandle, id: u32) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || attach_on_main(&app2, id));
}

/// Вызывать в главном потоке.
fn attach_on_main(app: &AppHandle, id: u32) {
    let found = {
        let st = app.state::<Rdp>();
        let mut g = st.sessions.lock().unwrap();
        g.get_mut(&id).filter(|s| s.detached).map(|s| {
            s.detached = false;
            (s.hwnd, s.rect)
        })
    };
    let Some((hwnd, (x, y, w, h, visible))) = found else { return };
    let owner = main_hwnd(app).unwrap_or(0);
    #[cfg(windows)]
    {
        win::attach(hwnd, owner);
        win::place(hwnd, owner, x, y, w, h, visible);
        if visible {
            win::focus(hwnd);
        }
    }
    #[cfg(not(windows))]
    let _ = (hwnd, owner, x, y, w, h, visible);
    use tauri::Emitter;
    let _ = app.emit("rdp-docked", id);
    sync_clipboard(app);
}

/// Главное окно сдвинули/развернули — переставить окна сеансов.
pub fn reposition_all(app: &AppHandle) {
    let all: Vec<(isize, (i32, i32, i32, i32, bool))> = app
        .state::<Rdp>()
        .sessions
        .lock()
        .unwrap()
        .values()
        .filter(|s| !s.detached)
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
    fn injection_and_unsafe_extras_rejected() {
        let base = ConnParams { name: "a\nb".into(), protocol: "rdp".into(), host: "h".into(), ..Default::default() };
        // имя с переводом строки — просто заголовок окна, без новой строки
        let a = build_args(&base, 1, 800, 600, None).unwrap();
        assert!(a.iter().all(|x| !x.contains('\n')));
        for (field, val) in [("password", "x\n/drives"), ("username", "u\r\n/cert:ignore"), ("host", "h\n/proxy:http://evil")] {
            let mut p = base.clone();
            match field {
                "password" => p.password = val.into(),
                "username" => p.username = val.into(),
                _ => p.host = val.into(),
            }
            assert!(build_args(&p, 1, 800, 600, None).is_err(), "{} не отклонён", field);
        }
        for bad in ["/cert:ignore", "+drives", "/drive:c,C:\\", "/sec:rdp", "-sec-nla", "/log-level:TRACE", "/proxy:http://x", "/tls:seclevel:0", "/p:x"] {
            let mut p = base.clone();
            p.opts.insert("freerdp".into(), bad.into());
            assert!(build_args(&p, 1, 800, 600, None).is_err(), "{} пропущен", bad);
        }
        let mut p = base.clone();
        p.opts.insert("freerdp".into(), "/network:lan +fonts /bpp:32".into());
        assert!(build_args(&p, 1, 800, 600, None).is_ok());
        let a = build_args(&base, 1, 800, 600, Some("deny,fingerprint:sha256:AB")).unwrap();
        assert!(a.contains(&"/cert:deny,fingerprint:sha256:AB".to_string()));
    }

    #[test]
    fn logoff_is_not_an_error() {
        let logoff = "[ERROR][com.freerdp.core] - [rdp_set_error_info]: ERRINFO_LOGOFF_BY_USER [0x0001000C]";
        assert_eq!(log_errors(logoff), "");
        let fail = "[ERROR][com.freerdp.core] - [nla_recv_pdu]: ERRCONNECT_LOGON_FAILURE [0x00020014]";
        assert_eq!(log_errors(fail), "Неверный логин или пароль");
        let p = ConnParams { protocol: "rdp".into(), host: "h".into(), ..Default::default() };
        assert!(use_kerberos(&p), "Kerberos по умолчанию");
        let mut n = p.clone();
        n.opts.insert("auth".into(), "ntlm".into());
        assert!(!use_kerberos(&n));
    }

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
        let a = build_args(&p, 1234, 1000, 700, None).unwrap();
        assert!(a.contains(&"/v:10.0.0.5:3390".to_string()));
        assert!(a.contains(&"/parent-window:1234".to_string()));
        assert!(a.contains(&"/admin".to_string()));
        assert!(a.contains(&"/gateway:g:gw.corp:443,u:g,p:gp,type:http,no-websockets".to_string()));
        assert!(a.contains(&"/dynamic-resolution".to_string()));
    }
}
