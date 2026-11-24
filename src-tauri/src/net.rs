//! Проверка доступности хостов.
//!
//! Одного TCP-рукопожатия мало: VPN/прокси в режиме TUN и некоторые антивирусы принимают
//! соединение к любому адресу сами. Поэтому хост считается доступным, только если он
//! ответил по своему протоколу: SSH присылает баннер, RDP отвечает на X.224 Connection
//! Request, HTTPS (RD-шлюз) — на TLS ClientHello. Без авторизации, соединение сразу закрывается.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;

#[derive(Deserialize)]
pub struct Target {
    pub id: String,
    pub host: String,
    pub port: u16,
    /// rdp | ssh | tls | tcp (по умолчанию tcp)
    #[serde(default)]
    pub kind: String,
}

#[derive(Serialize)]
pub struct Reach {
    pub id: String,
    pub up: bool,
}

/// X.224 Connection Request с RDP_NEG_REQ (TLS | CredSSP)
const RDP_CR: [u8; 19] = [
    0x03, 0x00, 0x00, 0x13, 0x0e, 0xe0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x08, 0x00, 0x03, 0x00, 0x00, 0x00,
];

/// Минимальный TLS 1.2 ClientHello. Любой ответ (ServerHello или alert) значит, что сервер жив.
fn tls_hello() -> Vec<u8> {
    let mut body = vec![0x03, 0x03];
    body.extend((0..32u8).map(|i| i.wrapping_mul(37).wrapping_add(11)));
    body.push(0); // session id
    let suites: [u16; 6] = [0xc02f, 0xc030, 0xc02b, 0xc02c, 0x009c, 0x003c];
    body.extend(((suites.len() * 2) as u16).to_be_bytes());
    for s in suites {
        body.extend(s.to_be_bytes());
    }
    body.extend([1, 0]); // compression: null
    let mut hs = vec![0x01, 0, (body.len() >> 8) as u8, body.len() as u8];
    hs.extend(body);
    let mut rec = vec![0x16, 0x03, 0x01];
    rec.extend((hs.len() as u16).to_be_bytes());
    rec.extend(hs);
    rec
}

/// ClientHello старого TLS (1.0/1.1) с классическими шифрами: ответ сервера
/// (ServerHello 0x16 или alert 0x15) показывает, говорит ли он на этой версии.
fn old_tls_hello(ver: [u8; 2]) -> Vec<u8> {
    let mut body = ver.to_vec();
    body.extend((0..32u8).map(|i| i.wrapping_mul(37).wrapping_add(11)));
    body.push(0); // session id
    let suites: [u16; 5] = [0x002f, 0x0035, 0x000a, 0x0005, 0x00ff];
    body.extend(((suites.len() * 2) as u16).to_be_bytes());
    for s in suites {
        body.extend(s.to_be_bytes());
    }
    body.extend([1, 0]); // compression: null
    let mut hs = vec![0x01, 0, (body.len() >> 8) as u8, body.len() as u8];
    hs.extend(body);
    let mut rec = vec![0x16, ver[0], ver[1]];
    rec.extend((hs.len() as u16).to_be_bytes());
    rec.extend(hs);
    rec
}

/// ServerHello на ClientHello данной версии — сервер её принимает.
async fn old_tls_accepts(addr: &str, ver: [u8; 2], rdp: bool) -> bool {
    let timeout = Duration::from_millis(1500);
    let Ok(Ok(mut s)) = tokio::time::timeout(timeout, TcpStream::connect(addr)).await else {
        return false;
    };
    if rdp {
        if s.write_all(&RDP_CR).await.is_err() {
            return false;
        }
        let mut cc = [0u8; 19];
        if tokio::time::timeout(timeout, s.read_exact(&mut cc)).await.is_err() {
            return false;
        }
        // RDP_NEG_RSP с выбранным TLS/CredSSP; отказ negotiation — старого TLS нет
        if cc[5] != 0xd0 || cc[11] != 0x02 || cc[15] == 0 {
            return false;
        }
    }
    if s.write_all(&old_tls_hello(ver)).await.is_err() {
        return false;
    }
    let mut first = [0u8; 1];
    matches!(tokio::time::timeout(timeout, s.read_exact(&mut first)).await, Ok(Ok(_)) if first[0] == 0x16)
}

