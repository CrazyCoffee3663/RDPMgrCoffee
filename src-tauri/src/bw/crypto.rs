//! Криптография Bitwarden: вывод ключей, EncString (AES-256-CBC + HMAC-SHA256), RSA-OAEP.

use aes::Aes256;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use hmac::{Hmac, Mac};
use rand::RngCore;
use rsa::pkcs8::DecodePrivateKey;
use rsa::{Oaep, RsaPrivateKey};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

type HmacSha256 = Hmac<Sha256>;
type AesEnc = cbc::Encryptor<Aes256>;
type AesDec = cbc::Decryptor<Aes256>;

pub type CResult<T> = Result<T, String>;

/// Параметры функции вывода ключа из мастер-пароля.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Kdf {
    /// 0 = PBKDF2-SHA256, 1 = Argon2id
    pub kind: u8,
    pub iterations: u32,
    pub memory: Option<u32>,
    pub parallelism: Option<u32>,
}

/// Симметричный ключ: 32 байта шифрования + 32 байта MAC.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SymKey {
    pub enc: [u8; 32],
    pub mac: [u8; 32],
}

impl SymKey {
    pub fn from_bytes(b: &[u8]) -> CResult<Self> {
        if b.len() != 64 {
            return Err(format!("Неверная длина ключа: {} байт", b.len()));
        }
        let mut enc = [0u8; 32];
        let mut mac = [0u8; 32];
        enc.copy_from_slice(&b[..32]);
        mac.copy_from_slice(&b[32..]);
        Ok(Self { enc, mac })
    }

    #[allow(dead_code)]
    pub fn generate() -> Self {
        let mut b = [0u8; 64];
        rand::thread_rng().fill_bytes(&mut b);
        let k = Self::from_bytes(&b).expect("64 bytes");
        b.zeroize();
        k
    }

    #[allow(dead_code)]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(64);
        v.extend_from_slice(&self.enc);
        v.extend_from_slice(&self.mac);
        v
    }

    /// Расшифровать EncString типа 2 (или устаревшего типа 0) в байты.
    pub fn decrypt(&self, s: &str) -> CResult<Vec<u8>> {
        let (kind, rest) = s
            .split_once('.')
            .ok_or_else(|| "Повреждённые данные хранилища (EncString)".to_string())?;
        let parts: Vec<&str> = rest.split('|').collect();
        match kind {
            "2" => {
                if parts.len() != 3 {
                    return Err("Повреждённые данные хранилища (EncString)".into());
                }
                let iv = b64(parts[0])?;
                let ct = b64(parts[1])?;
                let mac = b64(parts[2])?;
                let mut m = <HmacSha256 as Mac>::new_from_slice(&self.mac).map_err(e2s)?;
                m.update(&iv);
                m.update(&ct);
                m.verify_slice(&mac)
                    .map_err(|_| "Неверный ключ или повреждённые данные (MAC)".to_string())?;
                aes_decrypt(&self.enc, &iv, &ct)
            }
            // Тип 0 — без кода целостности: подменённые данные расшифровались бы без ошибки
            // (атака на файл кеша/хранилища). Допустим только для ключа очень старых аккаунтов,
            // см. decrypt_user_key.
            "0" => Err("Данные без защиты целостности (тип 0) не принимаются".into()),
            other => Err(format!("Неподдерживаемый тип шифрования: {}", other)),
        }
    }

    pub fn decrypt_str(&self, s: &str) -> CResult<String> {
        let b = self.decrypt(s)?;
        String::from_utf8(b).map_err(|_| "Расшифрованный текст не в UTF-8".to_string())
    }

    /// Зашифровать в EncString типа 2.
    pub fn encrypt(&self, data: &[u8]) -> String {
        let mut iv = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut iv);
        let ct = AesEnc::new_from_slices(&self.enc, &iv)
            .expect("key/iv length")
            .encrypt_padded_vec_mut::<Pkcs7>(data);
        let mut m = <HmacSha256 as Mac>::new_from_slice(&self.mac).expect("hmac key");
        m.update(&iv);
        m.update(&ct);
        let mac = m.finalize().into_bytes();
        format!("2.{}|{}|{}", B64.encode(iv), B64.encode(ct), B64.encode(mac))
    }

    pub fn encrypt_str(&self, s: &str) -> String {
        self.encrypt(s.as_bytes())
    }
}

