use std::path::PathBuf;

#[derive(serde::Serialize)]
pub struct FontInfo {
    pub family: String,
}

#[derive(serde::Serialize)]
pub struct CustomFontInfo {
    pub family: String,
    pub file: String,
}

/// 读 2 字节大端；越界返回 None（字体字节来自用户文件，任何偏移都不能直接下标 panic）。
fn u16be(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(off)?, *b.get(off + 1)?]))
}

/// 读 4 字节大端；越界返回 None。
fn be32(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_be_bytes([
        *b.get(off)?,
        *b.get(off + 1)?,
        *b.get(off + 2)?,
        *b.get(off + 3)?,
    ]))
}

fn utf16be(b: &[u8], off: usize, len: usize) -> Option<String> {
    if off.checked_add(len)? > b.len() || len % 2 != 0 {
        return None;
    }
    let mut units = Vec::with_capacity(len / 2);
    for i in (0..len).step_by(2) {
        let idx = off + i;
        units.push(u16::from_be_bytes([b[idx], b[idx + 1]]));
    }
    Some(String::from_utf16_lossy(&units))
}

/// 从 TTF/OTF/TTC 字节中解析字体家族名（优先 Windows 平台，nameID 16 优于 1）
pub fn font_family_from_bytes(data: &[u8]) -> Option<String> {
    // 先校验容器魔数：任意字节（含损坏字体）只能返回 None，不得把非 sfnt 当表目录解析。
    let magic = data.get(0..4)?;
    let mut font_off = 0usize;
    if magic == b"ttcf" {
        // TTC：至少 16 字节头、字体数 > 0、首字体目录偏移在文件内。
        if data.len() < 16 || be32(data, 8)? == 0 {
            return None;
        }
        font_off = be32(data, 12)? as usize;
    } else if magic != b"\x00\x01\x00\x00"
        && magic != b"OTTO"
        && magic != b"true"
        && magic != b"typ1"
    {
        return None;
    }
    if font_off.checked_add(12)? > data.len() {
        return None;
    }
    let num_tables = u16be(data, font_off + 4)? as usize;
    let mut name_off = 0usize;
    let mut name_len = 0usize;
    for i in 0..num_tables {
        let Some(r) = font_off
            .checked_add(12)
            .and_then(|b| i.checked_mul(16).and_then(|k| b.checked_add(k)))
        else {
            break;
        };
        if r.checked_add(16).map_or(true, |e| e > data.len()) {
            break;
        }
        if data.get(r..r + 4) == Some(b"name") {
            name_off = be32(data, r + 8)? as usize;
            // 表记录里的 length 是 uint32；旧实现只读高 16 位（几乎恒为 0），
            // 导致随后的记录循环一条都扫不到、族名解析恒失败。这里按规范读满 4 字节。
            name_len = be32(data, r + 12)? as usize;
            break;
        }
    }
    // name 表头固定 6 字节（format/count/stringOffset），长度不足 6 的不能解析。
    if name_off == 0 || name_len < 6 || name_off.checked_add(name_len)? > data.len() {
        return None;
    }
    // name 表头：format(2) + count(2) + stringOffset(2)。旧实现把 format 当 count、
    // 把 count 当 stringOffset（整体错开一个字段），族名解析同样恒失败；按规范读。
    let count = u16be(data, name_off + 2)? as usize;
    let string_off = name_off.checked_add(u16be(data, name_off + 4)? as usize)?;
    if string_off > data.len() {
        return None;
    }
    let mut candidates: Vec<(u32, String)> = Vec::new();
    let name_end = name_off + name_len;
    for i in 0..count {
        let Some(rec) = name_off
            .checked_add(6)
            .and_then(|b| i.checked_mul(12).and_then(|k| b.checked_add(k)))
        else {
            break;
        };
        if rec.checked_add(12).map_or(true, |e| e > name_end) {
            break;
        }
        let platform = u16be(data, rec)?;
        let encoding = u16be(data, rec + 2)?;
        let language = u16be(data, rec + 4)?;
        let name_id = u16be(data, rec + 6)?;
        let length = u16be(data, rec + 8)? as usize;
        let off = string_off.checked_add(u16be(data, rec + 10)? as usize)?;
        if name_id != 1 && name_id != 16 {
            continue;
        }
        if off.checked_add(length).map_or(true, |e| e > data.len()) {
            continue;
        }
        let s = match (platform, encoding) {
            (3, 1) | (3, 10) | (0, _) => utf16be(data, off, length),
            (3, 0) => Some(
                String::from_utf8_lossy(&data[off..off + length]).into_owned(),
            ),
            (1, _) => Some(String::from_utf8_lossy(&data[off..off + length]).into_owned()),
            _ => None,
        };
        if let Some(s) = s {
            if !s.trim().is_empty() {
                let win = if platform == 3 { 0u32 } else { 1u32 };
                let ty = if name_id == 16 { 0u32 } else { 1u32 };
                let lang = if language == 0 || language == 0x0804 || language == 0x0409 {
                    0u32
                } else {
                    1u32
                };
                candidates.push((win << 8 | ty << 6 | lang << 4, s));
            }
        }
    }
    candidates.sort();
    candidates.into_iter().next().map(|(_, s)| s)
}

