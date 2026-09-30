//! 凭据加密（DPAPI）。
//!
//! 敏感配置（AI/TTS/WebDAV 的密钥）在落盘前统一收集为一个 JSON，经 Windows
//! DPAPI 以「当前用户」为密钥加密，再 base64 后写入 settings 表的 `secret.config`。
//! 前端 localStorage 不再保留任何明文凭据；云端备份上传的也是该密文。
//!
//! 设计取舍：
//! - 明文只在本模块的入参/出参内存中出现，日志与错误信息一律不回显明文；
//! - 使用 `CRYPTPROTECT_UI_FORBIDDEN` 禁止任何交互式提示（后台静默完成）；
//! - 附加固定 entropy，使密文与同用户下其它程序的 DPAPI blob 相互隔离；
//! - 非 Windows 平台不提供实现，返回可读错误而非 panic（本应用仅发布 Windows）。

use base64::Engine;

/// settings 表中存放密文的键。
pub const SECRET_SETTING_KEY: &str = "secret.config";

/// 固定附加熵：绑定到本应用语义，避免跨程序误用。
const ENTROPY: &[u8] = b"FlowRead.secret.config.v1";

#[cfg(windows)]
mod dpapi {
    use super::ENTROPY;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN,
    };

    /// 构造 CRYPT_INTEGER_BLOB（借用 data 的字节，不转移所有权）。
    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        }
    }

    /// 把 DPAPI 输出 blob 拷出后立刻 LocalFree，避免泄漏。
    unsafe fn take_blob(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        let bytes = if out.pbData.is_null() || out.cbData == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec()
        };
        if !out.pbData.is_null() {
            let _ = LocalFree(Some(HLOCAL(out.pbData as *mut _)));
        }
        bytes
    }

    pub fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
        let data_in = blob(plain);
        let entropy = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB::default();
        // 描述串传 null（不写入额外元数据）。
        unsafe {
            CryptProtectData(
                &data_in,
                PCWSTR::null(),
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
            .map_err(|e| format!("凭据加密失败：{e}"))?;
        }
        Ok(unsafe { take_blob(out) })
    }

    pub fn unprotect(cipher: &[u8]) -> Result<Vec<u8>, String> {
        if cipher.is_empty() {
            return Err("凭据密文为空".into());
        }
        let data_in = blob(cipher);
        let entropy = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptUnprotectData(
                &data_in,
                None,
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
            .map_err(|_| "凭据解密失败（可能来自其它电脑或其它用户）".to_string())?;
        }
        Ok(unsafe { take_blob(out) })
    }
}

#[cfg(not(windows))]
mod dpapi {
    pub fn protect(_plain: &[u8]) -> Result<Vec<u8>, String> {
        Err("当前平台不支持用 DPAPI 加密凭据".into())
    }
    pub fn unprotect(_cipher: &[u8]) -> Result<Vec<u8>, String> {
        Err("当前平台不支持用 DPAPI 解密凭据".into())
    }
}

/// 加密明文并 base64（供内部复用与单元测试）。
fn encrypt_to_b64(plain: &str) -> Result<String, String> {
    let bytes = dpapi::protect(plain.as_bytes())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// base64 解码后解密（供内部复用与单元测试）。
fn decrypt_from_b64(b64: &str) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim().as_bytes())
        .map_err(|_| "凭据密文格式错误（base64 解码失败）".to_string())?;
    let plain = dpapi::unprotect(&bytes)?;
    String::from_utf8(plain).map_err(|_| "凭据解密结果不是合法文本".to_string())
}

/// 加密明文凭据并写入 settings 表的 `secret.config`（base64 密文）。
/// 失败返回可读错误，不 panic，也不回显明文。
#[tauri::command]
pub fn secret_write(
    state: tauri::State<'_, std::sync::Arc<crate::library::AppState>>,
    plain: String,
) -> Result<(), String> {
    let b64 = encrypt_to_b64(&plain)?;
    let conn = state.db.0.lock().unwrap();
    crate::db::set_setting(&conn, SECRET_SETTING_KEY, &b64);
    Ok(())
}

/// 读取 `secret.config` 的 base64 密文并解密；无记录返回 None。
/// 失败返回可读错误，不 panic，也不回显明文。
#[tauri::command]
pub fn secret_read(
    state: tauri::State<'_, std::sync::Arc<crate::library::AppState>>,
) -> Result<Option<String>, String> {
    let b64 = {
        let conn = state.db.0.lock().unwrap();
        crate::db::get_setting(&conn, SECRET_SETTING_KEY)
    };
    match b64 {
        None => Ok(None),
        Some(s) if s.trim().is_empty() => Ok(None),
        Some(s) => decrypt_from_b64(&s).map(Some),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 加密 → 解密应还原原文（仅 Windows 生效；非 Windows 下 protect 直接失败）。
    #[cfg(windows)]
    #[test]
    fn roundtrip_plaintext() {
        let plain = r#"{"ai:1:apiKey":"sk-test","webdav:pass":"p@ss"}"#;
        let b64 = encrypt_to_b64(plain).unwrap();
        assert_ne!(b64, plain, "密文不应等于明文");
        assert!(!b64.contains("sk-test"), "密文不得包含明文片段");
        assert_eq!(decrypt_from_b64(&b64).unwrap(), plain);
    }

    /// 密文被篡改时解密失败但不 panic（返回 Err）。
    #[cfg(windows)]
    #[test]
    fn tampered_cipher_fails_gracefully() {
        let b64 = encrypt_to_b64("secret").unwrap();
        // 改动首字符：它决定首个字节的高 6 位，必然改变解码后的密文。
        let first = b64.chars().next().unwrap();
        let flipped = if first == 'A' { 'B' } else { 'A' };
        let tampered: String = std::iter::once(flipped).chain(b64.chars().skip(1)).collect();
        assert_ne!(tampered, b64);
        assert!(decrypt_from_b64(&tampered).is_err());
    }

    /// 非 base64 输入应返回可读错误而非 panic。
    #[test]
    fn invalid_base64_errors() {
        assert!(decrypt_from_b64("！！！不是base64").is_err());
    }
}
