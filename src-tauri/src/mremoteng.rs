//! Импорт подключений из mRemoteNG (confCons.xml).
//!
//! Поддерживаются файлы с шифрованием AES-GCM (mRemoteNG 1.75+, в т.ч. полное
//! шифрование файла) и старый формат AES-CBC. Пароль по умолчанию — «mR3m»;
//! если файл защищён своим паролем, его нужно передать в `password`.
//! Модуль только разбирает файл — записи в Bitwarden создаёт интерфейс.

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use aes_gcm::aead::consts::U16;
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::aes::Aes256;
use aes_gcm::AesGcm;
use base64::Engine;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

const DEFAULT_PASSWORD: &str = "mR3m";
/// Специальная ошибка: файл защищён паролем (интерфейс спросит его)
pub const NEED_PASSWORD: &str = "MREMOTENG_PASSWORD";

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Gateway {
    pub host: String,
    pub username: String,
    pub password: String,
    pub domain: String,
    /// Шлюз использует учётку самого подключения
    pub same: bool,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Conn {
    /// Путь папок от корня файла
    pub path: Vec<String>,
    pub name: String,
    pub protocol: String,
    pub host: String,
    pub port: Option<u16>,
    pub username: String,
    pub password: String,
    pub domain: String,
    pub notes: String,
    /// Поля подключения в формате менеджера (resolution, admin, drives…)
    pub opts: BTreeMap<String, String>,
    pub gateway: Option<Gateway>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Import {
    pub conns: Vec<Conn>,
    /// Все папки (включая пустые)
    pub folders: Vec<Vec<String>>,
    /// Пропущенные записи: «имя (протокол)»
    pub skipped: Vec<String>,
}

enum Mode {
    Gcm { iterations: u32 },
    Cbc,
}

struct Crypto {
    mode: Mode,
    password: String,
}

impl Crypto {
    fn decrypt(&self, b64: &str) -> Result<String, ()> {
        let b64 = b64.trim();
        if b64.is_empty() {
            return Ok(String::new());
        }
        let data = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|_| ())?;
        let plain = match self.mode {
            Mode::Gcm { iterations } => {
                // соль(16) | nonce(16) | шифртекст+тег(16); соль — ассоциированные данные
                if data.len() < 16 + 16 + 16 {
                    return Err(());
                }
                let (salt, rest) = data.split_at(16);
                let (nonce, ct) = rest.split_at(16);
                let mut key = [0u8; 32];
                pbkdf2::pbkdf2_hmac::<sha1::Sha1>(self.password.as_bytes(), salt, iterations, &mut key);
                let c = AesGcm::<Aes256, U16>::new_from_slice(&key).map_err(|_| ())?;
                c.decrypt(nonce.into(), Payload { msg: ct, aad: salt }).map_err(|_| ())?
            }
            Mode::Cbc => {
                // Старый формат: ключ = MD5(пароль), IV — первые 16 байт
                if data.len() < 32 {
                    return Err(());
                }
                use md5::Digest;
                let key = md5::Md5::digest(self.password.as_bytes());
                let (iv, ct) = data.split_at(16);
                cbc::Decryptor::<aes::Aes128>::new_from_slices(&key, iv)
                    .map_err(|_| ())?
                    .decrypt_padded_vec_mut::<Pkcs7>(ct)
                    .map_err(|_| ())?
            }
        };
        String::from_utf8(plain).map_err(|_| ())
    }
}

/// Атрибуты узла с учётом наследования от родительской папки (Inherit*="True").
fn resolve(node: roxmltree::Node, parent: &HashMap<String, String>) -> HashMap<String, String> {
    let own: HashMap<String, String> = node
        .attributes()
        .map(|a| (a.name().to_string(), a.value().to_string()))
        .collect();
    let mut out = HashMap::new();
    for (k, v) in &own {
        if k.starts_with("Inherit") {
            continue;
        }
        let inherit = own
            .get(&format!("Inherit{}", k))
            .is_some_and(|x| x.eq_ignore_ascii_case("true"));
        let val = match (inherit, parent.get(k)) {
            (true, Some(pv)) => pv.clone(),
            _ => v.clone(),
        };
        out.insert(k.clone(), val);
    }
    // Унаследованные атрибуты, которых у узла нет вовсе
    for (k, v) in parent {
        if !out.contains_key(k)
            && own
                .get(&format!("Inherit{}", k))
                .is_some_and(|x| x.eq_ignore_ascii_case("true"))
        {
            out.insert(k.clone(), v.clone());
        }
    }
    out
}

fn truthy(v: Option<&String>) -> bool {
    v.is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "yes" | "1"))
}