fn aes_decrypt(key: &[u8], iv: &[u8], ct: &[u8]) -> CResult<Vec<u8>> {
    AesDec::new_from_slices(key, iv)
        .map_err(e2s)?
        .decrypt_padded_vec_mut::<Pkcs7>(ct)
        .map_err(|_| "Ошибка расшифровки (padding)".to_string())
}

pub fn b64(s: &str) -> CResult<Vec<u8>> {
    B64.decode(s.trim())
        .map_err(|_| "Ошибка декодирования base64".to_string())
}

fn e2s<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Мастер-ключ (32 байта) из мастер-пароля.
pub fn master_key(password: &str, email: &str, kdf: &Kdf) -> CResult<[u8; 32]> {
    let email = email.trim().to_lowercase();
    let mut out = [0u8; 32];
    match kdf.kind {
        0 => {
            pbkdf2::pbkdf2_hmac::<Sha256>(
                password.as_bytes(),
                email.as_bytes(),
                kdf.iterations,
                &mut out,
            );
        }
        1 => {
            let salt = Sha256::digest(email.as_bytes());
            let mem_mib = kdf.memory.unwrap_or(64);
            let par = kdf.parallelism.unwrap_or(4);
            let params = argon2::Params::new(mem_mib * 1024, kdf.iterations, par, Some(32))
                .map_err(|e| format!("Параметры Argon2: {}", e))?;
            argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
                .hash_password_into(password.as_bytes(), &salt, &mut out)
                .map_err(|e| format!("Argon2: {}", e))?;
        }
        k => return Err(format!("Неизвестный тип KDF: {}", k)),
    }
    Ok(out)
}

/// Хеш мастер-пароля для аутентификации на сервере (base64).
pub fn master_password_hash(master_key: &[u8; 32], password: &str) -> String {
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(master_key, password.as_bytes(), 1, &mut out);
    B64.encode(out)
}

/// Растянутый мастер-ключ (HKDF-Expand "enc"/"mac").
pub fn stretch(master_key: &[u8; 32]) -> SymKey {
    let hk = hkdf::Hkdf::<Sha256>::from_prk(master_key).expect("prk length");
    let mut enc = [0u8; 32];
    let mut mac = [0u8; 32];
    hk.expand(b"enc", &mut enc).expect("hkdf");
    hk.expand(b"mac", &mut mac).expect("hkdf");
    SymKey { enc, mac }
}

/// Зашифровать ключ пользователя мастер-ключом (как в Bitwarden: растянутый ключ, EncString типа 2).
pub fn protect_user_key(master_key: &[u8; 32], user_key: &SymKey) -> String {
    let mut b = user_key.to_bytes();
    let out = stretch(master_key).encrypt(&b);
    b.zeroize();
    out
}

/// HMAC-SHA256 (ключ MAC ключа пользователя) — контрольная сумма файла кеша/хранилища.
pub fn file_mac(key: &SymKey, data: &[u8]) -> String {
    let mut m = <HmacSha256 as Mac>::new_from_slice(&key.mac).expect("hmac key");
    m.update(b"rdpmgrcoffee-cache-v1");
    m.update(data);
    B64.encode(m.finalize().into_bytes())
}

/// Проверка контрольной суммы (сравнение за постоянное время).
pub fn file_mac_ok(key: &SymKey, data: &[u8], mac: &str) -> bool {
    let Ok(tag) = B64.decode(mac.trim()) else { return false };
    let mut m = <HmacSha256 as Mac>::new_from_slice(&key.mac).expect("hmac key");
    m.update(b"rdpmgrcoffee-cache-v1");
    m.update(data);
    m.verify_slice(&tag).is_ok()
}

/// Расшифровать защищённый ключ пользователя мастер-ключом.
pub fn decrypt_user_key(master_key: &[u8; 32], protected: &str) -> CResult<SymKey> {
    let bytes = if protected.starts_with("0.") {
        // Очень старые аккаунты: ключ зашифрован самим мастер-ключом без MAC
        let parts: Vec<&str> = protected[2..].split('|').collect();
        if parts.len() != 2 {
            return Err("Повреждённый ключ пользователя".into());
        }
        aes_decrypt(master_key, &b64(parts[0])?, &b64(parts[1])?)?
    } else {
        stretch(master_key)
            .decrypt(protected)
            .map_err(|_| "Неверный мастер-пароль".to_string())?
    };
    SymKey::from_bytes(&bytes)
}