fn fonts_dir(app: &tauri::AppHandle) -> PathBuf {
    let dir = crate::data_dir(app).join("fonts");
    std::fs::create_dir_all(&dir).ok();
    dir
}

pub fn installed_custom_fonts(app: &tauri::AppHandle) -> Vec<(String, String)> {
    let dir = fonts_dir(app);
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            let is_font =
                name.ends_with(".ttf") || name.ends_with(".otf") || name.ends_with(".ttc");
            if is_font {
                let stem = name
                    .rsplit_once('.')
                    .map(|(s, _)| s.to_string())
                    .unwrap_or_else(|| name.clone());
                let path = e.path();
                let family = std::fs::read(&path)
                    .ok()
                    .and_then(|d| font_family_from_bytes(&d))
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| stem.clone());
                out.push((family, name));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

#[tauri::command]
pub fn list_system_fonts() -> Vec<FontInfo> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;
    let mut out: Vec<FontInfo> = Vec::new();
    let hk = RegKey::predef(HKEY_LOCAL_MACHINE);
    if let Ok(key) = hk.open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts") {
        for name in key.enum_values().flatten() {
            let family = name.0.split('(').next().unwrap_or(&name.0).trim().to_string();
            if !family.is_empty() && !out.iter().any(|f| f.family == family) {
                out.push(FontInfo { family });
            }
        }
    }
    if let Ok(uk) = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows NT\CurrentVersion\Fonts")
    {
        for name in uk.enum_values().flatten() {
            let family = name.0.split('(').next().unwrap_or(&name.0).trim().to_string();
            if !family.is_empty() && !out.iter().any(|f| f.family == family) {
                out.push(FontInfo { family });
            }
        }
    }
    out.sort_by(|a, b| a.family.to_lowercase().cmp(&b.family.to_lowercase()));
    out
}

#[tauri::command]
pub fn install_font_file(app: tauri::AppHandle, path: String) -> Result<String, String> {
    let src = PathBuf::from(&path);
    let data = std::fs::read(&src).map_err(|e| e.to_string())?;
    // 先解析再落盘：坏字体不写进 fonts 目录（旧实现先写盘后解析，坏文件会留在目录里，
    // 之后每次列字体都读取它）。族名直接复用解析结果，不再二次读盘。
    let family = font_family_from_bytes(&data)
        .filter(|s| !s.trim().is_empty())
        .ok_or("字体文件无法解析")?;
    let dir = fonts_dir(&app);
    let file_name = src
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .ok_or("路径无效")?
        .to_lowercase();
    let out = dir.join(&file_name);
    std::fs::write(&out, &data).map_err(|e| e.to_string())?;
    Ok(family.trim().to_string())
}

#[tauri::command]
pub fn list_custom_fonts(app: tauri::AppHandle) -> Vec<CustomFontInfo> {
    installed_custom_fonts(&app)
        .into_iter()
        .map(|(family, file)| CustomFontInfo { family, file })
        .collect()
}

