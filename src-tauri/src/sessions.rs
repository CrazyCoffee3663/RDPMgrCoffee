//! Терминальные сессии (SSH, Telnet, COM-порт) и общий менеджер сессий.
//! Вывод уходит в интерфейс сырыми байтами через ipc::Channel, ввод приходит командами.

use crate::conn::ConnParams;
use russh::client;
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::ChannelMsg;
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

pub enum TermCmd {
    Data(Vec<u8>),
    Resize(u32, u32),
    Close,
}

#[derive(Default)]
pub struct Sessions {
    next_id: AtomicU32,
    next_prompt: AtomicU64,
    terms: Mutex<HashMap<u32, mpsc::UnboundedSender<TermCmd>>>,
    prompts: Mutex<HashMap<u64, oneshot::Sender<Option<String>>>>,
    pub known_hosts: Mutex<PathBuf>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct StatusEvent {
    id: u32,
    /// connecting | connected | closed | error
    state: String,
    message: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PromptEvent {
    id: u64,
    session: u32,
    /// confirm | secret | text
    kind: String,
    title: String,
    message: String,
    danger: bool,
}

pub fn emit_status(app: &AppHandle, id: u32, state: &str, message: impl Into<String>) {
    let _ = app.emit(
        "session-status",
        StatusEvent {
            id,
            state: state.into(),
            message: message.into(),
        },
    );
}

impl Sessions {
    pub fn alloc_id(&self) -> u32 {
        self.next_id.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn send(&self, id: u32, cmd: TermCmd) -> bool {
        match self.terms.lock().unwrap().get(&id) {
            Some(tx) => tx.send(cmd).is_ok(),
            None => false,
        }
    }

    pub fn answer(&self, prompt: u64, value: Option<String>) {
        if let Some(tx) = self.prompts.lock().unwrap().remove(&prompt) {
            let _ = tx.send(value);
        }
    }

    pub fn close_all(&self) {
        for (_, tx) in self.terms.lock().unwrap().drain() {
            let _ = tx.send(TermCmd::Close);
        }
    }
}

/// Спросить пользователя (диалог в интерфейсе). None — отмена.
async fn ask(
    app: &AppHandle,
    session: u32,
    kind: &str,
    title: &str,
    message: &str,
    danger: bool,
) -> Option<String> {
    let st = app.state::<Sessions>();
    let id = st.next_prompt.fetch_add(1, Ordering::Relaxed) + 1;
    let (tx, rx) = oneshot::channel();
    st.prompts.lock().unwrap().insert(id, tx);
    let _ = app.emit(
        "prompt",
        PromptEvent {
            id,
            session,
            kind: kind.into(),
            title: title.into(),
            message: message.into(),
            danger,
        },
    );
    match tokio::time::timeout(Duration::from_secs(600), rx).await {
        Ok(Ok(v)) => v,
        _ => {
            st.prompts.lock().unwrap().remove(&id);
            None
        }
    }
}

fn send_out(out: &Channel<InvokeResponseBody>, data: &[u8]) {
    let _ = out.send(InvokeResponseBody::Raw(data.to_vec()));
}

fn send_text(out: &Channel<InvokeResponseBody>, text: &str) {
    send_out(out, text.replace('\n', "\r\n").as_bytes());
}

/// Запустить терминальную сессию в фоне.
pub fn start_term(
    app: AppHandle,
    id: u32,
    p: ConnParams,
    cols: u32,
    rows: u32,
    out: Channel<InvokeResponseBody>,
) {
    let (tx, rx) = mpsc::unbounded_channel();
    app.state::<Sessions>().terms.lock().unwrap().insert(id, tx);
    tauri::async_runtime::spawn(async move {
        emit_status(&app, id, "connecting", "");
        let res = match p.protocol.as_str() {
            "ssh" => run_ssh(&app, id, &p, cols, rows, &out, rx).await,
            "telnet" => run_telnet(&app, id, &p, cols, rows, &out, rx).await,
            "serial" => run_serial(&app, id, &p, &out, rx).await,
            other => Err(format!("Протокол {} не поддерживается терминалом", other)),
        };
        app.state::<Sessions>().terms.lock().unwrap().remove(&id);
        match res {
            Ok(()) => {
                send_text(&out, "\n\x1b[90m[Соединение закрыто]\x1b[0m\n");
                emit_status(&app, id, "closed", "");
            }
            Err(e) => {
                send_text(&out, &format!("\n\x1b[31m{}\x1b[0m\n", e));
                emit_status(&app, id, "error", e);
            }
        }
    });
}

// ---------------- SSH ----------------

struct SshHandler {
    app: AppHandle,
    sid: u32,
    host_id: String,
}

fn known_hosts_check(path: &PathBuf, host_id: &str, key_line: &str) -> (bool, bool) {
    // (найден точно такой ключ, есть другой ключ для хоста)
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut other = false;
    for line in text.lines() {
        if let Some((h, k)) = line.split_once(' ') {
            if h == host_id {
                if k.trim() == key_line {
                    return (true, false);
                }
                other = true;
            }
        }
    }
    (false, other)
}

fn known_hosts_store(path: &PathBuf, host_id: &str, key_line: &str) {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut lines: Vec<String> = text
        .lines()
        .filter(|l| l.split_once(' ').map(|(h, _)| h != host_id).unwrap_or(true))
        .map(String::from)
        .collect();
    lines.push(format!("{} {}", host_id, key_line));
    let _ = std::fs::write(path, lines.join("\n") + "\n");
}

impl client::Handler for SshHandler {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let kd = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.key_data().clone(),
            PublicKeyOrCertificate::Certificate(c) => c.public_key().clone(),
        };
        let pk = russh::keys::PublicKey::new(kd, "");
        let line = pk.to_openssh().unwrap_or_default();
        let fp = pk.fingerprint(HashAlg::Sha256).to_string();
        let path = self.app.state::<Sessions>().known_hosts.lock().unwrap().clone();
        let (known, changed) = known_hosts_check(&path, &self.host_id, &line);
        if known {
            return Ok(true);
        }
        let (title, msg, danger) = if changed {
            (
                "Ключ сервера ИЗМЕНИЛСЯ",
                format!(
                    "Ключ хоста {} не совпадает с сохранённым.\nЭто может быть атака «человек посередине» или переустановка сервера.\n\nНовый отпечаток:\n{}\n\nДоверять новому ключу?",
                    self.host_id, fp
                ),
                true,
            )
        } else {
            (
                "Новый сервер",
                format!(
                    "Подключение к {} впервые.\n\nОтпечаток ключа ({}):\n{}\n\nДоверять этому серверу?",
                    self.host_id,
                    pk.algorithm().as_str(),
                    fp
                ),
                false,
            )
        };
        let ok = ask(&self.app, self.sid, "confirm", title, &msg, danger).await.is_some();
        if ok {
            known_hosts_store(&path, &self.host_id, &line);
        }
        Ok(ok)
    }
}

async fn run_ssh(
    app: &AppHandle,
    id: u32,
    p: &ConnParams,
    cols: u32,
    rows: u32,
    out: &Channel<InvokeResponseBody>,
    mut rx: mpsc::UnboundedReceiver<TermCmd>,
) -> Result<(), String> {
    let port = p.port.unwrap_or(22);
    let keepalive: u64 = p.opt("keepalive").parse().unwrap_or(30);
    let config = Arc::new(client::Config {
        inactivity_timeout: None,
        keepalive_interval: if keepalive > 0 { Some(Duration::from_secs(keepalive)) } else { None },
        keepalive_max: 4,
        nodelay: true,
        ..Default::default()
    });
    let host_id = if port == 22 { p.host.clone() } else { format!("[{}]:{}", p.host, port) };
    send_text(out, &format!("\x1b[90mПодключение к {}:{}…\x1b[0m\n", p.host, port));
    let handler = SshHandler {
        app: app.clone(),
        sid: id,
        host_id,
    };
    let mut h = tokio::time::timeout(
        Duration::from_secs(20),
        client::connect(config, (p.host.as_str(), port), handler),
    )
    .await
    .map_err(|_| "Превышено время ожидания подключения".to_string())?
    .map_err(|e| match e {
        russh::Error::UnknownKey => "Ключ сервера не принят".to_string(),
        e => format!("Не удалось подключиться: {}", e),
    })?;

    // Логин
    let user = if p.username.is_empty() {
        ask(app, id, "text", "Вход по SSH", &format!("Логин для {}:", p.host), false)
            .await
            .ok_or("Отменено")?
    } else {
        p.username.clone()
    };

    let mut ok = false;
    // 1. Ключ
    if !p.ssh_private_key.trim().is_empty() {
        let key = russh::keys::decode_secret_key(p.ssh_private_key.trim(), None)
            .map_err(|e| format!("Не удалось прочитать SSH-ключ: {}", e))?;
        let hash = h.best_supported_rsa_hash().await.ok().flatten().flatten();
        ok = h
            .authenticate_publickey(&user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
            .await
            .map_err(|e| e.to_string())?
            .success();
    }
    // 2. Пароль
    if !ok && !p.password.is_empty() {
        ok = h
            .authenticate_password(&user, &p.password)
            .await
            .map_err(|e| e.to_string())?
            .success();
    }
    // 3. keyboard-interactive (OTP и т.п.)
    if !ok {
        let mut used_pw = p.password.is_empty();
        let mut resp = h
            .authenticate_keyboard_interactive_start(&user, None::<String>)
            .await
            .map_err(|e| e.to_string())?;
        for _ in 0..5 {
            match resp {
                client::KeyboardInteractiveAuthResponse::Success => {
                    ok = true;
                    break;
                }
                client::KeyboardInteractiveAuthResponse::Failure { .. } => break,
                client::KeyboardInteractiveAuthResponse::InfoRequest { name, instructions, prompts } => {
                    let mut answers = Vec::new();
                    for pr in &prompts {
                        let low = pr.prompt.to_lowercase();
                        if !used_pw && (low.contains("password") || low.contains("пароль")) {
                            answers.push(p.password.clone());
                            used_pw = true;
                            continue;
                        }
                        let title = if name.is_empty() { "Вход по SSH" } else { name.as_str() };
                        let msg = format!("{}\n{}", instructions, pr.prompt).trim().to_string();
                        let a = ask(app, id, if pr.echo { "text" } else { "secret" }, title, &msg, false)
                            .await
                            .ok_or("Отменено")?;
                        answers.push(a);
                    }
                    resp = h
                        .authenticate_keyboard_interactive_respond(answers)
                        .await
                        .map_err(|e| e.to_string())?;
                }
            }
        }
    }
    // 4. Спросить пароль
    let mut tries = 0;
    while !ok && tries < 3 {
        tries += 1;
        let msg = if tries == 1 && p.password.is_empty() {
            format!("Пароль для {}@{}:", user, p.host)
        } else {
            format!("Неверный пароль. Пароль для {}@{}:", user, p.host)
        };
        let Some(pw) = ask(app, id, "secret", "Вход по SSH", &msg, false).await else { break };
        ok = h
            .authenticate_password(&user, pw)
            .await
            .map_err(|e| e.to_string())?
            .success();
    }
    if !ok {
        return Err("Аутентификация не удалась".into());
    }

    let mut ch = h.channel_open_session().await.map_err(|e| e.to_string())?;
    ch.request_pty(false, "xterm-256color", cols, rows, 0, 0, &[])
        .await
        .map_err(|e| e.to_string())?;
    ch.request_shell(false).await.map_err(|e| e.to_string())?;
    emit_status(app, id, "connected", "");

    loop {
        tokio::select! {
            msg = ch.wait() => match msg {
                Some(ChannelMsg::Data { data }) => send_out(out, &data),
                Some(ChannelMsg::ExtendedData { data, .. }) => send_out(out, &data),
                Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => break,
                _ => {}
            },
            cmd = rx.recv() => match cmd {
                Some(TermCmd::Data(d)) => { let _ = ch.data_bytes(d).await; }
                Some(TermCmd::Resize(c, r)) => { let _ = ch.window_change(c, r, 0, 0).await; }
                Some(TermCmd::Close) | None => {
                    let _ = ch.close().await;
                    break;
                }
            }
        }
    }
    let _ = h
        .disconnect(russh::Disconnect::ByApplication, "", "en")
        .await;
    Ok(())
}

// ---------------- Telnet ----------------

const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;
const OPT_ECHO: u8 = 1;
const OPT_SGA: u8 = 3;
const OPT_TTYPE: u8 = 24;
const OPT_NAWS: u8 = 31;

fn naws(cols: u32, rows: u32) -> Vec<u8> {
    let mut v = vec![IAC, SB, OPT_NAWS];
    for b in [(cols >> 8) as u8, cols as u8, (rows >> 8) as u8, rows as u8] {
        v.push(b);
        if b == IAC {
            v.push(IAC);
        }
    }
    v.extend([IAC, SE]);
    v
}

/// Разбор потока telnet: возвращает полезные данные и ответы серверу.
struct TelnetParser {
    state: u8, // 0 данные, 1 после IAC, 2 после WILL/WONT/DO/DONT, 3 в SB, 4 IAC в SB
    verb: u8,
    sb: Vec<u8>,
    naws_on: bool,
    answered: std::collections::HashSet<(u8, u8)>,
}

impl TelnetParser {
    fn new() -> Self {
        Self { state: 0, verb: 0, sb: vec![], naws_on: false, answered: Default::default() }
    }

    fn feed(&mut self, input: &[u8], cols: u32, rows: u32, data: &mut Vec<u8>, reply: &mut Vec<u8>) {
        for &b in input {
            match self.state {
                0 => {
                    if b == IAC {
                        self.state = 1
                    } else {
                        data.push(b)
                    }
                }
                1 => match b {
                    IAC => {
                        data.push(IAC);
                        self.state = 0
                    }
                    WILL | WONT | DO | DONT => {
                        self.verb = b;
                        self.state = 2
                    }
                    SB => {
                        self.sb.clear();
                        self.state = 3
                    }
                    _ => self.state = 0,
                },
                2 => {
                    self.state = 0;
                    let opt = b;
                    if !self.answered.insert((self.verb, opt)) {
                        continue;
                    }
                    match self.verb {
                        WILL => {
                            let ok = opt == OPT_ECHO || opt == OPT_SGA;
                            reply.extend([IAC, if ok { DO } else { DONT }, opt]);
                        }
                        DO => match opt {
                            OPT_TTYPE | OPT_SGA => reply.extend([IAC, WILL, opt]),
                            OPT_NAWS => {
                                reply.extend([IAC, WILL, opt]);
                                reply.extend(naws(cols, rows));
                                self.naws_on = true;
                            }
                            _ => reply.extend([IAC, WONT, opt]),
                        },
                        _ => {}
                    }
                }
                3 => {
                    if b == IAC {
                        self.state = 4
                    } else {
                        self.sb.push(b)
                    }
                }
                _ => {
                    if b == SE {
                        self.state = 0;
                        // Запрос типа терминала: SB TTYPE SEND
                        if self.sb.first() == Some(&OPT_TTYPE) && self.sb.get(1) == Some(&1) {
                            reply.extend([IAC, SB, OPT_TTYPE, 0]);
                            reply.extend(b"XTERM-256COLOR");
                            reply.extend([IAC, SE]);
                        }
                    } else {
                        if b == IAC {
                            self.sb.push(IAC)
                        }
                        self.state = 3
                    }
                }
            }
        }
    }
}

async fn run_telnet(
    app: &AppHandle,
    id: u32,
    p: &ConnParams,
    mut cols: u32,
    mut rows: u32,
    out: &Channel<InvokeResponseBody>,
    mut rx: mpsc::UnboundedReceiver<TermCmd>,
) -> Result<(), String> {
    let port = p.port.unwrap_or(23);
    send_text(out, &format!("\x1b[90mПодключение к {}:{}…\x1b[0m\n", p.host, port));
    let stream = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::net::TcpStream::connect((p.host.as_str(), port)),
    )
    .await
    .map_err(|_| "Превышено время ожидания подключения".to_string())?
    .map_err(|e| format!("Не удалось подключиться: {}", e))?;
    let _ = stream.set_nodelay(true);
    let (mut rd, mut wr) = stream.into_split();
    emit_status(app, id, "connected", "");

    let mut parser = TelnetParser::new();
    let mut buf = vec![0u8; 16384];
    // Автоввод логина и пароля по приглашению
    let mut tail = String::new();
    let mut sent_user = p.username.is_empty();
    let mut sent_pw = p.password.is_empty();
    loop {
        tokio::select! {
            n = rd.read(&mut buf) => {
                let n = n.map_err(|e| e.to_string())?;
                if n == 0 { break; }
                let mut data = Vec::new();
                let mut reply = Vec::new();
                parser.feed(&buf[..n], cols, rows, &mut data, &mut reply);
                if !reply.is_empty() { wr.write_all(&reply).await.map_err(|e| e.to_string())?; }
                if !data.is_empty() {
                    send_out(out, &data);
                    if !(sent_user && sent_pw) {
                        tail.push_str(&String::from_utf8_lossy(&data).to_lowercase());
                        if tail.len() > 200 { tail = tail[tail.len() - 100..].to_string(); }
                        let t = tail.trim_end();
                        if !sent_user && (t.ends_with("login:") || t.ends_with("username:")) {
                            wr.write_all(format!("{}\r\n", p.username).as_bytes()).await.map_err(|e| e.to_string())?;
                            sent_user = true;
                            tail.clear();
                        } else if !sent_pw && (t.ends_with("password:") || t.ends_with("пароль:")) {
                            wr.write_all(format!("{}\r\n", p.password).as_bytes()).await.map_err(|e| e.to_string())?;
                            sent_pw = true;
                            tail.clear();
                        }
                    }
                }
            }
            cmd = rx.recv() => match cmd {
                Some(TermCmd::Data(d)) => {
                    let mut v = Vec::with_capacity(d.len() + 4);
                    let mut i = 0;
                    while i < d.len() {
                        let b = d[i];
                        if b == IAC { v.extend([IAC, IAC]); }
                        else if b == b'\r' && d.get(i + 1) != Some(&b'\n') { v.extend(b"\r\n"); }
                        else { v.push(b); }
                        i += 1;
                    }
                    wr.write_all(&v).await.map_err(|e| e.to_string())?;
                }
                Some(TermCmd::Resize(c, r)) => {
                    cols = c; rows = r;
                    if parser.naws_on { let _ = wr.write_all(&naws(c, r)).await; }
                }
                Some(TermCmd::Close) | None => break,
            }
        }
    }
    Ok(())
}

// ---------------- COM-порт ----------------

fn parse_sercfg(p: &ConnParams) -> (u32, serialport::DataBits, serialport::Parity, serialport::StopBits, serialport::FlowControl) {
    use serialport::*;
    let cfg = p.opt("sercfg").to_string();
    let parts: Vec<String> = cfg.split(',').map(|s| s.trim().to_lowercase()).collect();
    let baud = parts
        .first()
        .and_then(|b| b.parse().ok())
        .or_else(|| p.opt("baud").parse().ok())
        .unwrap_or(9600);
    let data = match parts.get(1).map(|s| s.as_str()) {
        Some("5") => DataBits::Five,
        Some("6") => DataBits::Six,
        Some("7") => DataBits::Seven,
        _ => DataBits::Eight,
    };
    let parity = match parts.get(2).map(|s| s.as_str()) {
        Some("o") => Parity::Odd,
        Some("e") => Parity::Even,
        _ => Parity::None,
    };
    let stop = match parts.get(3).map(|s| s.as_str()) {
        Some("2") => StopBits::Two,
        _ => StopBits::One,
    };
    let flow = match parts.get(4).map(|s| s.as_str()) {
        Some("x") => FlowControl::Software,
        Some("r") | Some("h") => FlowControl::Hardware,
        _ => FlowControl::None,
    };
    (baud, data, parity, stop, flow)
}

async fn run_serial(
    app: &AppHandle,
    id: u32,
    p: &ConnParams,
    out: &Channel<InvokeResponseBody>,
    mut rx: mpsc::UnboundedReceiver<TermCmd>,
) -> Result<(), String> {
    let (baud, data, parity, stop, flow) = parse_sercfg(p);
    let port = serialport::new(&p.host, baud)
        .data_bits(data)
        .parity(parity)
        .stop_bits(stop)
        .flow_control(flow)
        .timeout(Duration::from_millis(50))
        .open()
        .map_err(|e| format!("Не удалось открыть {}: {}", p.host, e))?;
    let mut reader = port.try_clone().map_err(|e| e.to_string())?;
    let writer = Arc::new(Mutex::new(port));
    send_text(out, &format!("\x1b[90m{} открыт, {} бод\x1b[0m\n", p.host, baud));
    emit_status(app, id, "connected", "");

    let stop_flag = Arc::new(AtomicBool::new(false));
    let sf = stop_flag.clone();
    let out2 = out.clone();
    let (done_tx, mut done_rx) = oneshot::channel::<String>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut err = String::new();
        while !sf.load(Ordering::Relaxed) {
            match reader.read(&mut buf) {
                Ok(0) => {}
                Ok(n) => send_out(&out2, &buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) => {
                    err = e.to_string();
                    break;
                }
            }
        }
        let _ = done_tx.send(err);
    });

    let result = loop {
        tokio::select! {
            r = &mut done_rx => {
                let e = r.unwrap_or_default();
                break if e.is_empty() { Ok(()) } else { Err(format!("Ошибка порта: {}", e)) };
            }
            cmd = rx.recv() => match cmd {
                Some(TermCmd::Data(d)) => {
                    if let Err(e) = writer.lock().unwrap().write_all(&d) {
                        break Err(format!("Ошибка записи в порт: {}", e));
                    }
                }
                Some(TermCmd::Resize(..)) => {}
                Some(TermCmd::Close) | None => break Ok(()),
            }
        }
    };
    stop_flag.store(true, Ordering::Relaxed);
    result
}

pub fn serial_ports() -> Vec<String> {
    serialport::available_ports()
        .map(|v| v.into_iter().map(|p| p.port_name).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telnet_negotiation() {
        let mut p = TelnetParser::new();
        let mut data = vec![];
        let mut reply = vec![];
        p.feed(&[b'h', IAC, DO, OPT_NAWS, IAC, WILL, OPT_ECHO, IAC, IAC, b'i'], 80, 24, &mut data, &mut reply);
        assert_eq!(data, vec![b'h', IAC, b'i']);
        assert_eq!(&reply[..3], &[IAC, WILL, OPT_NAWS]);
        assert!(p.naws_on);
        assert!(reply.ends_with(&[IAC, DO, OPT_ECHO]));
        // Повторный запрос не вызывает повторного ответа
        let mut r2 = vec![];
        p.feed(&[IAC, DO, OPT_NAWS], 80, 24, &mut data, &mut r2);
        assert!(r2.is_empty());
        // TTYPE SEND
        let mut r3 = vec![];
        p.feed(&[IAC, SB, OPT_TTYPE, 1, IAC, SE], 80, 24, &mut data, &mut r3);
        assert!(r3.windows(14).any(|w| w == b"XTERM-256COLOR"));
    }
}
