//! Импорт подключений из Terminals (github.com/Terminals-Origin/Terminals).
//!
//! Файлы (обычно %LOCALAPPDATA%\Robert_Chartier\Terminals\Data или папка Data рядом с программой):
//!   Terminals.config — настройки; атрибут terminalsPassword — проверка мастер-пароля;
//!                      в старых версиях здесь же <favorites><add …/></favorites>
//!   Favorites.xml    — подключения, группы (новый формат)
//!   Credentials.xml  — сохранённые учётные данные
//!
//! Шифрование (PasswordFunctions2):
//!   с мастер-паролем — ключ PBKDF2-HMAC-SHA1(пароль, соль, 2121) → 32 байта; значение —
//!     Base64(IV[16] || AES-256-CBC-PKCS7(UTF-16LE текст));
//!   без мастер-пароля — Base64(IV[16] || DPAPI(CurrentUser, entropy = IV)).

use crate::mremoteng::{Conn, Gateway, Import};
use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use std::collections::{BTreeMap, HashMap};

pub const NEED_PASSWORD: &str = "TERMINALS_PASSWORD";
const ITERATIONS: u32 = 2121;

/// Чем расшифровывать значения
enum Key {
    Master([u8; 32]),
    Dpapi,
}

fn b64(s: &str) -> Option<Vec<u8>> {
    B64.decode(s.trim()).ok()
}

fn utf16(b: &[u8]) -> String {
    let w: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&w).trim_end_matches('\0').to_string()
}

impl Key {
    fn decrypt(&self, value: &str) -> Result<String, String> {
        let v = value.trim();
        if v.is_empty() {
            return Ok(String::new());
        }
        let raw = b64(v).ok_or("значение повреждено")?;
        if raw.len() <= 16 {
            return Ok(String::new());
        }
        let (iv, ct) = raw.split_at(16);
        let plain = match self {
            Key::Master(k) => cbc::Decryptor::<aes::Aes256>::new_from_slices(k, iv)
                .map_err(|e| e.to_string())?
                .decrypt_padded_vec_mut::<Pkcs7>(ct)
                .map_err(|_| "не удалось расшифровать".to_string())?,
            Key::Dpapi => dpapi_unprotect(ct, iv)?,
        };
        Ok(utf16(&plain))
    }
}

#[cfg(windows)]
fn dpapi_unprotect(data: &[u8], entropy: &[u8]) -> Result<Vec<u8>, String> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
    let ent = CRYPT_INTEGER_BLOB { cbData: entropy.len() as u32, pbData: entropy.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(&input, None, Some(&ent), None, None, 0, &mut out)
            .map_err(|_| "пароль зашифрован другим пользователем Windows или на другом компьютере".to_string())?;
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(out.pbData as _)));
        Ok(v)
    }
}
#[cfg(not(windows))]
fn dpapi_unprotect(_: &[u8], _: &[u8]) -> Result<Vec<u8>, String> {
    Err("DPAPI есть только в Windows".into())
}

fn pbkdf2_key(password: &str, salt: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password.as_bytes(), salt, ITERATIONS, &mut out);
    out
}

/// Ключ по атрибуту terminalsPassword из Terminals.config и мастер-паролю.
fn make_key(stored: &str, password: Option<&str>) -> Result<Key, String> {
    if stored.trim().is_empty() {
        return Ok(Key::Dpapi);
    }
    let raw = b64(stored).ok_or("Не удалось прочитать мастер-пароль Terminals")?;
    if raw.len() != 64 {
        return Err("Файлы старой версии Terminals (до 3.0) не поддерживаются: откройте их в новой версии Terminals и сохраните".into());
    }
    let Some(pw) = password.filter(|p| !p.is_empty()) else {
        return Err(NEED_PASSWORD.into());
    };
    let (key_salt, rest) = raw.split_at(16);
    let (val_salt, val_key) = rest.split_at(16);
    if pbkdf2_key(pw, val_salt).as_slice() != val_key {
        return Err("Неверный мастер-пароль Terminals".into());
    }
    Ok(Key::Master(pbkdf2_key(pw, key_salt)))
}