struct Walker<'a> {
    crypto: &'a Crypto,
    out: Import,
}

impl Walker<'_> {
    fn dec(&self, v: Option<&String>) -> String {
        v.map(|v| self.crypto.decrypt(v).unwrap_or_default()).unwrap_or_default()
    }

    fn walk(&mut self, parent_node: roxmltree::Node, parent: &HashMap<String, String>, path: &[String]) {
        for n in parent_node.children().filter(|n| n.is_element() && n.tag_name().name() == "Node") {
            let a = resolve(n, parent);
            let name = a.get("Name").cloned().unwrap_or_default();
            let kind = a.get("Type").map(|s| s.as_str()).unwrap_or("Connection");
            if kind.eq_ignore_ascii_case("Container") {
                let mut p = path.to_vec();
                p.push(name.replace('/', "-"));
                self.out.folders.push(p.clone());
                self.walk(n, &a, &p);
                continue;
            }
            self.conn(&a, name, path);
            // У подключения тоже могут быть дочерние узлы (редко) — обходим как папку
            if n.children().any(|c| c.is_element()) {
                self.walk(n, &a, path);
            }
        }
    }

    fn conn(&mut self, a: &HashMap<String, String>, name: String, path: &[String]) {
        let get = |k: &str| a.get(k).cloned().unwrap_or_default();
        let raw_proto = get("Protocol");
        let protocol = match raw_proto.to_ascii_uppercase().as_str() {
            "RDP" => "rdp",
            "SSH1" | "SSH2" => "ssh",
            "TELNET" => "telnet",
            _ => {
                self.out.skipped.push(format!("{} ({})", name, raw_proto));
                return;
            }
        };
        let default_port = match protocol {
            "rdp" => 3389,
            "ssh" => 22,
            _ => 23,
        };
        let port = get("Port").trim().parse::<u16>().ok().filter(|p| *p != default_port && *p != 0);

        let mut opts = BTreeMap::new();
        if protocol == "rdp" {
            let res = get("Resolution");
            if let Some(r) = res.strip_prefix("Res") {
                opts.insert("resolution".into(), r.to_string());
            }
            if truthy(a.get("UseConsoleSession")) {
                opts.insert("admin".into(), "true".into());
            }
            if get("RedirectSound") == "BringToThisComputer" {
                opts.insert("sound".into(), "true".into());
            }
            if truthy(a.get("RedirectAudioCapture")) {
                opts.insert("microphone".into(), "true".into());
            }
            let drives = get("RedirectDiskDrives");
            if truthy(Some(&drives)) || drives == "All" || drives == "Local" {
                opts.insert("drives".into(), "true".into());
            }
            if truthy(a.get("RedirectPrinters")) {
                opts.insert("printers".into(), "true".into());
            }
            if truthy(a.get("RedirectSmartCards")) {
                opts.insert("smartcard".into(), "true".into());
            }
            if a.get("RedirectClipboard").is_some_and(|v| v.eq_ignore_ascii_case("false")) {
                opts.insert("clipboard".into(), "off".into());
            }
        }

        let gateway = if protocol == "rdp"
            && matches!(get("RDGatewayUsageMethod").as_str(), "Always" | "Detect")
            && !get("RDGatewayHostname").trim().is_empty()
        {
            let same = get("RDGatewayUseConnectionCredentials") != "No";
            Some(Gateway {
                host: get("RDGatewayHostname").trim().to_string(),
                username: if same { String::new() } else { get("RDGatewayUsername") },
                password: if same { String::new() } else { self.dec(a.get("RDGatewayPassword")) },
                domain: if same { String::new() } else { get("RDGatewayDomain") },
                same,
            })
        } else {
            None
        };

        let host = get("Hostname").trim().to_string();
        if host.is_empty() {
            self.out.skipped.push(format!("{} (нет адреса)", name));
            return;
        }
        self.out.conns.push(Conn {
            path: path.to_vec(),
            name: if name.trim().is_empty() { host.clone() } else { name },
            protocol: protocol.into(),
            host,
            port,
            username: get("Username"),
            password: self.dec(a.get("Password")),
            domain: get("Domain"),
            notes: get("Descr"),
            opts,
            gateway,
        });
    }
}