/// Приватный RSA-ключ пользователя (PKCS#8 DER, зашифрован ключом пользователя).
pub fn decrypt_private_key(user_key: &SymKey, enc: &str) -> CResult<RsaPrivateKey> {
    let der = user_key.decrypt(enc)?;
    RsaPrivateKey::from_pkcs8_der(&der).map_err(|e| format!("Приватный ключ: {}", e))
}

/// Расшифровать EncString типа 3/4 (RSA-OAEP) — ключи организаций.
pub fn rsa_decrypt(key: &RsaPrivateKey, s: &str) -> CResult<Vec<u8>> {
    let (kind, rest) = s.split_once('.').ok_or("Повреждённый ключ организации")?;
    let data = b64(rest.split('|').next().unwrap_or(""))?;
    match kind {
        "3" => key.decrypt(Oaep::new::<Sha256>(), &data).map_err(e2s),
        "4" => key.decrypt(Oaep::new::<sha1::Sha1>(), &data).map_err(e2s),
        k => Err(format!("Неподдерживаемый тип RSA-шифрования: {}", k)),
    }
}

/// Генератор паролей.
pub fn generate_password(len: usize, upper: bool, lower: bool, digits: bool, symbols: bool) -> String {
    let mut sets: Vec<&[u8]> = Vec::new();
    // Без похожих символов (0/O, 1/l/I)
    if upper {
        sets.push(b"ABCDEFGHJKLMNPQRSTUVWXYZ");
    }
    if lower {
        sets.push(b"abcdefghijkmnopqrstuvwxyz");
    }
    if digits {
        sets.push(b"23456789");
    }
    if symbols {
        sets.push(b"!@#$%^&*-_=+?");
    }
    if sets.is_empty() {
        sets.push(b"abcdefghijkmnopqrstuvwxyz");
    }
    let len = len.clamp(4, 128).max(sets.len());
    let all: Vec<u8> = sets.iter().flat_map(|s| s.iter().copied()).collect();
    let mut rng = rand::thread_rng();
    let pick = |set: &[u8], rng: &mut rand::rngs::ThreadRng| {
        set[(rng.next_u32() as usize) % set.len()]
    };
    // По одному символу из каждого набора, остальное — из общего
    let mut out: Vec<u8> = sets.iter().map(|s| pick(s, &mut rng)).collect();
    while out.len() < len {
        out.push(pick(&all, &mut rng));
    }
    // Перемешивание Фишера–Йетса
    for i in (1..out.len()).rev() {
        let j = (rng.next_u32() as usize) % (i + 1);
        out.swap(i, j);
    }
    String::from_utf8(out).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let k = SymKey::generate();
        let s = k.encrypt_str("привет, мир");
        assert!(s.starts_with("2."));
        assert_eq!(k.decrypt_str(&s).unwrap(), "привет, мир");
    }

    #[test]
    fn wrong_key_fails_mac() {
        let a = SymKey::generate();
        let b = SymKey::generate();
        let s = a.encrypt_str("secret");
        assert!(b.decrypt(&s).is_err());
    }

    #[test]
    fn pbkdf2_known_vector() {
        // RFC 7914, раздел 11: PBKDF2-HMAC-SHA256("passwd", "salt", 1)
        let mut out = [0u8; 16];
        pbkdf2::pbkdf2_hmac::<Sha256>(b"passwd", b"salt", 1, &mut out);
        let hex: String = out.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(hex, "55ac046e56e3089fec1691c22544b605");
    }

    #[test]
    fn user_key_unwrap() {
        let kdf = Kdf { kind: 0, iterations: 5000, memory: None, parallelism: None };
        let mk = master_key("pass", "User@Example.com ", &kdf).unwrap();
        let mk2 = master_key("pass", "user@example.com", &kdf).unwrap();
        assert_eq!(mk, mk2);
        let user = SymKey::generate();
        let protected = stretch(&mk).encrypt(&user.to_bytes());
        let back = decrypt_user_key(&mk, &protected).unwrap();
        assert_eq!(back.to_bytes(), user.to_bytes());
        let wrong = master_key("nope", "user@example.com", &kdf).unwrap();
        assert!(decrypt_user_key(&wrong, &protected).is_err());
    }

    #[test]
    fn password_generator() {
        let p = generate_password(20, true, true, true, true);
        assert_eq!(p.len(), 20);
        assert!(p.chars().any(|c| c.is_ascii_digit()));
    }
}