fn local<'a>(n: roxmltree::Node<'a, 'a>) -> &'a str {
    n.tag_name().name()
}
fn child<'a>(n: roxmltree::Node<'a, 'a>, name: &str) -> Option<roxmltree::Node<'a, 'a>> {
    n.children().find(|c| c.is_element() && local(*c).eq_ignore_ascii_case(name))
}
fn text(n: roxmltree::Node, name: &str) -> String {
    child(n, name).and_then(|c| c.text()).unwrap_or("").trim().to_string()
}
fn attr(n: roxmltree::Node, name: &str) -> String {
    n.attributes().find(|a| a.name().eq_ignore_ascii_case(name)).map(|a| a.value().trim().to_string()).unwrap_or_default()
}

fn protocol(p: &str) -> Option<&'static str> {
    match p.trim().to_ascii_uppercase().as_str() {
        "" | "RDP" => Some("rdp"),
        "SSH" => Some("ssh"),
        "TELNET" => Some("telnet"),
        _ => None,
    }
}

/// Заметки в Terminals хранятся в Base64
fn notes(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    match b64(s) {
        Some(b) => String::from_utf8(b.clone()).unwrap_or_else(|_| utf16(&b)),
        None => s.to_string(),
    }
}

#[derive(Clone, Default)]
struct Cred {
    user: String,
    domain: String,
    password: String,
}

/// "ДОМЕН\логин" → (домен, логин)
fn split_user(u: &str, domain: &str) -> (String, String) {
    match u.split_once('\\') {
        Some((d, n)) if domain.is_empty() => (d.to_string(), n.to_string()),
        _ => (domain.to_string(), u.to_string()),
    }
}