/// Какую старую версию TLS принимает сервер: "1.1"/"1.0", либо "auto" — сервер
/// говорит на современном TLS, но со слабыми шифрами (хватит понизить seclevel).
/// rdp = true — сначала согласование RDP (X.224), иначе обычный TLS (RD-шлюз).
pub async fn old_tls_version(host: &str, port: u16, rdp: bool) -> &'static str {
    let h = host.trim_matches(['[', ']']);
    let addr = if h.contains(':') { format!("[{}]:{}", h, port) } else { format!("{}:{}", h, port) };
    for (ver, name) in [([0x03, 0x02], "1.1"), ([0x03, 0x01], "1.0")] {
        if old_tls_accepts(&addr, ver, rdp).await {
            return name;
        }
    }
    "auto"
}

async fn check(addr: String, kind: &str, timeout: Duration) -> bool {
    let start = tokio::time::Instant::now();
    let Ok(Ok(mut s)) = tokio::time::timeout(timeout, TcpStream::connect(addr)).await else {
        return false;
    };
    // На ответ — оставшееся время, но не меньше секунды: через прокси он приходит дольше
    let left = timeout.saturating_sub(start.elapsed()).max(Duration::from_millis(1000));
    let hello: Option<Vec<u8>> = match kind {
        "rdp" => Some(RDP_CR.to_vec()),
        "tls" => Some(tls_hello()),
        _ => None,
    };
    if let Some(h) = hello {
        if s.write_all(&h).await.is_err() {
            return false;
        }
    }
    let mut buf = [0u8; 64];
    // Баннер SSH/VNC может прийти по частям: одиночный read вернёт «SS» — и хост
    // ложно считался бы недоступным. Копим минимум 4 байта в пределах оставшегося времени.
    let need = if matches!(kind, "ssh" | "vnc") { 4 } else { 1 };
    let mut acc: Vec<u8> = Vec::new();
    let read = async {
        while acc.len() < need {
            let n = s.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            acc.extend_from_slice(&buf[..n]);
        }
        Ok::<(), std::io::Error>(())
    };
    match tokio::time::timeout(left, read).await {
        // Пришли данные: у RDP ответ начинается с TPKT 0x03, у SSH — с «SSH-», у TLS — 0x15/0x16
        Ok(Ok(())) if !acc.is_empty() => match kind {
            "rdp" => acc[0] == 0x03,
            "ssh" => acc.starts_with(b"SSH-"),
            "vnc" => acc.starts_with(b"RFB "),
            "tls" => acc[0] == 0x15 || acc[0] == 0x16,
            _ => true,
        },
        // Соединение закрыли — за адресом никого нет (так отвечает прокси)
        Ok(_) => false,
        // Тишина: для протоколов, где первым говорит сервер или ответ обязателен, — недоступен;
        // для прочих (telnet, WinBox) — соединение держится, считаем доступным
        Err(_) => !matches!(kind, "rdp" | "ssh" | "tls" | "vnc"),
    }
}

