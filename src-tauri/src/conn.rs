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
        matches!(
            self.opt(k).to_lowercase().as_str(),
            "1" | "true" | "yes" | "да" | "on"
        )
    }
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
        let port = after.strip_prefix(':').and_then(|p| p.parse().ok());
        (h.to_string(), port)
    } else if scheme == "serial" {
        // serial://COM3 — порт как «хост», без номера порта
        (hp.to_string(), None)
    } else {
        match hp.rsplit_once(':') {
            Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => {
                (h.to_string(), p.parse().ok())
            }
            _ => (hp.to_string(), None),
        }
    };
    if host.is_empty() {
        return None;
    }
    Some(ParsedUri { scheme, user, host, port })
}

/// "ДОМЕН\логин" или "логин@домен" → (домен, логин).
pub fn split_user(raw: &str) -> (String, String) {
    let raw = raw.trim();
    if let Some((d, u)) = raw.split_once('\\') {
        return (d.to_string(), u.to_string());
    }
    (String::new(), raw.to_string())
}

pub const PROTOCOLS: [&str; 6] = ["rdp", "ssh", "telnet", "serial", "winbox", "vnc"];

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
        .ok_or("У записи нет адреса вида rdp://, ssh://, telnet://, serial:// или winbox://")?;
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
                        .map(|p| match p.port {
                            Some(port) => format!("{}:{}", p.host, port),
                            None => p.host,
                        })
                        .unwrap_or_else(|| u.clone())
                })
                .unwrap_or_default();
            if addr.is_empty() {
                return Err(format!("У записи шлюза «{}» не указан адрес", gw.item.name));
            }
            let gf = fields_map(&gw.fields);
            let same = matches!(
                gf.get("samecreds").map(|v| v.to_lowercase()).as_deref(),
                Some("1" | "true" | "yes" | "да" | "on")
            );
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
}
