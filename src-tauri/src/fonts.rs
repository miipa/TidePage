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

fn u16be(b: &[u8], off: usize) -> u16 {
    ((b[off] as u16) << 8) | (b[off + 1] as u16)
}

fn utf16be(b: &[u8], off: usize, len: usize) -> Option<String> {
    if off + len > b.len() || len % 2 != 0 {
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
    let mut font_off = 0usize;
    if data.len() >= 16 && &data[0..4] == b"ttcf" {
        let num = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
        if num > 0 {
            font_off = u32::from_be_bytes([data[12], data[13], data[14], data[15]]) as usize;
        }
    }
    if font_off + 12 > data.len() {
        return None;
    }
    let num_tables = u16be(data, font_off + 4) as usize;
    let mut name_off = 0usize;
    let mut name_len = 0usize;
    for i in 0..num_tables {
        let r = font_off + 12 + i * 16;
        if r + 16 > data.len() {
            break;
        }
        if &data[r..r + 4] == b"name" {
            name_off = u32::from_be_bytes([data[r + 8], data[r + 9], data[r + 10], data[r + 11]]) as usize;
            name_len = u16be(data, r + 12) as usize;
            break;
        }
    }
    if name_off == 0 || name_off + name_len > data.len() {
        return None;
    }
    let count = u16be(data, name_off) as usize;
    let string_off = name_off + u16be(data, name_off + 2) as usize;
    let mut candidates: Vec<(u32, String)> = Vec::new();
    for i in 0..count {
        let rec = name_off + 6 + i * 12;
        if rec + 12 > name_off + name_len {
            break;
        }
        let platform = u16be(data, rec);
        let encoding = u16be(data, rec + 2);
        let language = u16be(data, rec + 4);
        let name_id = u16be(data, rec + 6);
        let length = u16be(data, rec + 8) as usize;
        let off = string_off + u16be(data, rec + 10) as usize;
        if name_id != 1 && name_id != 16 {
            continue;
        }
        if off + length > data.len() {
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
    let dir = fonts_dir(&app);
    let file_name = src
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .ok_or("路径无效")?
        .to_lowercase();
    let stem = file_name
        .rsplit_once('.')
        .map(|(s, _)| s.to_string())
        .unwrap_or_else(|| file_name.clone());
    let out = dir.join(&file_name);
    std::fs::write(&out, data).map_err(|e| e.to_string())?;
    let family = font_family_from_bytes(&std::fs::read(&out).unwrap_or_default())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .unwrap_or(stem);
    Ok(family)
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

pub fn font_css_face(app: &tauri::AppHandle) -> String {
    let mut css = String::new();
    for (family, file) in installed_custom_fonts(app) {
        css.push_str(&format!(
            "@font-face {{ font-family: \"{family}\"; src: url('http://tidepage.localhost/fonts/{file}'); }}"
        ));
    }
    css
}
