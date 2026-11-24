//! Параметры подключения: собираются из записи Bitwarden (или строки быстрого подключения).
//!
//! Формат записи-подключения (тип «Логин»):
//!   URI:    протокол://хост[:порт]   (rdp, ssh, telnet, serial)
//!   Логин/пароль — учётные данные; «домен\логин» тоже понимается.
//!   Пользовательские поля (имена без учёта регистра):
//!     credential — имя или id другой записи, откуда брать логин/пароль
//!     domain     — домен (RDP)
//!     gateway    — имя или id записи RD-шлюза (RDP); в записи шлюза поле
//!                  samecreds=true — брать логин/пароль самого подключения
//!     sshkey     — имя или id записи «SSH-ключ» (SSH)
//!     baud / sercfg — настройки COM-порта (serial)
//!     и любые параметры протокола, см. описание в README.

use crate::bw::{FieldV, Vault};
use std::collections::HashMap;

#[derive(Clone, Default)]
pub struct ConnParams {
    pub name: String,
    pub protocol: String,
    pub host: String,
    pub port: Option<u16>,
    pub username: String,
    pub password: String,
    pub domain: String,
    pub ssh_private_key: String,
    /// Поля записи (ключ в нижнем регистре)
    pub opts: HashMap<String, String>,
    /// Параметры RD-шлюза: адрес, логин, домен, пароль
    pub gateway: Option<(String, String, String, String)>,
    /// Промежуточный SSH-хост (jump host)
    pub jump: Option<Box<ConnParams>>,
}

impl ConnParams {
    pub fn opt(&self, k: &str) -> &str {
        self.opts.get(k).map(|s| s.as_str()).unwrap_or("")
    }
    pub fn flag(&self, k: &str) -> bool {
        is_truthy(self.opt(k))
    }
}

/// Единый список «истинных» значений флажков для всей программы (фронтенд зеркалит его
/// в flagValue): записи приходят из импорта и общих коллекций с разным написанием
pub fn is_truthy(v: &str) -> bool {
    matches!(v.trim().to_lowercase().as_str(), "1" | "true" | "yes" | "да" | "on")
}

pub struct ParsedUri {
    pub scheme: String,
    pub user: String,
    pub host: String,
    pub port: Option<u16>,
}