/// Разобрать файлы Terminals. Любой из текстов может быть пустым.
pub fn parse(config: &str, favorites: &str, credentials: &str, password: Option<&str>) -> Result<Import, String> {
    let cfg_doc = if config.trim().is_empty() {
        None
    } else {
        Some(roxmltree::Document::parse(config).map_err(|e| format!("Terminals.config повреждён: {}", e))?)
    };
    let stored = cfg_doc
        .as_ref()
        .and_then(|d| d.descendants().find(|n| n.is_element() && n.has_attribute("terminalsPassword")))
        .map(|n| attr(n, "terminalsPassword"))
        .unwrap_or_default();
    if stored.is_empty() && cfg_doc.is_none() && password.is_some_and(|p| !p.is_empty()) {
        return Err("Для файлов с мастер-паролем нужен и Terminals.config (в нём проверка пароля)".into());
    }
    let key = make_key(&stored, password)?;
    let mut failed = 0usize;
    let mut dec = |v: &str| -> String {
        key.decrypt(v).unwrap_or_else(|_| {
            failed += 1;
            String::new()
        })
    };

    // Сохранённые учётные данные
    let mut creds_by_id: HashMap<String, Cred> = HashMap::new();
    let mut creds_by_name: HashMap<String, Cred> = HashMap::new();
    if !credentials.trim().is_empty() {
        let doc = roxmltree::Document::parse(credentials).map_err(|e| format!("Credentials.xml повреждён: {}", e))?;
        for n in doc.descendants().filter(|n| n.is_element() && local(*n) == "CredentialSet") {
            let c = Cred {
                user: dec(&text(n, "EncryptedUserName")),
                domain: dec(&text(n, "EncryptedDomain")),
                password: dec(&text(n, "EncryptedPassword")),
            };
            creds_by_id.insert(attr(n, "id").to_lowercase(), c.clone());
            creds_by_name.insert(text(n, "Name").to_lowercase(), c);
        }
    }
    let find_cred = |r: &str| -> Option<Cred> {
        let r = r.trim().to_lowercase();
        if r.is_empty() || r == "00000000-0000-0000-0000-000000000000" {
            return None;
        }
        creds_by_id.get(&r).or_else(|| creds_by_name.get(&r)).cloned()
    };

    let mut conns = Vec::new();
    let mut skipped = Vec::new();
    let mut folders: Vec<Vec<String>> = Vec::new();
    let mut add_folder = |p: &[String]| {
        for i in 1..=p.len() {
            let f = p[..i].to_vec();
            if !folders.contains(&f) {
                folders.push(f);
            }
        }
    };

    // ---- Новый формат: Favorites.xml ----
    if !favorites.trim().is_empty() {
        let doc = roxmltree::Document::parse(favorites).map_err(|e| format!("Favorites.xml повреждён: {}", e))?;
        // Группы: id → (имя, родитель)
        let mut groups: HashMap<String, (String, String)> = HashMap::new();
        for g in doc.descendants().filter(|n| n.is_element() && local(*n) == "Group") {
            let id = attr(g, "id").to_lowercase();
            let parent = {
                let p = attr(g, "parent");
                if p.is_empty() { text(g, "Parent") } else { p }
            };
            groups.insert(id, (text(g, "Name"), parent.to_lowercase()));
        }
        let group_path = |id: &str| -> Vec<String> {
            let mut path = vec![];
            let mut cur = id.to_string();
            for _ in 0..16 {
                let Some((name, parent)) = groups.get(&cur) else { break };
                if !name.is_empty() {
                    path.insert(0, name.clone());
                }
                if parent.is_empty() {
                    break;
                }
                cur = parent.clone();
            }
            path
        };
        // Подключение → первая его группа
        let mut fav_group: HashMap<String, Vec<String>> = HashMap::new();
        for fg in doc.descendants().filter(|n| n.is_element() && local(*n) == "FavoritesInGroup") {
            let gid = attr(fg, "groupId").to_lowercase();
            let path = group_path(&gid);
            for g in fg.descendants().filter(|n| n.is_element() && local(*n).eq_ignore_ascii_case("guid")) {
                if let Some(t) = g.text() {
                    fav_group.entry(t.trim().to_lowercase()).or_insert_with(|| path.clone());
                }
            }
        }
        for f in doc.descendants().filter(|n| n.is_element() && local(*n) == "Favorite" && child(*n, "ServerName").is_some()) {
            let name = text(f, "Name");
            let proto_raw = text(f, "Protocol");
            let Some(proto) = protocol(&proto_raw) else {
                skipped.push(format!("{} ({})", name, proto_raw));
                continue;
            };
            let sec = child(f, "Security");
            let own = sec
                .map(|s| Cred {
                    user: dec(&text(s, "EncryptedUserName")),
                    domain: dec(&text(s, "EncryptedDomain")),
                    password: dec(&text(s, "EncryptedPassword")),
                })
                .unwrap_or_default();
            let c = sec.and_then(|s| find_cred(&text(s, "Credential"))).unwrap_or(own);
            let (domain, user) = split_user(&c.user, &c.domain);
            // RD-шлюз: элемент *Gateway* с адресом сервера (точная схема может отличаться по версиям)
            let gateway = f
                .descendants()
                .find(|n| n.is_element() && local(*n).to_lowercase().contains("gateway") && n.children().any(|c| c.is_element()))
                .and_then(|g| {
                    let host = g
                        .children()
                        .find(|c| c.is_element() && local(*c).to_lowercase().contains("host"))
                        .and_then(|c| c.text())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let usage = text(g, "UsageMethod");
                    if host.is_empty() || usage == "0" {
                        return None;
                    }
                    let separate = text(g, "SeparateLogin").eq_ignore_ascii_case("true");
                    let gs = child(g, "Security");
                    let gc = gs
                        .map(|s| {
                            find_cred(&text(s, "Credential")).unwrap_or(Cred {
                                user: dec(&text(s, "EncryptedUserName")),
                                domain: dec(&text(s, "EncryptedDomain")),
                                password: dec(&text(s, "EncryptedPassword")),
                            })
                        })
                        .unwrap_or_default();
                    let (gd, gu) = split_user(&gc.user, &gc.domain);
                    Some(Gateway { host, username: gu, password: gc.password, domain: gd, same: !separate || gc.user.is_empty() })
                });
            let path = fav_group.get(&attr(f, "id").to_lowercase()).cloned().unwrap_or_default();
            add_folder(&path);
            conns.push(Conn {
                path,
                name: if name.is_empty() { text(f, "ServerName") } else { name },
                protocol: proto.into(),
                host: text(f, "ServerName"),
                port: text(f, "Port").parse().ok().filter(|p| *p != default_port(proto)),
                username: user,
                password: c.password,
                domain,
                notes: notes(&text(f, "Notes")),
                opts: BTreeMap::new(),
                gateway: if proto == "rdp" { gateway } else { None },
            });
        }
    }

    // ---- Старый формат: <favorites><add …/></favorites> в Terminals.config ----
    if conns.is_empty() {
        if let Some(doc) = &cfg_doc {
            for f in doc.descendants().filter(|n| n.is_element() && n.has_attribute("serverName")) {
                let name = attr(f, "name");
                let proto_raw = attr(f, "protocol");
                let Some(proto) = protocol(&proto_raw) else {
                    skipped.push(format!("{} ({})", name, proto_raw));
                    continue;
                };
                let c = find_cred(&attr(f, "credential")).unwrap_or(Cred {
                    user: attr(f, "userName"),
                    domain: attr(f, "domainName"),
                    password: dec(&attr(f, "encryptedPassword")),
                });
                let (domain, user) = split_user(&c.user, &c.domain);
                let usage = attr(f, "tsgwUsageMethod");
                let gw_host = attr(f, "tsgwHostname");
                let gateway = (proto == "rdp" && !gw_host.is_empty() && !usage.is_empty() && usage != "0").then(|| {
                    let separate = attr(f, "tsgwSeparateLogin").eq_ignore_ascii_case("true");
                    let (gd, gu) = split_user(&attr(f, "tsgwUsername"), &attr(f, "tsgwDomain"));
                    Gateway { host: gw_host, username: gu, password: dec(&attr(f, "tsgwPassword")), domain: gd, same: !separate }
                });
                let path: Vec<String> = attr(f, "tags")
                    .split(',')
                    .map(|t| t.trim().to_string())
                    .find(|t| !t.is_empty())
                    .into_iter()
                    .collect();
                add_folder(&path);
                let port: Option<u16> = attr(f, "port").parse().ok();
                conns.push(Conn {
                    path,
                    name: if name.is_empty() { attr(f, "serverName") } else { name },
                    protocol: proto.into(),
                    host: attr(f, "serverName"),
                    port: port.filter(|p| *p != default_port(proto)),
                    username: user,
                    password: c.password,
                    domain,
                    notes: notes(&attr(f, "notes")),
                    opts: BTreeMap::new(),
                    gateway,
                });
            }
        }
    }
    if conns.is_empty() && skipped.is_empty() {
        return Err("В файлах не найдено подключений Terminals. Выберите Favorites.xml (и Credentials.xml, Terminals.config) или старый Terminals.config".into());
    }
    if failed > 0 {
        skipped.push(format!("Не удалось расшифровать значений: {} — эти пароли нужно будет ввести вручную", failed));
    }
    Ok(Import { conns, folders, skipped })
}