#[tauri::command]
pub fn delete_custom_font(app: tauri::AppHandle, name: String) -> Result<(), String> {
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err("无效字体名".into());
    }
    let dir = fonts_dir(&app);
    for ext in [".ttf", ".otf", ".ttc"] {
        std::fs::remove_file(dir.join(format!("{name}{ext}"))).ok();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::font_family_from_bytes;

    /// 构造最小 TTF：sfnt 头 + 一张 name 表 + 一条 nameID 16 的 Windows UTF-16BE 记录。
    fn minimal_ttf(family: &str) -> Vec<u8> {
        let units: Vec<u16> = family.encode_utf16().collect();
        let mut name = Vec::with_capacity(units.len() * 2);
        for u in units {
            name.extend_from_slice(&u.to_be_bytes());
        }
        // name 表：format(2) + count(2) + stringOffset(2) + 一条 12 字节记录 + 字符串。
        let string_off: u16 = 6 + 12;
        let mut name_table = Vec::new();
        name_table.extend_from_slice(&0u16.to_be_bytes());
        name_table.extend_from_slice(&1u16.to_be_bytes());
        name_table.extend_from_slice(&string_off.to_be_bytes());
        name_table.extend_from_slice(&3u16.to_be_bytes()); // platformID: Windows
        name_table.extend_from_slice(&1u16.to_be_bytes()); // encodingID: Unicode BMP
        name_table.extend_from_slice(&0x0409u16.to_be_bytes()); // languageID: en-US
        name_table.extend_from_slice(&16u16.to_be_bytes()); // nameID: typographic family
        name_table.extend_from_slice(&(name.len() as u16).to_be_bytes());
        name_table.extend_from_slice(&0u16.to_be_bytes()); // offset in string storage
        name_table.extend_from_slice(&name);

        let mut out = Vec::new();
        out.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // sfnt version 1.0
        out.extend_from_slice(&1u16.to_be_bytes()); // numTables
        out.extend_from_slice(&0u16.to_be_bytes()); // searchRange
        out.extend_from_slice(&0u16.to_be_bytes()); // entrySelector
        out.extend_from_slice(&0u16.to_be_bytes()); // rangeShift
        out.extend_from_slice(b"name");
        out.extend_from_slice(&0u32.to_be_bytes()); // checksum
        out.extend_from_slice(&28u32.to_be_bytes()); // 表数据偏移
        out.extend_from_slice(&(name_table.len() as u32).to_be_bytes());
        assert_eq!(out.len(), 28);
        out.extend_from_slice(&name_table);
        out
    }

    #[test]
    fn parses_minimal_name_table() {
        let ttf = minimal_ttf("Test Family");
        assert_eq!(font_family_from_bytes(&ttf).as_deref(), Some("Test Family"));
    }

    /// 任意截断前缀都不能 panic（旧实现 u16be 直接下标，name 表短于 6 字节即越界）。
    #[test]
    fn every_prefix_does_not_panic() {
        let ttf = minimal_ttf("TidePage 测试字体");
        for n in 0..=ttf.len() {
            let _ = font_family_from_bytes(&ttf[..n]);
        }
    }

    #[test]
    fn rejects_bad_containers_and_short_name_table() {
        // 非 sfnt 魔数
        assert_eq!(font_family_from_bytes(b"abcd...."), None);
        assert_eq!(font_family_from_bytes(b""), None);
        // ttc：头不足 16 字节 / 字体数为 0 / 首字体目录偏移越界
        assert_eq!(font_family_from_bytes(b"ttcf"), None);
        let mut ttc = vec![0u8; 16];
        ttc[0..4].copy_from_slice(b"ttcf");
        assert_eq!(font_family_from_bytes(&ttc), None);
        ttc[8..12].copy_from_slice(&1u32.to_be_bytes());
        assert_eq!(font_family_from_bytes(&ttc), None);
        ttc[12..16].copy_from_slice(&1000u32.to_be_bytes());
        assert_eq!(font_family_from_bytes(&ttc), None);
        // name 表长度 2（不足表头 6 字节）⇒ 拒绝（表记录 length 是 u32）
        let mut short = minimal_ttf("Short");
        short[24..28].copy_from_slice(&2u32.to_be_bytes());
        assert_eq!(font_family_from_bytes(&short), None);
        // 表目录声明的 name 表偏移越界 ⇒ 拒绝
        let mut bad_off = minimal_ttf("BadOffset");
        bad_off[20..24].copy_from_slice(&0xFFFF_FF00u32.to_be_bytes());
        assert_eq!(font_family_from_bytes(&bad_off), None);
    }
}