/// Проверить хосты параллельно (не больше `parallel` одновременно).
pub async fn probe(targets: Vec<Target>, timeout_ms: u64, parallel: usize) -> Vec<Reach> {
    let sem = Arc::new(Semaphore::new(parallel.clamp(1, 64)));
    let timeout = Duration::from_millis(timeout_ms.clamp(200, 10_000));
    let mut tasks = Vec::with_capacity(targets.len());
    for t in targets {
        let sem = sem.clone();
        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await;
            let h = t.host.trim_matches(['[', ']']);
            // IPv6-адрес для connect обязан быть в скобках, иначе порт неотличим от адреса
            let addr = if h.contains(':') { format!("[{}]:{}", h, t.port) } else { format!("{}:{}", h, t.port) };
            let up = check(addr, &t.kind, timeout).await;
            Reach { id: t.id, up }
        }));
    }
    let mut out = Vec::with_capacity(tasks.len());
    for t in tasks {
        if let Ok(r) = t.await {
            out.push(r);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    fn tgt(id: &str, port: u16, kind: &str) -> Target {
        Target { id: id.into(), host: "127.0.0.1".into(), port, kind: kind.into() }
    }

    #[tokio::test]
    async fn probe_kinds() {
        // SSH-сервер, присылающий баннер
        let ssh = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ssh_port = ssh.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut c, _)) = ssh.accept().await {
                let _ = c.write_all(b"SSH-2.0-test\r\n").await;
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
        // «Прокси»: принимает и молчит / сразу закрывает
        let silent = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let silent_port = silent.local_addr().unwrap().port();
        tokio::spawn(async move {
            let mut keep = vec![];
            while let Ok((c, _)) = silent.accept().await {
                keep.push(c);
            }
        });
        let closer = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let closer_port = closer.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((c, _)) = closer.accept().await {
                drop(c);
            }
        });
        let r = probe(
            vec![
                tgt("ssh", ssh_port, "ssh"),
                tgt("ssh-silent", silent_port, "ssh"),
                tgt("rdp-closed", closer_port, "rdp"),
                tgt("tcp-silent", silent_port, "tcp"),
                tgt("refused", 1, "tcp"),
            ],
            800,
            8,
        )
        .await;
        let up = |id: &str| r.iter().find(|x| x.id == id).unwrap().up;
        assert!(up("ssh"));
        assert!(!up("ssh-silent"));
        assert!(!up("rdp-closed"));
        assert!(up("tcp-silent"));
        assert!(!up("refused"));
        assert_eq!(tls_hello().len(), 5 + 4 + 2 + 32 + 1 + 2 + 12 + 2);
    }

    /// v72: баннер, пришедший по одному байту, не должен давать «недоступен»
    #[tokio::test]
    async fn ssh_banner_split_across_reads() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut c, _)) = l.accept().await {
                for b in b"SSH-2.0-split\r\n" {
                    let _ = c.write_all(&[*b]).await;
                    tokio::time::sleep(Duration::from_millis(30)).await;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
        let r = probe(vec![tgt("split", port, "ssh")], 1500, 4).await;
        assert!(r[0].up, "частичный баннер не должен давать «недоступен»");
    }

    /// v33: пробник ходит на IPv6-адрес в скобках
    #[tokio::test]
    async fn ipv6_loopback_probe() {
        let l = TcpListener::bind("[::1]:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut c, _)) = l.accept().await {
                let _ = c.write_all(b"SSH-2.0-v6\r\n").await;
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
        let r = probe(vec![Target { id: "v6".into(), host: "::1".into(), port, kind: "ssh".into() }], 800, 4).await;
        assert!(r[0].up, "IPv6 loopback должен проверяться");
    }

    /// Пробник старого TLS: ServerHello на 1.0 и alert на 1.1 → "1.0"; alert на оба → "auto"
    #[tokio::test]
    async fn old_tls_version_probe() {
        const CC: [u8; 19] = [
            0x03, 0x00, 0x00, 0x13, 0x0e, 0xd0, 0x00, 0x00, 0x00, 0x01, 0x00, 0x02, 0x00, 0x08, 0x00, 0x01, 0x00, 0x00, 0x00,
        ];
        let spawn = |only_tls10: bool| async move {
            let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = l.local_addr().unwrap().port();
            tokio::spawn(async move {
                while let Ok((mut c, _)) = l.accept().await {
                    let mut cr = [0u8; 19];
                    if c.read_exact(&mut cr).await.is_err() {
                        continue;
                    }
                    let _ = c.write_all(&CC).await;
                    let mut rec = [0u8; 5];
                    if c.read_exact(&mut rec).await.is_err() {
                        continue;
                    }
                    let hello10 = rec[1..3] == [0x03, 0x01];
                    let resp: &[u8] = if only_tls10 && hello10 {
                        &[0x16, 0x03, 0x01, 0x00, 0x00]
                    } else {
                        &[0x15, 0x03, 0x01, 0x00, 0x02, 0x02, 0x46]
                    };
                    let _ = c.write_all(resp).await;
                    tokio::time::sleep(Duration::from_millis(300)).await;
                }
            });
            port
        };
        let port = spawn(true).await;
        assert_eq!(old_tls_version("127.0.0.1", port, true).await, "1.0");
        let mport = spawn(false).await;
        assert_eq!(old_tls_version("127.0.0.1", mport, true).await, "auto");
        assert_eq!(old_tls_version("127.0.0.1", 1, true).await, "auto");
    }

    /// Ручная проверка через сеть этого ПК: cargo test probe_live -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn probe_live() {
        let t = |id: &str, h: &str, p: u16, k: &str| Target { id: id.into(), host: h.into(), port: p, kind: k.into() };
        let r = probe(
            vec![
                t("dead-rdp", "192.0.2.1", 3389, "rdp"),
                t("dead-ssh", "203.0.113.5", 22, "ssh"),
                t("dead-tls", "192.0.2.1", 443, "tls"),
                t("rebex-ssh", "test.rebex.net", 22, "ssh"),
                t("cf-tls", "1.1.1.1", 443, "tls"),
            ],
            1500,
            8,
        )
        .await;
        for x in &r {
            eprintln!("{} {}", x.id, x.up);
        }
    }
}
