//! 便携设置备份的加密（跨设备可解密）。
//!
//! 云端备份里的凭据用 **WebDAV 账号** 派生密钥加密：口令 = `user\u{0}pass`，
//! PBKDF2-HMAC-SHA256（30 万次迭代）派生 32 字节密钥，AES-256-GCM 认证加密。
//! 因此另一台电脑配置同一个 WebDAV 账号即可解密恢复；本机落盘仍走 DPAPI
//! （见 `secret.rs`），二者互不影响。
//!
//! 安全约束：错误信息一律不含明文与口令；`backup_open` 对迭代次数做上界限制，
//! 避免恶意 blob 造成 PBKDF2 计算夯死。

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;
use rand::RngCore;
use serde_json::json;

/// PBKDF2 迭代次数。
const ITER: u32 = 300_000;
/// 迭代次数上界（解密时防御 DoS）。
const ITER_MAX: u32 = 10_000_000;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

fn b64e(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}

fn b64d(s: &str) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::STANDARD
        .decode(s.as_bytes())
        .map_err(|_| "备份无法解密（WebDAV 账号不一致或备份损坏）".to_string())
}

/// 用 PBKDF2-HMAC-SHA256 派生 32 字节密钥。
fn derive_key(passphrase: &str, salt: &[u8], iter: u32) -> [u8; KEY_LEN] {
    let mut key = [0u8; KEY_LEN];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(passphrase.as_bytes(), salt, iter, &mut key);
    key
}

/// 本机设备名：COMPUTERNAME（Windows）优先，其次 HOSTNAME；非 [A-Za-z0-9_-] 替换为 `_`。
#[tauri::command]
pub fn host_name() -> String {
    let raw = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "tidepage-pc".to_string());
    let s: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        "tidepage-pc".to_string()
    } else {
        s
    }
}

/// 用口令派生的密钥加密 plain，返回可入 JSON 的封装串。
/// 错误信息不含 plain 与 passphrase。
fn seal_impl(plain: &str, passphrase: &str) -> Result<String, String> {
    let mut salt = [0u8; SALT_LEN];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);

    let key = derive_key(passphrase, &salt, ITER);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plain.as_bytes())
        .map_err(|_| "备份加密失败".to_string())?;

    Ok(json!({
        "kdf": "pbkdf2-sha256",
        "iter": ITER,
        "salt": b64e(&salt),
        "iv": b64e(&nonce_bytes),
        "ct": b64e(&ct),
    })
    .to_string())
}

/// 解析 backup_seal 产物并用口令派生密钥解密。失败返回可读中文错误（不含明文）。
fn open_impl(blob: &str, passphrase: &str) -> Result<String, String> {
    const FAIL: &str = "备份无法解密（WebDAV 账号不一致或备份损坏）";
    let v: serde_json::Value = serde_json::from_str(blob).map_err(|_| FAIL.to_string())?;
    if v.get("kdf").and_then(|x| x.as_str()) != Some("pbkdf2-sha256") {
        return Err(FAIL.to_string());
    }
    let iter = v.get("iter").and_then(|x| x.as_u64()).unwrap_or(ITER as u64) as u32;
    if iter == 0 || iter > ITER_MAX {
        return Err(FAIL.to_string());
    }
    let salt = b64d(v.get("salt").and_then(|x| x.as_str()).unwrap_or(""))?;
    let iv = b64d(v.get("iv").and_then(|x| x.as_str()).unwrap_or(""))?;
    let ct = b64d(v.get("ct").and_then(|x| x.as_str()).unwrap_or(""))?;
    if salt.is_empty() || iv.len() != NONCE_LEN || ct.is_empty() {
        return Err(FAIL.to_string());
    }

    let key = derive_key(passphrase, &salt, iter);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let nonce = Nonce::from_slice(&iv);
    let plain = cipher
        .decrypt(nonce, ct.as_ref())
        .map_err(|_| FAIL.to_string())?;
    String::from_utf8(plain).map_err(|_| FAIL.to_string())
}

/// 加密命令：PBKDF2 较重，放阻塞线程池避免占用主线程。
#[tauri::command]
pub async fn backup_seal(plain: String, passphrase: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || seal_impl(&plain, &passphrase))
        .await
        .map_err(|e| e.to_string())?
}

/// 解密命令：同上，放阻塞线程池。
#[tauri::command]
pub async fn backup_open(blob: String, passphrase: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || open_impl(&blob, &passphrase))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 加解密往返还原原文，且密文不含明文片段。
    #[test]
    fn seal_open_roundtrip() {
        let plain = r#"{"ai:1:apiKey":"sk-test","webdav:pass":"p@ss"}"#;
        let blob = seal_impl(plain, "user\u{0}pass").unwrap();
        assert!(!blob.contains("sk-test"), "封装不得含明文");
        let out = open_impl(&blob, "user\u{0}pass").unwrap();
        assert_eq!(out, plain);
    }

    /// 口令不一致时解密失败，且错误信息不含明文/口令。
    #[test]
    fn seal_open_wrong_passphrase_fails() {
        let blob = seal_impl("secret-data", "right").unwrap();
        let err = open_impl(&blob, "wrong").unwrap_err();
        assert!(err.contains("备份无法解密"));
        assert!(!err.contains("secret-data"));
        assert!(!err.contains("right"));
        assert!(!err.contains("wrong"));
    }

    /// 损坏 / 非法输入返回可读错误而非 panic。
    #[test]
    fn open_bad_blob_errors() {
        assert!(open_impl("not-json", "x").is_err());
        assert!(open_impl("{\"kdf\":\"other\"}", "x").is_err());
    }
}
