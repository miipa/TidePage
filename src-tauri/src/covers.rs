use base64::Engine;
use std::path::{Path, PathBuf};

pub fn b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

pub fn covers_dir(app: &tauri::AppHandle) -> PathBuf {
    use tauri::Manager;
    let dir = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("covers");
    std::fs::create_dir_all(&dir).ok();
    dir
}

pub fn serve_cover(base: &Path, name: &str) -> (String, Vec<u8>) {
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return ("image/svg+xml".into(), placeholder_svg("?", "").into_bytes());
    }
    let full = base.join(name);
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    };
    match std::fs::read(&full) {
        Ok(b) => (mime.into(), b),
        Err(_) => ("image/svg+xml".into(), placeholder_svg("📖", "").into_bytes()),
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn wrap_lines(s: &str, max: usize, max_lines: usize) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut lines: Vec<String> = Vec::new();
    let mut i = 0;
    while i < chars.len() && lines.len() < max_lines {
        let end = (i + max).min(chars.len());
        lines.push(chars[i..end].iter().collect());
        i = end;
    }
    if i < chars.len() && !lines.is_empty() {
        let last = lines.last_mut().unwrap();
        last.pop();
        last.push('…');
    }
    lines
}

pub fn placeholder_svg(title: &str, author: &str) -> String {
    let lines = wrap_lines(title, 10, 5);
    let mut tspans = String::new();
    for (i, l) in lines.iter().enumerate() {
        tspans.push_str(&format!(
            "<tspan x=\"30\" dy=\"{}\">{}</tspan>",
            if i == 0 { 0 } else { 44 },
            escape(l)
        ));
    }
    let author_tspan = if author.is_empty() {
        String::new()
    } else {
        format!(
            "<tspan x=\"30\" y=\"560\" font-size=\"26\" opacity=\"0.7\">{}</tspan>",
            escape(&author.chars().take(16).collect::<String>())
        )
    };
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="320" height="480" viewBox="0 0 320 480">
<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1">
<stop offset="0" stop-color="#4a6b8a"/><stop offset="1" stop-color="#22303f"/>
</linearGradient></defs>
<rect width="320" height="480" fill="url(#g)"/>
<rect x="14" y="14" width="292" height="452" fill="none" stroke="rgba(255,255,255,0.25)" stroke-width="2"/>
<text x="30" y="140" font-size="36" fill="#f0f0f0" font-family="sans-serif">{tspans}</text>
{author_tspan}
</svg>"##
    )
}

pub fn make_thumbnail(data: &[u8], out_png: &Path) -> Result<(), String> {
    let img = image::load_from_memory(data).map_err(|e| e.to_string())?;
    let thumb = img.thumbnail(320, 480);
    thumb.save(out_png).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn write_placeholder(out_svg: &Path, title: &str, author: &str) -> Result<(), String> {
    std::fs::write(out_svg, placeholder_svg(title, author))
        .map_err(|e| e.to_string())?;
    Ok(())
}
