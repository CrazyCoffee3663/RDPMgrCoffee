//! Проверка сертификата RDP-сервера и RD-шлюза до запуска FreeRDP — как ключи SSH:
//! при первом подключении пользователь видит отпечаток и подтверждает его, при смене
//! сертификата — предупреждение. FreeRDP затем получает только подтверждённый отпечаток
//! (/cert:deny,fingerprint:sha256:…) и с другим сертификатом не соединится.
//!
//! Режим FreeRDP «tofu» молча доверяет первому увиденному сертификату — перехват в этот
//! момент остался бы незамеченным.

use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// X.224 Connection Request с RDP_NEG_REQ (TLS | CredSSP)
const RDP_CR: [u8; 19] = [
    0x03, 0x00, 0x00, 0x13, 0x0e, 0xe0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x08, 0x00, 0x03, 0x00, 0x00, 0x00,
];

/// Принимает любой сертификат и запоминает его — проверку делаем сами по отпечатку.
#[derive(Debug)]
struct Capture {
    cert: Mutex<Option<Vec<u8>>>,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl rustls::client::danger::ServerCertVerifier for Capture {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        *self.cert.lock().unwrap() = Some(end_entity.to_vec());
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// SHA-256 сертификата сервера (DER), шестнадцатеричной строкой в верхнем регистре.
/// rdp = true — сначала согласование RDP (X.224), иначе обычный TLS (RD-шлюз, HTTPS).
pub async fn fetch(host: &str, port: u16, rdp: bool) -> Result<String, String> {
    let io = async {
        let mut s = TcpStream::connect((host.trim_matches(['[', ']']), port))
            .await
            .map_err(|e| format!("Не удалось подключиться к {}:{}: {}", host, port, e))?;
        if rdp {
            s.write_all(&RDP_CR).await.map_err(|e| e.to_string())?;
            let mut hdr = [0u8; 4];
            s.read_exact(&mut hdr).await.map_err(|_| "Сервер не ответил по протоколу RDP".to_string())?;
            let len = u16::from_be_bytes([hdr[2], hdr[3]]) as usize;
            if hdr[0] != 3 || !(7..=512).contains(&len) {
                return Err("Сервер не ответил по протоколу RDP".to_string());
            }
            let mut body = vec![0u8; len - 4];
            s.read_exact(&mut body).await.map_err(|e| e.to_string())?;
            // body: X.224 CC (7 байт), затем RDP_NEG_RSP/FAILURE: type, flags, length(2), value(4)
            if body.len() < 15 {
                return Err("Сервер использует устаревшую защиту RDP без TLS — подключение небезопасно".to_string());
            }
            match body[7] {
                0x02 => {
                    let proto = u32::from_le_bytes([body[11], body[12], body[13], body[14]]);
                    if proto == 0 {
                        return Err("Сервер использует устаревшую защиту RDP без TLS — подключение небезопасно".to_string());
                    }
                }
                0x03 => return Err("Сервер отказался от защищённого (TLS) подключения".to_string()),
                _ => return Err("Неожиданный ответ RDP-сервера".to_string()),
            }
        }
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let cap = Arc::new(Capture { cert: Mutex::new(None), provider: provider.clone() });
        let cfg = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .dangerous()
            .with_custom_certificate_verifier(cap.clone())
            .with_no_client_auth();
        let name = rustls::pki_types::ServerName::try_from(host.trim_matches(['[', ']']).to_string())
            .unwrap_or_else(|_| rustls::pki_types::ServerName::try_from("rdp.invalid").unwrap())
            .to_owned();
        let conn = tokio_rustls::TlsConnector::from(Arc::new(cfg));
        let _tls = conn.connect(name, s).await.map_err(|e| format!("TLS с {}:{} не установлен: {}", host, port, e))?;
        let der = cap.cert.lock().unwrap().take().ok_or("Сервер не прислал сертификат")?;
        Ok::<String, String>(hex(&Sha256::digest(&der)))
    };
    tokio::time::timeout(Duration::from_secs(15), io)
        .await
        .map_err(|_| format!("{}:{} не ответил за 15 секунд", host, port))?
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02X}", x)).collect()
}

/// «AB:CD:…» — для показа пользователю
pub fn pretty(fp: &str) -> String {
    fp.as_bytes().chunks(2).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join(":")
}

/// Запомненный отпечаток для «хост:порт»
pub fn known(path: &Path, id: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()?
        .lines()
        .find_map(|l| l.split_once(' ').filter(|(h, _)| *h == id).map(|(_, f)| f.trim().to_string()))
}

pub fn remember(path: &Path, id: &str, fp: &str) {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut lines: Vec<String> = text
        .lines()
        .filter(|l| l.split_once(' ').map(|(h, _)| h != id).unwrap_or(true))
        .map(String::from)
        .collect();
    lines.push(format!("{} {}", id, fp));
    let _ = std::fs::write(path, lines.join("\n") + "\n");
}

/// Значение для /cert: принимать только эти отпечатки, остальные — отклонять без вопросов.
/// Отпечаток передаётся в обоих написаниях (с двоеточиями и без) — их понимают разные версии FreeRDP.
pub fn freerdp_pin(fps: &[&str]) -> String {
    let mut out = vec!["deny".to_string()];
    for f in fps {
        out.push(format!("fingerprint:sha256:{}", f));
        out.push(format!("fingerprint:sha256:{}", pretty(f)));
    }
    out.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_and_pin() {
        let p = std::env::temp_dir().join(format!("rdpcert-{}", std::process::id()));
        let _ = std::fs::remove_file(&p);
        assert_eq!(known(&p, "h:3389"), None);
        remember(&p, "h:3389", "AABB");
        remember(&p, "g:443", "CCDD");
        remember(&p, "h:3389", "EEFF");
        assert_eq!(known(&p, "h:3389").as_deref(), Some("EEFF"));
        assert_eq!(known(&p, "g:443").as_deref(), Some("CCDD"));
        assert_eq!(pretty("AABBCC"), "AA:BB:CC");
        assert_eq!(freerdp_pin(&["AABB"]), "deny,fingerprint:sha256:AABB,fingerprint:sha256:AA:BB");
        let _ = std::fs::remove_file(&p);
    }

    /// Проверка на RDP этого компьютера: cargo test rdp_localhost -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn rdp_localhost() {
        let fp = fetch("127.0.0.1", 3389, true).await.unwrap();
        eprintln!("localhost RDP sha256 {}", pretty(&fp));
        assert_eq!(fp.len(), 64);
    }
}