/// "ssh://user@host:22/path" → части. IPv6 — в квадратных скобках.
pub fn parse_uri(uri: &str) -> Option<ParsedUri> {
    let (scheme, rest) = uri.trim().split_once("://")?;
    let scheme = scheme.to_lowercase();
    let auth = rest.split(['/', '?', '#']).next().unwrap_or("");
    let (user, hp) = match auth.rsplit_once('@') {
        Some((u, h)) => (u.to_string(), h),
        None => (String::new(), auth),
    };
    let (host, port) = if let Some(r) = hp.strip_prefix('[') {
        let (h, after) = r.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(p) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => Some(p.parse::<u16>().ok()?),
            _ => None,
        };
        (h.to_string(), port)
    } else if scheme == "serial" {
        // serial://COM3 — порт как «хост», без номера порта
        (hp.to_string(), None)
    } else if hp.matches(':').count() >= 2 {
        // Голый IPv6: разбор по последнему двоеточию отрезал бы часть адреса
        // (fe80::1 → fe80: + порт 1). Порт у IPv6 возможен только в скобках
        (hp.to_string(), None)
    } else {
        match hp.rsplit_once(':') {
            Some((h, p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => {
                // Порт вне u16 — отказ, а не молчаливый порт по умолчанию
                (h.to_string(), Some(p.parse::<u16>().ok()?))
            }
            _ => (hp.to_string(), None),
        }
    };
    if host.is_empty() {
        return None;
    }
    Some(ParsedUri { scheme, user, host, port })
}

/// "ДОМЕН\логин" → (домен, логин). «логин@домен» (UPN) намеренно остаётся логином целиком:
/// FreeRDP сам понимает UPN при NLA, а разделение по @ сломало бы такие входы.
pub fn split_user(raw: &str) -> (String, String) {
    let raw = raw.trim();
    if let Some((d, u)) = raw.split_once('\\') {
        return (d.to_string(), u.to_string());
    }
    (String::new(), raw.to_string())
}

pub const PROTOCOLS: [&str; 8] = ["rdp", "ssh", "telnet", "serial", "winbox", "vnc", "http", "https"];

fn fields_map(fields: &[FieldV]) -> HashMap<String, String> {
    fields
        .iter()
        .map(|f| {
            (
                f.name.trim().to_lowercase(),
                f.value.clone().unwrap_or_default().trim().to_string(),
            )
        })
        .collect()
}

/// Собрать параметры подключения из записи хранилища.
pub async fn from_vault(vault: &Vault, id: &str) -> Result<ConnParams, String> {
    from_vault_depth(vault, id, 0).await
}

/// depth > 0 — это промежуточный хост: его собственный jump не раскрываем (без циклов).
async fn from_vault_depth(vault: &Vault, id: &str, depth: u8) -> Result<ConnParams, String> {
    let cs = vault.conn_secret(id).await?;
    let uri = cs
        .item
        .uris
        .iter()
        .find_map(|u| parse_uri(u).filter(|p| PROTOCOLS.contains(&p.scheme.as_str())))
        .ok_or_else(|| format!("У записи нет адреса подключения (поддерживаются схемы: {})", PROTOCOLS.join(", ")))?;
    let opts = fields_map(&cs.fields);

    let mut username = if cs.item.username.is_empty() { uri.user.clone() } else { cs.item.username.clone() };
    let mut password = cs.password.clone();
    let mut domain = opts.get("domain").cloned().unwrap_or_default();
    let mut ssh_key = cs.ssh_private_key.clone();

    // Учётные данные из другой записи
    if let Some(cred) = opts.get("credential").filter(|c| !c.is_empty()) {
        let cid = vault.resolve_ref(cred).await?;
        let c = vault.conn_secret(&cid).await?;
        if !c.item.username.is_empty() {
            username = c.item.username.clone();
        }
        password = c.password.clone();
        if !c.ssh_private_key.is_empty() {
            ssh_key = c.ssh_private_key.clone();
        }
        if domain.is_empty() {
            domain = fields_map(&c.fields).get("domain").cloned().unwrap_or_default();
        }
    }

    // SSH-ключ из отдельной записи
    if let Some(k) = opts.get("sshkey").filter(|c| !c.is_empty()) {
        let kid = vault.resolve_ref(k).await?;
        let c = vault.conn_secret(&kid).await?;
        ssh_key = if !c.ssh_private_key.is_empty() {
            c.ssh_private_key
        } else if c.item.notes.contains("PRIVATE KEY") {
            c.item.notes.clone()
        } else {
            return Err(format!("В записи «{}» нет SSH-ключа", c.item.name));
        };
    }

    let (d, u) = split_user(&username);
    if !d.is_empty() {
        domain = d;
        username = u;
    }

    // RD-шлюз
    let mut gateway = None;
    if uri.scheme == "rdp" {
        if let Some(g) = opts.get("gateway").filter(|g| !g.is_empty()) {
            let gid = vault.resolve_ref(g).await?;
            let gw = vault.conn_secret(&gid).await?;
            let addr = gw
                .item
                .uris
                .first()
                .map(|u| {
                    parse_uri(u)
                        .map(|p| {
                            // IPv6 с портом — в скобках: freerdp_parse_hostname сохраняет скобки,
                            // потребители адреса шлюза используют его как есть
                            if p.host.contains(':') {
                                match p.port {
                                    Some(port) => format!("[{}]:{}", p.host, port),
                                    None => p.host,
                                }
                            } else {
                                match p.port {
                                    Some(port) => format!("{}:{}", p.host, port),
                                    None => p.host,
                                }
                            }
                        })
                        .unwrap_or_else(|| u.clone())
                })
                .unwrap_or_default();
            if addr.is_empty() {
                return Err(format!("У записи шлюза «{}» не указан адрес", gw.item.name));
            }
            let gf = fields_map(&gw.fields);
            let same = gf.get("samecreds").map(|v| is_truthy(v)).unwrap_or(false);
            // «Те же данные»: шлюз авторизуется учёткой самого подключения.
            // Так же поступаем, если у записи шлюза нет ни логина, ни пароля.
            if same || (gw.item.username.trim().is_empty() && gw.password.is_empty()) {
                gateway = Some((addr, username.clone(), domain.clone(), password.clone()));
            } else {
                let (gd, gu) = split_user(&gw.item.username);
                let gd = if gd.is_empty() { gf.get("domain").cloned().unwrap_or_default() } else { gd };
                gateway = Some((addr, gu, gd, gw.password));
            }
        }
    }

    // Промежуточный SSH-хост
    let mut jump = None;
    if uri.scheme == "ssh" && depth == 0 {
        if let Some(j) = opts.get("jump").filter(|j| !j.is_empty()) {
            let jid = vault.resolve_ref(j).await?;
            if jid == id {
                return Err("Подключение не может быть промежуточным хостом само для себя".into());
            }
            let jp = Box::pin(from_vault_depth(vault, &jid, 1)).await?;
            if jp.protocol != "ssh" {
                return Err(format!("Промежуточный хост «{}» должен быть SSH-подключением", jp.name));
            }
            jump = Some(Box::new(jp));
        }
    }

    Ok(ConnParams {
        jump,
        name: cs.item.name.clone(),
        protocol: uri.scheme,
        host: uri.host,
        port: uri.port,
        username,
        password,
        domain,
        ssh_private_key: ssh_key,
        opts,
        gateway,
    })
}

/// Быстрое подключение по строке: "ssh://user@host:22", "rdp://host" или просто "host".
pub fn from_quick(input: &str, default_proto: &str) -> Result<ConnParams, String> {
    let input = input.trim();
    let full = if input.contains("://") {
        input.to_string()
    } else {
        format!("{}://{}", default_proto, input)
    };
    let p = parse_uri(&full).ok_or("Не удалось разобрать адрес")?;
    if !PROTOCOLS.contains(&p.scheme.as_str()) {
        return Err(format!("Протокол {} не поддерживается", p.scheme));
    }
    let (domain, username) = split_user(&p.user);
    Ok(ConnParams {
        name: input.to_string(),
        protocol: p.scheme,
        host: p.host,
        port: p.port,
        username,
        domain,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris() {
        let p = parse_uri("ssh://root@10.0.0.1:2222/x").unwrap();
        assert_eq!((p.scheme.as_str(), p.user.as_str(), p.host.as_str(), p.port), ("ssh", "root", "10.0.0.1", Some(2222)));
        let p = parse_uri("rdp://[fe80::1]:3390").unwrap();
        assert_eq!((p.host.as_str(), p.port), ("fe80::1", Some(3390)));
        let p = parse_uri("serial://COM3").unwrap();
        assert_eq!(p.host, "COM3");
        assert!(parse_uri("rdp://").is_none());
        assert_eq!(split_user("CORP\\ivan"), ("CORP".into(), "ivan".into()));
    }

    #[test]
    fn quick() {
        let c = from_quick("admin@srv", "ssh").unwrap();
        assert_eq!((c.protocol.as_str(), c.username.as_str(), c.host.as_str()), ("ssh", "admin", "srv"));
        assert!(from_quick("ftp://x", "ssh").is_err());
    }

    /// v33: голый IPv6 не режется по двоеточию; порт вне u16 — отказ; userinfo с @
    #[test]
    fn uri_ipv6_ports_userinfo() {
        let p = parse_uri("rdp://fe80::1").unwrap();
        assert_eq!((p.host.as_str(), p.port), ("fe80::1", None));
        let p = parse_uri("rdp://[fe80::1]").unwrap();
        assert_eq!((p.host.as_str(), p.port), ("fe80::1", None));
        let p = parse_uri("rdp://[fe80::1]:3390").unwrap();
        assert_eq!((p.host.as_str(), p.port), ("fe80::1", Some(3390)));
        assert!(parse_uri("rdp://h:99999").is_none(), "порт вне u16 принят");
        assert!(parse_uri("rdp://[::1]:99999").is_none(), "порт вне u16 в скобках принят");
        let p = parse_uri("ssh://u@d@h:22").unwrap();
        assert_eq!((p.user.as_str(), p.host.as_str(), p.port), ("u@d", "h", Some(22)));
        assert_eq!(parse_uri("ssh://h:022").unwrap().port, Some(22));
        assert_eq!(parse_uri("ssh://h:").unwrap().host, "h:");
        // serial не режется даже с двоеточиями
        assert_eq!(parse_uri("serial://COM3").unwrap().host, "COM3");
    }

    /// v37: единый список истинных значений
    #[test]
    fn truthy_list() {
        for v in ["1", "true", "TRUE", "yes", "да", "on", " true "] {
            assert!(is_truthy(v), "{} не истинно", v);
        }
        for v in ["", "0", "false", "no", "нет", "off", "2"] {
            assert!(!is_truthy(v), "{} истинно", v);
        }
    }
}

/// v79а: from_vault на настоящем локальном хранилище (учётки, шлюз, ключ, jump)
#[cfg(test)]
mod vault_tests {
    use super::*;
    use crate::bw::ItemInput;

    fn input(name: &str, uris: Vec<String>, fields: Vec<FieldV>, username: &str, password: Option<&str>) -> ItemInput {
        ItemInput {
            id: None,
            kind: Some(1),
            org_id: None,
            collection_ids: vec![],
            folder_id: None,
            name: name.into(),
            notes: String::new(),
            username: username.into(),
            password: password.map(String::from),
            uris,
            fields,
            favorite: false,
        }
    }

    fn field(name: &str, value: &str) -> FieldV {
        FieldV { name: name.into(), value: Some(value.into()), kind: 0 }
    }

    #[tokio::test]
    async fn from_vault_credential_gateway_key_jump() {
        let dir = std::env::temp_dir().join(format!("connmgr-fromvault-{}", uuid::Uuid::new_v4()));
        let v = Vault::new(dir.clone());
        v.create_local("correct horse battery").await.unwrap();

        let cred = v.save_item(input("Учётка", vec![], vec![], "CORP\\svc", Some("cred-pw"))).await.unwrap();
        // samecreds=1 — истинно по общему списку (v37), шлюз берёт учётку подключения
        let _gw = v.save_item(input("Шлюз", vec!["rdgw://gw.corp:443".into()], vec![field("samecreds", "1")], "", None)).await.unwrap();
        let conn = v.save_item(input(
            "Сервер",
            vec!["rdp://srv:3390".into()],
            vec![field("credential", &cred.id), field("gateway", "Шлюз")],
            "",
            None,
        )).await.unwrap();
        let p = from_vault(&v, &conn.id).await.unwrap();
        assert_eq!((p.host.as_str(), p.port), ("srv", Some(3390)));
        assert_eq!((p.username.as_str(), p.domain.as_str(), p.password.as_str()), ("svc", "CORP", "cred-pw"));
        let (addr, gu, gd, gp) = p.gateway.expect("шлюз должен раскрыться");
        assert_eq!((addr.as_str(), gu.as_str(), gd.as_str(), gp.as_str()), ("gw.corp:443", "svc", "CORP", "cred-pw"));

        // SSH-ключ из заметки записи kind=5
        let mut key_input = input("Ключ", vec![], vec![], "", None);
        key_input.kind = Some(5);
        key_input.notes = "-----BEGIN OPENSSH PRIVATE KEY-----\ntest\n-----END OPENSSH PRIVATE KEY-----".into();
        let _key = v.save_item(key_input).await.unwrap();
        let sshconn = v.save_item(input("SSH", vec!["ssh://host".into()], vec![field("sshkey", "Ключ")], "root", None)).await.unwrap();
        let sp = from_vault(&v, &sshconn.id).await.unwrap();
        assert!(sp.ssh_private_key.contains("PRIVATE KEY"));

        // jump раскрывается, собственный jump промежуточного хоста — нет
        let jump = v.save_item(input("Jump", vec!["ssh://jump.host:2222".into()], vec![], "juser", Some("jpw"))).await.unwrap();
        let jc = v.save_item(input("Through", vec!["ssh://final".into()], vec![field("jump", &jump.id)], "", None)).await.unwrap();
        let jp = from_vault(&v, &jc.id).await.unwrap();
        let j = jp.jump.expect("jump должен раскрыться");
        assert_eq!((j.host.as_str(), j.port, j.username.as_str()), ("jump.host", Some(2222), "juser"));
        assert!(j.jump.is_none());

        // Сам себе промежуточный хост — отказ
        let selfj2 = v.save_item(input("Self2", vec!["ssh://self2".into()], vec![field("jump", "Self2")], "", None)).await.unwrap();
        assert!(from_vault(&v, &selfj2.id).await.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
