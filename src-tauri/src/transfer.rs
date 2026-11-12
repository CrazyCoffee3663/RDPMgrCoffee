//! Зашифрованный экспорт подключений вместе с логинами и паролями (для переноса
//! между хранилищами) и его импорт.
//!
//! Файл: JSON { format, version, kdf: {alg, m, t, p, salt}, data }, где data — EncString
//! (AES-256-CBC + HMAC-SHA256, как в Bitwarden), ключ 64 байта — Argon2id от пароля экспорта.
//! Внутри data — тот же JSON, что у обычного экспорта, но с логинами, паролями и заметками.

use crate::bw::crypto::SymKey;
use crate::bw::Vault;
use crate::conn;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rand::RngCore;
use serde_json::{json, Value};
use zeroize::Zeroize;

pub const FORMAT: &str = "rdpmgrcoffee-encrypted";
const M_KIB: u32 = 64 * 1024;
const T: u32 = 3;
const P: u32 = 4;

fn derive(password: &str, salt: &[u8], m: u32, t: u32, p: u32) -> Result<SymKey, String> {
    if !(8 * 1024..=1024 * 1024).contains(&m) || !(1..=20).contains(&t) || !(1..=16).contains(&p) {
        return Err("Недопустимые параметры ключа в файле".into());
    }
    let params = argon2::Params::new(m, t, p, Some(64)).map_err(|e| e.to_string())?;
    let mut out = [0u8; 64];
    argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, &mut out)
        .map_err(|e| e.to_string())?;
    let k = SymKey::from_bytes(&out);
    out.zeroize();
    k
}

/// Зашифровать текст паролем экспорта.
pub fn seal(plain: &str, password: &str) -> Result<String, String> {
    if password.chars().count() < 12 {
        return Err("Пароль экспорта должен быть не короче 12 символов".into());
    }
    let mut salt = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt);
    let key = derive(password, &salt, M_KIB, T, P)?;
    Ok(serde_json::to_string_pretty(&json!({
        "format": FORMAT,
        "version": 1,
        "kdf": { "alg": "argon2id", "m": M_KIB, "t": T, "p": P, "salt": B64.encode(salt) },
        "data": key.encrypt(plain.as_bytes()),
    }))
    .unwrap())
}

/// Расшифровать файл зашифрованного экспорта.
pub fn open(text: &str, password: &str) -> Result<String, String> {
    let v: Value = serde_json::from_str(text).map_err(|_| "Файл повреждён: это не JSON".to_string())?;
    if v["format"] != json!(FORMAT) {
        return Err("Это не зашифрованный экспорт RDPMgrCoffee".into());
    }
    let k = &v["kdf"];
    if k["alg"] != json!("argon2id") {
        return Err("Неизвестный способ шифрования файла".into());
    }
    let salt = B64.decode(k["salt"].as_str().unwrap_or("")).map_err(|_| "Файл повреждён".to_string())?;
    let n = |x: &Value| x.as_u64().unwrap_or(0) as u32;
    let key = derive(password, &salt, n(&k["m"]), n(&k["t"]), n(&k["p"]))?;
    let data = key
        .decrypt(v["data"].as_str().unwrap_or(""))
        .map_err(|_| "Неверный пароль файла (или файл повреждён)".to_string())?;
    String::from_utf8(data).map_err(|_| "Файл повреждён".to_string())
}

/// Дополнить записи обычного экспорта учётными данными из хранилища и зашифровать.
/// entries[i] соответствует ids[i]; notes — заметки записей (по тому же индексу).
pub async fn export(vault: &Vault, mut file: Value, ids: Vec<String>, password: &str) -> Result<String, String> {
    if password.chars().count() < 12 {
        return Err("Пароль экспорта должен быть не короче 12 символов".into());
    }
    let conns = file["conns"].as_array_mut().ok_or("Нет подключений")?;
    if conns.len() != ids.len() {
        return Err("Внутренняя ошибка: список записей не совпадает".into());
    }
    for (c, id) in conns.iter_mut().zip(ids) {
        let p = conn::from_vault(vault, &id).await?;
        c["username"] = json!(p.username);
        c["password"] = json!(p.password);
        if c["domain"].as_str().unwrap_or("").is_empty() {
            c["domain"] = json!(p.domain);
        }
        if let (Some((_, gu, gd, gp)), Some(g)) = (&p.gateway, c.get_mut("gateway").filter(|g| g.is_object())) {
            if g["same"] != json!(true) {
                g["username"] = json!(gu);
                g["domain"] = json!(gd);
                g["password"] = json!(gp);
            }
        }
    }
    let mut plain = serde_json::to_string(&file).unwrap();
    let out = seal(&plain, password);
    plain.zeroize();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open() {
        assert!(seal("x", "short").is_err());
        let f = seal(r#"{"a":"пароль-1"}"#, "export password 1").unwrap();
        assert!(!f.contains("пароль-1"));
        assert_eq!(open(&f, "export password 1").unwrap(), r#"{"a":"пароль-1"}"#);
        assert!(open(&f, "export password 2").is_err());
        let mut v: Value = serde_json::from_str(&f).unwrap();
        v["kdf"]["m"] = json!(1);
        assert!(open(&v.to_string(), "export password 1").is_err());
    }
}