pub fn parse(xml: &str, password: Option<&str>) -> Result<Import, String> {
    let doc = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("Не удалось разобрать XML: {}", e))?;
    let root = doc.root_element();
    if root.tag_name().name() != "Connections" {
        return Err("Это не файл подключений mRemoteNG (ожидается confCons.xml)".into());
    }
    let mode = match root.attribute("BlockCipherMode") {
        Some(m) if m.eq_ignore_ascii_case("GCM") => Mode::Gcm {
            iterations: root.attribute("KdfIterations").and_then(|v| v.parse().ok()).unwrap_or(1000),
        },
        Some(m) => return Err(format!("Режим шифрования {} не поддерживается", m)),
        None => Mode::Cbc,
    };
    let pw = password.filter(|p| !p.is_empty()).unwrap_or(DEFAULT_PASSWORD).to_string();
    let crypto = Crypto { mode, password: pw };

    // Проверка пароля по атрибуту Protected
    if let Some(prot) = root.attribute("Protected") {
        match crypto.decrypt(prot) {
            Ok(_) => {}
            Err(_) if password.is_none() => return Err(NEED_PASSWORD.into()),
            Err(_) => return Err("Неверный пароль файла mRemoteNG".into()),
        }
    }

    let mut w = Walker {
        crypto: &crypto,
        out: Import { conns: vec![], folders: vec![], skipped: vec![] },
    };
    let full = root.attribute("FullFileEncryption").is_some_and(|v| v.eq_ignore_ascii_case("true"));
    if full {
        let inner = crypto
            .decrypt(root.text().unwrap_or_default())
            .map_err(|_| if password.is_none() { NEED_PASSWORD.to_string() } else { "Неверный пароль файла mRemoteNG".into() })?;
        let wrapped = format!("<Connections>{}</Connections>", inner.trim_start_matches('\u{feff}'));
        let d2 = roxmltree::Document::parse(&wrapped).map_err(|e| format!("Не удалось разобрать файл: {}", e))?;
        w.walk(d2.root_element(), &HashMap::new(), &[]);
    } else {
        w.walk(root, &HashMap::new(), &[]);
    }
    Ok(w.out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc_gcm(plain: &str, password: &str, iterations: u32) -> String {
        let salt = [7u8; 16];
        let nonce = [9u8; 16];
        let mut key = [0u8; 32];
        pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password.as_bytes(), &salt, iterations, &mut key);
        let c = AesGcm::<Aes256, U16>::new_from_slice(&key).unwrap();
        let ct = c.encrypt((&nonce).into(), Payload { msg: plain.as_bytes(), aad: &salt }).unwrap();
        let mut out = salt.to_vec();
        out.extend_from_slice(&nonce);
        out.extend(ct);
        base64::engine::general_purpose::STANDARD.encode(out)
    }

    #[test]
    fn parses_tree_inheritance_and_gateway() {
        let pw = enc_gcm("secret", "mR3m", 1000);
        let gpw = enc_gcm("gwpass", "mR3m", 1000);
        let prot = enc_gcm("ThisIsNotProtected", "mR3m", 1000);
        let xml = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<Connections Name="Connections" Export="false" EncryptionEngine="AES" BlockCipherMode="GCM" KdfIterations="1000" FullFileEncryption="false" Protected="{prot}" ConfVersion="2.6">
  <Node Name="Офис" Type="Container" Username="ivan" Domain="CORP" Password="{pw}">
    <Node Name="srv1" Type="Connection" Protocol="RDP" Hostname="10.0.0.1" Port="3389" Username="" InheritUsername="true" InheritDomain="true" InheritPassword="true"
      Resolution="Res1920x1080" RedirectClipboard="false" RedirectDiskDrives="true" UseConsoleSession="true"
      RDGatewayUsageMethod="Always" RDGatewayHostname="gw.corp" RDGatewayUseConnectionCredentials="No" RDGatewayUsername="gw" RDGatewayPassword="{gpw}" RDGatewayDomain="CORP" />
    <Node Name="linux" Type="Connection" Protocol="SSH2" Hostname="10.0.0.2" Port="2222" Username="root" />
    <Node Name="web" Type="Connection" Protocol="HTTPS" Hostname="x" />
  </Node>
</Connections>"#
        );
        let r = parse(&xml, None).unwrap();
        assert_eq!(r.folders, vec![vec!["Офис".to_string()]]);
        assert_eq!(r.conns.len(), 2);
        let s = &r.conns[0];
        assert_eq!(s.path, vec!["Офис".to_string()]);
        assert_eq!((s.username.as_str(), s.domain.as_str(), s.password.as_str()), ("ivan", "CORP", "secret"));
        assert_eq!(s.port, None);
        assert_eq!(s.opts.get("resolution").map(|s| s.as_str()), Some("1920x1080"));
        assert_eq!(s.opts.get("clipboard").map(|s| s.as_str()), Some("off"));
        assert!(s.opts.contains_key("drives") && s.opts.contains_key("admin"));
        let g = s.gateway.as_ref().unwrap();
        assert_eq!((g.host.as_str(), g.username.as_str(), g.password.as_str(), g.same), ("gw.corp", "gw", "gwpass", false));
        assert_eq!(r.conns[1].protocol, "ssh");
        assert_eq!(r.conns[1].port, Some(2222));
        assert_eq!(r.skipped.len(), 1);
    }

    #[test]
    fn custom_password_and_full_encryption() {
        let inner = r#"<Node Name="a" Type="Connection" Protocol="SSH2" Hostname="h" />"#;
        let body = enc_gcm(inner, "pw1", 10000);
        let prot = enc_gcm("ThisIsProtected", "pw1", 10000);
        let xml = format!(
            r#"<Connections Name="Connections" BlockCipherMode="GCM" KdfIterations="10000" FullFileEncryption="true" Protected="{prot}">{body}</Connections>"#
        );
        assert_eq!(parse(&xml, None).unwrap_err(), NEED_PASSWORD);
        assert!(parse(&xml, Some("bad")).is_err());
        let r = parse(&xml, Some("pw1")).unwrap();
        assert_eq!(r.conns.len(), 1);
        assert_eq!(r.conns[0].host, "h");
    }

    #[test]
    fn legacy_cbc() {
        use aes::cipher::BlockEncryptMut;
        use md5::Digest;
        let key = md5::Md5::digest(b"mR3m");
        let iv = [3u8; 16];
        let ct = cbc::Encryptor::<aes::Aes128>::new_from_slices(&key, &iv)
            .unwrap()
            .encrypt_padded_vec_mut::<Pkcs7>(b"pass");
        let mut data = iv.to_vec();
        data.extend(ct);
        let b = base64::engine::general_purpose::STANDARD.encode(data);
        let xml = format!(r#"<Connections Name="C"><Node Name="x" Type="Connection" Protocol="RDP" Hostname="h" Password="{b}"/></Connections>"#);
        let r = parse(&xml, None).unwrap();
        assert_eq!(r.conns[0].password, "pass");
    }
}