fn default_port(p: &str) -> u16 {
    match p {
        "ssh" => 22,
        "telnet" => 23,
        _ => 3389,
    }
}

/// Папка данных Terminals по умолчанию, если в ней есть файлы
pub fn default_dir() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    let d = std::path::Path::new(&base).join(r"Robert_Chartier\Terminals\Data");
    ["Favorites.xml", "Terminals.config"].iter().any(|f| d.join(f).is_file()).then_some(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockEncryptMut;

    fn enc(key: &[u8; 32], s: &str) -> String {
        let iv = [7u8; 16];
        let data: Vec<u8> = s.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
        let ct = cbc::Encryptor::<aes::Aes256>::new_from_slices(key, &iv).unwrap().encrypt_padded_vec_mut::<Pkcs7>(&data);
        B64.encode([iv.to_vec(), ct].concat())
    }

    fn stored(pw: &str) -> (String, [u8; 32]) {
        let (ks, vs) = ([1u8; 16], [2u8; 16]);
        let vk = pbkdf2_key(pw, &vs);
        (B64.encode([ks.to_vec(), vs.to_vec(), vk.to_vec()].concat()), pbkdf2_key(pw, &ks))
    }

    #[test]
    fn new_format_with_master_password() {
        let (st, k) = stored("мастер");
        let config = format!(r#"<configuration><settings terminalsPassword="{}" /></configuration>"#, st);
        let creds = format!(
            r#"<ArrayOfCredentialSet><CredentialSet id="aaaaaaaa-0000-0000-0000-000000000001"><Name>Админ</Name>
            <EncryptedUserName>{}</EncryptedUserName><EncryptedDomain>{}</EncryptedDomain><EncryptedPassword>{}</EncryptedPassword>
            </CredentialSet></ArrayOfCredentialSet>"#,
            enc(&k, "admin"), enc(&k, "CORP"), enc(&k, "Пароль-1")
        );
        let favs = format!(
            r#"<FavoritesFile xmlns="http://Terminals.codeplex.com" FileVerion="2.0"><Favorites>
            <Favorite id="f1"><Name>dc01</Name><Protocol>RDP</Protocol><Port>3389</Port><ServerName>10.0.0.1</ServerName>
              <Security><Credential>aaaaaaaa-0000-0000-0000-000000000001</Credential></Security><Notes>{}</Notes></Favorite>
            <Favorite id="f2"><Name>nas</Name><Protocol>SSH</Protocol><Port>2222</Port><ServerName>10.0.0.2</ServerName>
              <Security><EncryptedUserName>{}</EncryptedUserName><EncryptedPassword>{}</EncryptedPassword></Security></Favorite>
            <Favorite id="f3"><Name>vnc</Name><Protocol>VNC</Protocol><Port>5900</Port><ServerName>10.0.0.3</ServerName></Favorite>
            </Favorites><Groups><Group id="g1"><Name>Офис</Name></Group></Groups>
            <FavoritesInGroups><FavoritesInGroup groupId="g1"><Favorites><guid>f1</guid></Favorites></FavoritesInGroup></FavoritesInGroups>
            </FavoritesFile>"#,
            B64.encode("заметка"), enc(&k, "root"), enc(&k, "ssh-pass")
        );
        assert_eq!(parse(&config, &favs, &creds, None).unwrap_err(), NEED_PASSWORD);
        assert!(parse(&config, &favs, &creds, Some("не тот")).unwrap_err().contains("Неверный"));
        let r = parse(&config, &favs, &creds, Some("мастер")).unwrap();
        assert_eq!(r.conns.len(), 2);
        let dc = &r.conns[0];
        assert_eq!((dc.username.as_str(), dc.domain.as_str(), dc.password.as_str()), ("admin", "CORP", "Пароль-1"));
        assert_eq!(dc.path, vec!["Офис"]);
        assert_eq!(dc.notes, "заметка");
        assert_eq!(dc.port, None);
        let nas = &r.conns[1];
        assert_eq!((nas.protocol.as_str(), nas.port, nas.password.as_str()), ("ssh", Some(2222), "ssh-pass"));
        assert_eq!(r.skipped, vec!["vnc (VNC)"]);
    }

    #[test]
    fn old_config_format() {
        let (st, k) = stored("pw");
        let config = format!(
            r#"<configuration><settings terminalsPassword="{}"><favorites>
            <add name="srv" serverName="srv.corp" protocol="RDP" port="3390" userName="CORP\ivan" encryptedPassword="{}" tags="Серверы,Прочее"
                 tsgwUsageMethod="1" tsgwHostname="gw.corp" tsgwSeparateLogin="true" tsgwUsername="gwuser" tsgwPassword="{}" />
            </favorites></settings></configuration>"#,
            st, enc(&k, "p1"), enc(&k, "gwp")
        );
        let r = parse(&config, "", "", Some("pw")).unwrap();
        let c = &r.conns[0];
        assert_eq!((c.username.as_str(), c.domain.as_str(), c.password.as_str(), c.port), ("ivan", "CORP", "p1", Some(3390)));
        assert_eq!(c.path, vec!["Серверы"]);
        let g = c.gateway.as_ref().unwrap();
        assert_eq!((g.host.as_str(), g.username.as_str(), g.password.as_str(), g.same), ("gw.corp", "gwuser", "gwp", false));
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_without_master_password() {
        use windows::Win32::Foundation::{LocalFree, HLOCAL};
        use windows::Win32::Security::Cryptography::{CryptProtectData, CRYPT_INTEGER_BLOB};
        let iv = [9u8; 16];
        let data: Vec<u8> = "секрет".encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let ent = CRYPT_INTEGER_BLOB { cbData: 16, pbData: iv.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB::default();
        let blob = unsafe {
            CryptProtectData(&input, None, Some(&ent), None, None, 0, &mut out).unwrap();
            let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
            let _ = LocalFree(Some(HLOCAL(out.pbData as _)));
            v
        };
        let value = B64.encode([iv.to_vec(), blob].concat());
        let favs = format!(
            r#"<FavoritesFile><Favorites><Favorite id="x"><Name>a</Name><Protocol>RDP</Protocol><ServerName>h</ServerName>
            <Security><EncryptedPassword>{}</EncryptedPassword></Security></Favorite></Favorites></FavoritesFile>"#,
            value
        );
        let r = parse("<configuration><settings /></configuration>", &favs, "", None).unwrap();
        assert_eq!(r.conns[0].password, "секрет");
    }
}
