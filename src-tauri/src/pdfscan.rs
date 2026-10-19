//! 超大扫描版 PDF 的「直读页面图」通道。
//!
//! 背景：3.50 GiB 这类整册扫描件，pdf.js 在本运行时无法表示（渲染层按整份长度分配），
//! `Windows.Data.Pdf` 对这种册子会永久不返回。但这类文件的结构其实极简：
//! 每页 = 一个 `q … cm /Im1 Do Q` 内容流 + 一个整页图像 XObject。
//! 于是这里只做一件事：**按页从文件里取出那张图**，不建全文索引、不整份读入。
//!
//! 解析范围（够用即止，遇不支持一律"判不支持"而不是硬解）：
//! - 经典 xref 表（含多子段、`/Prev` 链）；**遇 `/XRefStm` 直接判不支持**（PDF 1.5 混排 xref）。
//! - `/Pages` → `/Kids`：扁平直取；`/Type /Pages` 节点按 `/Count` 递归，支持按页号跳转。
//! - 显示盒 = `CropBox ∩ MediaBox`（缺 CropBox 回落 MediaBox）；`/Rotate` 只支持 0/90/180/270。
//! - 图像流：`DCTDecode` 直出原始 JPEG 字节；`FlateDecode` 回未解压 zlib 字节 + 参数，
//!   由前端用 WebView2 自带的 `DecompressionStream` 解（图像流不经 flate2）；
//!   内容流则由本模块用 flate2 解压后分析（见 decode_stream / analyze_content）。
//!
//! 缓存按 `book_id` 键控：多窗口会同时开不同的书，`pdf_scan_close` 只清对应那一份。

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use tauri::State;

use crate::library::AppState;

// ============================ 基础类型 ============================

type Dict = BTreeMap<String, Val>;

#[derive(Clone, Debug)]
enum Val {
    Null,
    Bool(bool),
    Num(f64),
    Str(Vec<u8>),
    Name(String),
    Arr(Vec<Val>),
    Dict(Dict),
    Ref(u32, u16),
    /// 流对象：字典 + 流数据在文件里的起始偏移（长度按字典里的 `/Length` 现解析）。
    Stream(Box<Dict>, u64),
}

impl Val {
    fn as_num(&self) -> Option<f64> {
        match self {
            Val::Num(n) => Some(*n),
            _ => None,
        }
    }
    fn as_name(&self) -> Option<&str> {
        match self {
            Val::Name(s) => Some(s.as_str()),
            _ => None,
        }
    }
    fn as_dict(&self) -> Option<&Dict> {
        match self {
            Val::Dict(d) => Some(d),
            Val::Stream(d, _) => Some(d),
            _ => None,
        }
    }
    fn as_arr(&self) -> Option<&[Val]> {
        match self {
            Val::Arr(a) => Some(a.as_slice()),
            _ => None,
        }
    }
}

fn dict_get<'a>(d: &'a Dict, k: &str) -> Option<&'a Val> {
    d.get(k)
}

// ============================ 词法/语法 ============================

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> P<'a> {
    fn new(b: &'a [u8]) -> Self {
        P { b, i: 0 }
    }
    fn ws(&mut self) {
        while self.i < self.b.len() {
            let c = self.b[self.i];
            if c == b'%' {
                while self.i < self.b.len() && self.b[self.i] != b'\n' && self.b[self.i] != b'\r' {
                    self.i += 1;
                }
            } else if c == b' ' || c == b'\t' || c == b'\r' || c == b'\n' || c == 0x0c || c == 0 {
                self.i += 1;
            } else {
                break;
            }
        }
    }
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }
    fn starts(&self, s: &[u8]) -> bool {
        self.b.len() >= self.i + s.len() && &self.b[self.i..self.i + s.len()] == s
    }
    fn num(&mut self) -> Option<f64> {
        let st = self.i;
        if self.peek() == Some(b'+') || self.peek() == Some(b'-') {
            self.i += 1;
        }
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == b'.' {
                self.i += 1;
            } else {
                break;
            }
        }
        if st == self.i {
            return None;
        }
        std::str::from_utf8(&self.b[st..self.i]).ok()?.parse().ok()
    }
    fn name(&mut self) -> Option<String> {
        if self.peek() != Some(b'/') {
            return None;
        }
        self.i += 1;
        let st = self.i;
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace() || matches!(c, b'/' | b'[' | b']' | b'<' | b'>' | b'(' | b')' | b'%' | b'{' | b'}') {
                break;
            }
            self.i += 1;
        }
        Some(String::from_utf8_lossy(&self.b[st..self.i]).into_owned())
    }
    fn lit_string(&mut self) -> Option<Vec<u8>> {
        if self.peek() != Some(b'(') {
            return None;
        }
        self.i += 1;
        let mut out = Vec::new();
        let mut depth = 1;
        while let Some(c) = self.peek() {
            self.i += 1;
            match c {
                b'\\' => {
                    if let Some(e) = self.peek() {
                        self.i += 1;
                        match e {
                            b'n' => out.push(b'\n'),
                            b'r' => out.push(b'\r'),
                            b't' => out.push(b'\t'),
                            b'(' => out.push(b'('),
                            b')' => out.push(b')'),
                            b'\\' => out.push(b'\\'),
                            b'0'..=b'7' => {
                                let mut v: u32 = (e - b'0') as u32;
                                for _ in 0..2 {
                                    if let Some(d) = self.peek() {
                                        if (b'0'..=b'7').contains(&d) {
                                            v = v * 8 + (d - b'0') as u32;
                                            self.i += 1;
                                        }
                                    }
                                }
                                out.push(v as u8);
                            }
                            _ => out.push(e),
                        }
                    }
                }
                b'(' => {
                    depth += 1;
                    out.push(c);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(c);
                }
                _ => out.push(c),
            }
        }
        Some(out)
    }
    fn hex_string(&mut self) -> Option<Vec<u8>> {
        if self.peek() != Some(b'<') || self.b.get(self.i + 1) == Some(&b'<') {
            return None;
        }
        self.i += 1;
        let mut s = String::new();
        while let Some(c) = self.peek() {
            self.i += 1;
            if c == b'>' {
                break;
            }
            if (c as char).is_ascii_hexdigit() {
                s.push(c as char);
            }
        }
        if s.len() % 2 == 1 {
            s.push('0');
        }
        let mut out = Vec::new();
        let bs = s.as_bytes();
        for i in (0..bs.len()).step_by(2) {
            let hi = (bs[i] as char).to_digit(16)? as u8;
            let lo = (bs[i + 1] as char).to_digit(16)? as u8;
            out.push(hi << 4 | lo);
        }
        Some(out)
    }
    fn value(&mut self, depth: usize) -> Option<Val> {
        if depth > 64 {
            return None;
        }
        self.ws();
        match self.peek()? {
            b'/' => self.name().map(Val::Name),
            b'(' => self.lit_string().map(Val::Str),
            b'[' => {
                self.i += 1;
                let mut v = Vec::new();
                loop {
                    self.ws();
                    match self.peek() {
                        Some(b']') => {
                            self.i += 1;
                            break;
                        }
                        None => break,
                        _ => v.push(self.value(depth + 1)?),
                    }
                }
                Some(Val::Arr(v))
            }
            b'<' if self.b.get(self.i + 1) == Some(&b'<') => {
                self.i += 2;
                let mut d = Dict::new();
                loop {
                    self.ws();
                    if self.starts(b">>") {
                        self.i += 2;
                        break;
                    }
                    match self.peek() {
                        None => break,
                        Some(b'/') => {
                            let k = self.name()?;
                            let v = self.value(depth + 1)?;
                            d.insert(k, v);
                        }
                        _ => {
                            // 容错：跳过无法识别的位置
                            self.i += 1;
                        }
                    }
                }
                // 流：`stream` 关键字（先跳过 \r\n / \n / \r）
                let save = self.i;
                self.ws();
                if self.starts(b"stream") {
                    self.i += 6;
                    if self.starts(b"\r\n") {
                        self.i += 2;
                    } else if self.peek() == Some(b'\n') || self.peek() == Some(b'\r') {
                        self.i += 1;
                    }
                    return Some(Val::Stream(Box::new(d), self.i as u64));
                }
                self.i = save;
                Some(Val::Dict(d))
            }
            b'<' => self.hex_string().map(Val::Str),
            b't' if self.starts(b"true") => {
                self.i += 4;
                Some(Val::Bool(true))
            }
            b'f' if self.starts(b"false") => {
                self.i += 5;
                Some(Val::Bool(false))
            }
            b'n' if self.starts(b"null") => {
                self.i += 4;
                Some(Val::Null)
            }
            c if c.is_ascii_digit() || c == b'+' || c == b'-' || c == b'.' => {
                let n = self.num()?;
                // 间接引用：`N G R`
                let save = self.i;
                self.ws();
                if self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                    if let Some(g) = self.num() {
                        self.ws();
                        if self.peek() == Some(b'R') {
                            self.i += 1;
                            if n >= 0.0 && n <= u32::MAX as f64 && g >= 0.0 {
                                return Some(Val::Ref(n as u32, g as u16));
                            }
                        }
                    }
                }
                self.i = save;
                Some(Val::Num(n))
            }
            _ => None,
        }
    }
}

/// 在给定字节里解析一个完整对象（忽略尾部的 `endobj`）。
fn parse_val(buf: &[u8]) -> Option<Val> {
    let mut p = P::new(buf);
    p.value(0)
}

// ============================ 文件与 xref ============================

struct Src {
    f: File,
    len: u64,
}

impl Src {
    fn open(path: &Path) -> Result<Src, String> {
        let f = File::open(path).map_err(|e| format!("打开失败: {e}"))?;
        let len = f.metadata().map_err(|e| e.to_string())?.len();
        Ok(Src { f, len })
    }
    fn read_at(&mut self, off: u64, n: usize) -> Result<Vec<u8>, String> {
        if off >= self.len {
            return Ok(Vec::new());
        }
        let n = n.min((self.len - off) as usize);
        self.f.seek(SeekFrom::Start(off)).map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; n];
        self.f.read_exact(&mut buf).map_err(|e| e.to_string())?;
        Ok(buf)
    }
}

#[derive(Default)]
struct Xref {
    /// obj -> (offset, 可选代数)
    map: HashMap<u32, u64>,
    trailer: Dict,
}

fn find_last(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).rev().find(|&i| &hay[i..i + needle.len()] == needle)
}

impl Xref {
    /// 解析经典 xref + `/Prev` 链；遇 `/XRefStm` 判不支持。
    fn parse(src: &mut Src) -> Result<Xref, String> {
        let tail_len = 2048.min(src.len) as usize;
        let tail = src.read_at(src.len - tail_len as u64, tail_len)?;
        let pos = find_last(&tail, b"startxref").ok_or("缺少 startxref")?;
        let rest = &tail[pos + 9..];
        let mut p = P::new(rest);
        p.ws();
        let mut off = p.num().ok_or("startxref 偏移无法解析")? as u64;
        let mut out = Xref::default();
        let mut first = true;
        let mut guard = 0;
        loop {
            guard += 1;
            if guard > 64 {
                return Err("xref /Prev 链过深".into());
            }
            // `off` 指向 `xref` 关键字（也容忍前面有 1–2 字节空白）。
            let win_start = off.saturating_sub(2);
            let head = src.read_at(win_start, 32)?;
            let at = match find_last(&head, b"xref") {
                Some(i) => i,
                None => {
                    // 该偏移处不是经典 `xref` 表：很可能是**PDF 1.5 的 xref 流**（`N G obj` + `/Type /XRef`），
                    // 对象流（/ObjStm）就走这条路过不来。本实现不支持，明确区分原因码，
                    // 供日志统计（不静默当成"文件坏了"）。
                    let looks_like_obj = head
                        .iter()
                        .take(8)
                        .any(|b| b.is_ascii_digit());
                    if looks_like_obj {
                        return Err("不支持：xref 流/对象流（/XRef 或 /ObjStm）".into());
                    }
                    return Err(format!("xref 表不在预期偏移（{off}）"));
                }
            };
            let abs = win_start + at as u64;
            // xref 子段可能很长（本册 31,876 条 ≈ 640KB）：一次读 4MB，不够再翻倍。
            let mut want = 4 * 1024 * 1024usize;
            let mut blk;
            loop {
                blk = src.read_at(abs, want)?;
                if find_last(&blk, b"trailer").is_some() || blk.len() < want || want > 64 * 1024 * 1024 {
                    break;
                }
                want *= 2;
            }
            let tr_at = find_last(&blk, b"trailer").ok_or("xref 表后缺少 trailer")?;
            let body = &blk[4..tr_at.min(blk.len())];
            let mut bp = P::new(body);
            loop {
                bp.ws();
                if bp.i >= body.len() {
                    break;
                }
                if bp.peek() == Some(b't') {
                    break;
                }
                let start = match bp.num() {
                    Some(n) => n as u32,
                    None => break,
                };
                bp.ws();
                let count = match bp.num() {
                    Some(n) => n as usize,
                    None => break,
                };
                for k in 0..count {
                    bp.ws();
                    let o = match bp.num() {
                        Some(n) => n,
                        None => break,
                    };
                    bp.ws();
                    let _gen = bp.num();
                    bp.ws();
                    let kind = bp.peek();
                    bp.i += 1;
                    if kind == Some(b'n') {
                        let obj = start + k as u32;
                        out.map.entry(obj).or_insert(o as u64);
                    }
                }
            }
            let tr = &blk[tr_at + 7..];
            let tdict = match parse_val(tr) {
                Some(v) => v.as_dict().cloned().unwrap_or_default(),
                None => Dict::new(),
            };
            if tdict.contains_key("XRefStm") {
                return Err("不支持：混合 xref（/XRefStm）".into());
            }
            if first {
                out.trailer = tdict.clone();
                first = false;
            }
            match tdict_get_num(&tdict, "Prev") {
                Some(prev) if prev > 0.0 => off = prev as u64,
                _ => break,
            }
        }
        Ok(out)
    }
}

fn tdict_get_num(d: &Dict, k: &str) -> Option<f64> {
    match d.get(k) {
        Some(Val::Num(n)) => Some(*n),
        _ => None,
    }
}

// ============================ 文档 ============================

struct Doc {
    src: Src,
    xref: Xref,
    catalog: Dict,
    /// 摊平后的页对象号（按 Kids 顺序；只在前 1 万页内展开，避免异常文件里无限递归）。
    page_refs: Vec<u32>,
    /// 页对象号 -> 页序号（目录目的地的反查，O(1)）。
    page_index: HashMap<u32, usize>,
    outline: Option<Vec<OutlineItem>>,
}

struct PageGeom {
    pt_w: f64,
    pt_h: f64,
    rotate: i64,
}

#[derive(Clone, serde::Serialize)]
pub struct OutlineItem {
    pub title: String,
    /// 0-based 页号；解析不出给 -1。
    pub page: i64,
}

impl Doc {
    fn open(path: &Path) -> Result<Doc, String> {
        let mut src = Src::open(path)?;
        let xref = Xref::parse(&mut src)?;
        let root = match xref.trailer.get("Root") {
            Some(Val::Ref(n, _)) => *n,
            _ => return Err("trailer 缺少 /Root".into()),
        };
        let mut doc = Doc {
            src,
            xref,
            catalog: Dict::new(),
            page_refs: Vec::new(),
            page_index: HashMap::new(),
            outline: None,
        };
        let cat = match doc.resolve_obj(root) {
            Some(v) => v.as_dict().cloned().unwrap_or_default(),
            None => return Err("无法解析 Catalog".into()),
        };
        doc.catalog = cat;
        doc.build_pages()?;
        Ok(doc)
    }

    fn resolve_obj(&mut self, obj: u32) -> Option<Val> {
        let off = *self.xref.map.get(&obj)?;
        let buf = self.src.read_at(off, 64 * 1024).ok()?;
        // 跳过 `N G obj` 头
        let mut p = P::new(&buf);
        p.ws();
        let _ = p.num();
        p.ws();
        let _ = p.num();
        p.ws();
        if p.starts(b"obj") {
            p.i += 3;
        }
        let v = p.value(0)?;
        // 流数据偏移是**相对该缓冲**的；这里换算成文件绝对偏移（否则读到的是别的对象）。
        Some(match v {
            Val::Stream(d, rel) => Val::Stream(d, off + rel),
            other => other,
        })
    }

    fn resolve(&mut self, v: &Val) -> Val {
        match v {
            Val::Ref(n, _) => self.resolve_obj(*n).unwrap_or(Val::Null),
            other => other.clone(),
        }
    }

    fn build_pages(&mut self) -> Result<(), String> {
        let pages_ref = match self.catalog.get("Pages") {
            Some(Val::Ref(n, _)) => *n,
            _ => return Err("Catalog 缺少 /Pages".into()),
        };
        let mut out: Vec<u32> = Vec::new();
        self.walk_pages(pages_ref, 0, &mut out)?;
        if out.is_empty() {
            return Err("没有可用的页".into());
        }
        self.page_index = out.iter().enumerate().map(|(i, r)| (*r, i)).collect();
        self.page_refs = out;
        Ok(())
    }

    /// 递归展开 Kids：扁平列表与 Pages 节点都覆盖。
    fn walk_pages(&mut self, obj: u32, depth: usize, out: &mut Vec<u32>) -> Result<(), String> {
        if depth > 64 || out.len() > 100_000 {
            return Err("页树过深/过大".into());
        }
        let node = match self.resolve_obj(obj) {
            Some(v) => v,
            None => return Ok(()),
        };
        let d = match node.as_dict() {
            Some(d) => d.clone(),
            None => return Ok(()),
        };
        let typ = d.get("Type").and_then(|v| v.as_name()).unwrap_or("");
        if typ == "Page" {
            out.push(obj);
            return Ok(());
        }
        if let Some(kids) = d.get("Kids").cloned() {
            let kids = self.resolve(&kids);
            if let Some(arr) = kids.as_arr() {
                let kids: Vec<Val> = arr.to_vec();
                for k in kids {
                    match k {
                        Val::Ref(n, _) => self.walk_pages(n, depth + 1, out)?,
                        other => {
                            if let Some(kd) = other.as_dict() {
                                if let Some(Val::Ref(n, _)) = dict_get(kd, "Kids") {
                                    self.walk_pages(*n, depth + 1, out)?;
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn page_count(&self) -> usize {
        self.page_refs.len()
    }

    /// 页字典（解析后的克隆）。
    fn page_dict(&mut self, page: usize) -> Option<Dict> {
        let obj = *self.page_refs.get(page)?;
        let v = self.resolve_obj(obj)?;
        v.as_dict().cloned()
    }

    /// 显示盒 + 旋转（缺 CropBox 回落 MediaBox；CropBox 与 MediaBox 取交集）。
    fn page_geom(&mut self, page: usize) -> Result<PageGeom, String> {
        let d = self.page_dict(page).ok_or("页字典无法解析")?;
        let media = self.box_of(&d, "MediaBox")?;
        let crop = match self.box_of(&d, "CropBox") {
            Ok(b) => b,
            Err(_) => media,
        };
        let (x0, y0) = (crop.0.max(media.0), crop.1.max(media.1));
        let (x1, y1) = (crop.2.min(media.2), crop.3.min(media.3));
        let w = (x1 - x0).abs();
        let h = (y1 - y0).abs();
        if w < 1.0 || h < 1.0 {
            return Err("显示盒为空".into());
        }
        let rot = match d.get("Rotate") {
            Some(Val::Num(n)) => ((*n as i64) % 360 + 360) % 360,
            Some(Val::Ref(..)) => {
                let r = self.resolve(d.get("Rotate").unwrap());
                match r.as_num() {
                    Some(n) => ((n as i64) % 360 + 360) % 360,
                    None => 0,
                }
            }
            _ => 0,
        };
        if !matches!(rot, 0 | 90 | 180 | 270) {
            return Err(format!("不支持的 /Rotate {rot}"));
        }
        // 显示盒的宽高按旋转修正（90/270 交换）
        let (pt_w, pt_h) = if rot == 90 || rot == 270 { (h, w) } else { (w, h) };
        Ok(PageGeom { pt_w, pt_h, rotate: rot })
    }

    fn box_of(&mut self, d: &Dict, key: &str) -> Result<(f64, f64, f64, f64), String> {
        let v = d.get(key).cloned().ok_or("缺少盒")?;
        let v = self.resolve(&v);
        let arr = v.as_arr().ok_or("盒不是数组")?;
        let mut nums: Vec<f64> = Vec::new();
        for it in arr {
            let it = self.resolve(it);
            if let Some(n) = it.as_num() {
                nums.push(n);
            }
        }
        if nums.len() < 4 {
            return Err("盒数组不完整".into());
        }
        Ok((nums[0], nums[1], nums[2], nums[3]))
    }

    /// 取该页「唯一的整页图像」XObject 的对象号（多图像/无图像/含字体 ⇒ 不支持）。
    fn page_image_obj(&mut self, page: usize) -> Result<u32, String> {
        let d = self.page_dict(page).ok_or("页字典无法解析")?;
        if d.contains_key("Annots") {
            // 有注解也能渲染；这里不拒绝，仅在探测时作为"非纯图"的信号之一（见 probe）。
        }
        let res = match d.get("Resources") {
            Some(v) => self.resolve(v),
            None => return Err("页缺少 /Resources".into()),
        };
        let rd = res.as_dict().ok_or("/Resources 不是字典")?;
        // `/Font` **不参与**判定（选项 A）：双层 OCR 扫描件（古籍/工具书常见）就是"整页图 + 可见文字"。
        // 有可见文字仍直出整页图（丢的是文字层，不是整页不可读），由内容流分析计数 `text_overlaid`。
        // B1：页面级遮罩/混合模式。带遮罩的页（MRC、前景+背景合成）若按单张整页图直出，
        // 会得到黑底黑字这类"看起来像渲染 bug"的图 —— 宁可占位也不画错。
        if let Some(g) = rd.get("ExtGState").cloned() {
            let g = self.resolve(&g);
            if let Some(gd) = g.as_dict() {
                let keys: Vec<String> = gd.keys().cloned().collect();
                for k in keys {
                    let st = self.resolve(gd.get(&k).unwrap_or(&Val::Null));
                    if let Some(sd) = st.as_dict() {
                        if let Some(bm) = sd.get("BM").cloned() {
                            let bm = self.resolve(&bm);
                            let bad = match &bm {
                                Val::Name(n) => n != "Normal" && n != "Compatible",
                                Val::Arr(a) => a.iter().any(|v| {
                                    v.as_name().map(|n| n != "Normal" && n != "Compatible").unwrap_or(false)
                                }),
                                _ => false,
                            };
                            if bad {
                                return Err(format!("页面使用非 Normal 混合模式（/ExtGState {k}）"));
                            }
                        }
                    }
                }
            }
        }
        let xo = match rd.get("XObject") {
            Some(v) => self.resolve(v),
            None => return Err("页面没有 XObject".into()),
        };
        let xd = xo.as_dict().ok_or("/XObject 不是字典")?;
        let mut images: Vec<u32> = Vec::new();
        let keys: Vec<String> = xd.keys().cloned().collect();
        for k in keys {
            if let Some(Val::Ref(n, _)) = xd.get(&k) {
                images.push(*n);
            }
        }
        if images.len() != 1 {
            return Err(format!("页面有 {} 个 XObject（期望 1 个整页图）", images.len()));
        }
        Ok(images[0])
    }

    /// 解析图像对象的流字典与数据起点。
    fn image_stream(&mut self, obj: u32) -> Result<(Dict, u64, usize), String> {
        let v = self.resolve_obj(obj).ok_or("图像对象无法解析")?;
        let (d, start) = match v {
            Val::Stream(d, s) => (*d, s),
            _ => return Err("图像对象不是流".into()),
        };
        let sub = d.get("Subtype").and_then(|x| x.as_name()).unwrap_or("");
        if sub != "Image" {
            return Err(format!("对象不是图像（/Subtype {sub}）"));
        }
        // B1：图像自身的遮罩。`/SMask`（软遮罩，常与 Flate 或 JPX 并存）、`/Mask`（显式或颜色键）、
        // `/ImageMask`（1bpc 模板）都意味着"这张图不是完整的可见页面"，直出会画错 ⇒ 该页占位。
        if d.contains_key("SMask") {
            return Err("图像带软遮罩（/SMask）".into());
        }
        if d.contains_key("Mask") {
            return Err("图像带遮罩（/Mask）".into());
        }
        if d.get("ImageMask").map(|v| !matches!(v, Val::Bool(false))).unwrap_or(false) {
            return Err("图像是 /ImageMask 模板".into());
        }
        // `/Length` 可能是间接引用
        let len_v = d.get("Length").cloned().ok_or("流缺少 /Length")?;
        let len = match &len_v {
            Val::Num(n) => *n as usize,
            Val::Ref(..) => self
                .resolve(&len_v)
                .as_num()
                .ok_or("/Length 引用无法解析")? as usize,
            _ => return Err("/Length 类型不支持".into()),
        };
        // 保守上界：单页图像不可能超过 64MB
        if len == 0 || len > 64 * 1024 * 1024 {
            return Err(format!("/Length 异常（{len} 字节）"));
        }
        Ok((d, start, len))
    }

    fn outline(&mut self) -> Vec<OutlineItem> {
        if let Some(v) = &self.outline {
            return v.clone();
        }
        let mut out: Vec<OutlineItem> = Vec::new();
        let root = match self.catalog.get("Outlines").cloned() {
            Some(v) => self.resolve(&v),
            None => {
                self.outline = Some(out.clone());
                return out;
            }
        };
        if let Some(rd) = root.as_dict() {
            if let Some(Val::Ref(n, _)) = rd.get("First") {
                let _ = self.walk_outline(*n, 0, &mut out);
            }
        }
        self.outline = Some(out.clone());
        out
    }

    fn walk_outline(&mut self, obj: u32, depth: usize, out: &mut Vec<OutlineItem>) -> Result<(), String> {
        if depth > 32 || out.len() > 20_000 {
            return Ok(());
        }
        let mut cur = obj;
        let mut guard = 0;
        loop {
            guard += 1;
            if guard > 50_000 {
                break;
            }
            let d = match self.resolve_obj(cur) {
                Some(v) => match v.as_dict() {
                    Some(d) => d.clone(),
                    None => break,
                },
                None => break,
            };
            let title = match d.get("Title") {
                Some(Val::Str(b)) => decode_pdf_text(b),
                _ => String::new(),
            };
            let page = self.outline_dest_page(&d);
            if !title.is_empty() {
                out.push(OutlineItem { title, page });
            }
            if let Some(Val::Ref(n, _)) = d.get("First") {
                let _ = self.walk_outline(*n, depth + 1, out);
            }
            match d.get("Next") {
                Some(Val::Ref(n, _)) => cur = *n,
                _ => break,
            }
        }
        Ok(())
    }

    /// 目的地 → 页号：`/Dest [ref /Fit...]` 或 `/A << /D [...] >>`。
    /// **`/Count` 是"可见子项数"（负值表示折叠），绝不能当页码。**
    fn outline_dest_page(&mut self, d: &Dict) -> i64 {
        let dest = match d.get("Dest") {
            Some(v) => Some(v.clone()),
            None => match d.get("A") {
                Some(a) => {
                    let a = self.resolve(a);
                    a.as_dict().and_then(|ad| ad.get("D").cloned())
                }
                None => None,
            },
        };
        let dest = match dest {
            Some(v) => self.resolve(&v),
            None => return -1,
        };
        if let Some(arr) = dest.as_arr() {
            if let Some(Val::Ref(n, _)) = arr.first() {
                return self.page_index.get(n).map(|i| *i as i64).unwrap_or(-1);
            }
        }
        -1
    }
}

/// PDF 文本串：UTF-16BE（带 BOM）或 PDFDocEncoding（近似按 Latin-1 处理）。
fn decode_pdf_text(b: &[u8]) -> String {
    let s = decode_pdf_text_raw(b);
    // 书签标题里常见尾部 NUL 填充：去掉，避免界面出现方块/空字符。
    s.trim_end_matches('\u{0}').to_string()
}

fn decode_pdf_text_raw(b: &[u8]) -> String {
    if b.len() >= 2 && b[0] == 0xFE && b[1] == 0xFF {
        let mut u: Vec<u16> = Vec::new();
        let mut i = 2;
        while i + 1 < b.len() {
            u.push(((b[i] as u16) << 8) | b[i + 1] as u16);
            i += 2;
        }
        return String::from_utf16_lossy(&u);
    }
    b.iter().map(|&c| c as char).collect()
}

// ============================ 内容流分析（选项 A）============================

/// 页面判定结果：**探针与取页共用同一份判定**（否则会出现"探针说合格、取页却占位"）。
#[derive(Clone, Debug, Default)]
pub struct PageVerdict {
    /// 页面唯一整页图的 XObject 对象号。
    pub image_obj: u32,
    /// 内容流里有一条 `cm` + `Do` 覆盖了显示盒（≥95% 边长）。
    pub covered: bool,
    /// 页面有**可见**文字（`Tj/TJ/'/"` 且不在 `Tr 3` 下）。
    pub text_visible: bool,
    /// 内容流解析失败 ⇒ 退回"单图近似"（仍可出图，只是没有覆盖判定）。
    pub fallback: bool,
    /// 说明（日志用）。
    pub reason: String,
}

impl Doc {
    /// 取该页要渲染的那张整页图：先做遮罩校验（`image_stream`），再返回对象号。
    /// **这是探针与两条取页路径唯一的入口**。
    fn page_verdict(&mut self, page: usize) -> Result<PageVerdict, String> {
        let obj = self.page_image_obj(page)?;
        // 遮罩/混合模式：`image_stream` 里统一判（带 /SMask /Mask /ImageMask 或非 Normal BM ⇒ Err）。
        let (dict, _s, _l) = self.image_stream(obj)?;
        let _ = &dict;
        let mut v = PageVerdict {
            image_obj: obj,
            ..Default::default()
        };
        match self.page_content(page) {
            Ok((ctm_cover, text_visible)) => {
                v.covered = ctm_cover;
                v.text_visible = text_visible;
                if !ctm_cover {
                    v.reason = "image_not_covering".into();
                }
            }
            Err(e) => {
                // 阶梯第 2 层：内容流解析失败/遇到不认识的算子 ⇒ 退回单图近似，**不判死**。
                v.fallback = true;
                v.reason = format!("fallback_single_image:{e}");
            }
        }
        Ok(v)
    }

    /// 流长度（`/Length` 可能是间接引用）。
    fn stream_len(&mut self, dd: &Dict) -> Result<usize, String> {
        let v = dd.get("Length").cloned().ok_or("流缺 /Length")?;
        match v {
            Val::Num(n) => Ok(n as usize),
            r @ Val::Ref(..) => self
                .resolve(&r)
                .as_num()
                .map(|n| n as usize)
                .ok_or_else(|| "/Length 引用无法解析".into()),
            _ => Err("/Length 类型不支持".into()),
        }
    }

    /// 把 `/Contents`（单流或数组）拼起来取字节。
    fn page_content_bytes(&mut self, page: usize) -> Result<Vec<u8>, String> {
        let d = self.page_dict(page).ok_or("页字典无法解析")?;
        let contents = d.get("Contents").cloned().ok_or("页缺少 /Contents")?;
        let contents = self.resolve(&contents);
        let mut parts: Vec<u32> = Vec::new();
        match contents {
            Val::Ref(n, _) => parts.push(n),
            Val::Arr(a) => {
                for it in a {
                    let it = self.resolve(&it);
                    match it {
                        Val::Ref(n, _) => parts.push(n),
                        Val::Stream(_, _) => return Err("内联内容流不支持".into()),
                        _ => return Err("/Contents 数组元素类型不支持".into()),
                    }
                }
            }
            Val::Stream(dd, start) => {
                // 单条内容流（最常见）：`/Length` 可能是间接引用。
                let len = self.stream_len(&dd)?;
                let filters = norm_filter(&dd);
                let raw = self.src.read_at(start, len)?;
                return decode_stream(&filters, &raw);
            }
            _ => return Err("/Contents 类型不支持".into()),
        }
        let mut out: Vec<u8> = Vec::new();
        for obj in parts {
            let v = self.resolve_obj(obj).ok_or("内容流对象无法解析")?;
            let (dd, start) = match v {
                Val::Stream(dd, s) => (*dd, s),
                _ => return Err("内容流不是流对象".into()),
            };
            let len = self.stream_len(&dd)?;
            if len > 64 * 1024 * 1024 {
                return Err(format!("内容流过大（{len} 字节）"));
            }
            let filters = norm_filter(&dd);
            let raw = self.src.read_at(start, len)?;
            out.extend_from_slice(&decode_stream(&filters, &raw)?);
            out.push(b'\n');
        }
        Ok(out)
    }

    /// 解析内容流，返回 (是否有覆盖显示盒的图像绘制, 是否有可见文字)。
    fn page_content(&mut self, page: usize) -> Result<(bool, bool), String> {
        let bytes = self.page_content_bytes(page)?;
        let geom = self.page_geom(page)?;
        // 显示盒在"页面坐标"里是 [0..pt_w_rot]×[0..pt_h_rot]；这里只用宽高做覆盖判定。
        analyze_content(&bytes, geom.pt_w, geom.pt_h)
    }
}

/// 流解码（只支持 FlateDecode；**这里才是 flate2 的真实用途**）。
fn decode_stream(filters: &[String], raw: &[u8]) -> Result<Vec<u8>, String> {
    if filters.is_empty() {
        return Ok(raw.to_vec());
    }
    if filters.len() == 1 && (filters[0] == "FlateDecode" || filters[0] == "Fl") {
        use std::io::Read as _;
        let mut z = flate2::read::ZlibDecoder::new(raw);
        let mut out = Vec::new();
        z.read_to_end(&mut out).map_err(|e| format!("FlateDecode 失败：{e}"))?;
        return Ok(out);
    }
    Err(format!("内容流编码不支持 {filters:?}"))
}

/// 内容流分析：跟踪 `cm`/`Do` 的覆盖与可见文字。
///
/// 阶梯：只认最必要的一组算子（q Q cm Do BT ET Tf Td Tm Tj TJ ' " Tr BI ID EI）；
/// 遇到不认识的算子**不报错**（PDF 内容流里画线/颜色算子极多），只在无法继续分词的
/// 结构性问题上返回 Err —— 调用方据此退回"单图近似"。
fn analyze_content(bytes: &[u8], pt_w: f64, pt_h: f64) -> Result<(bool, bool), String> {
    let mut i = 0usize;
    let n = bytes.len();
    let mut stack: Vec<[f64; 6]> = vec![[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]];
    let mut nums: Vec<f64> = Vec::new();
    let mut last_name: Option<String> = None;
    let mut covered = false;
    let mut text_visible = false;
    let mut tr = 0i64;
    let mut depth = 0usize;
    let is_ws = |c: u8| c == b' ' || c == b'\t' || c == b'\r' || c == b'\n' || c == 0x0c || c == 0;
    while i < n {
        let c = bytes[i];
        if is_ws(c) {
            i += 1;
            continue;
        }
        if c == b'%' {
            while i < n && bytes[i] != b'\n' && bytes[i] != b'\r' {
                i += 1;
            }
            continue;
        }
        if c == b'(' {
            // 字符串：跳过（含转义与括注嵌套）
            let mut d = 1;
            i += 1;
            while i < n && d > 0 {
                match bytes[i] {
                    b'\\' => i += 1,
                    b'(' => d += 1,
                    b')' => d -= 1,
                    _ => {}
                }
                i += 1;
            }
            continue;
        }
        if c == b'<' {
            if bytes.get(i + 1) == Some(&b'<') {
                // 内联字典（可能是 BDC 的属性或 BI 的参数）：跳到 >>
                i += 2;
                while i + 1 < n && !(bytes[i] == b'>' && bytes[i + 1] == b'>') {
                    i += 1;
                }
                i = (i + 2).min(n);
            } else {
                while i < n && bytes[i] != b'>' {
                    i += 1;
                }
                i = (i + 1).min(n);
            }
            continue;
        }
        if c == b'[' || c == b']' {
            i += 1;
            continue;
        }
        if c == b'/' {
            let st = i + 1;
            i += 1;
            while i < n && !is_ws(bytes[i]) && !b"[]<>/()%".contains(&bytes[i]) {
                i += 1;
            }
            last_name = Some(String::from_utf8_lossy(&bytes[st..i]).into_owned());
            continue;
        }
        if c.is_ascii_digit() || c == b'-' || c == b'+' || c == b'.' {
            let st = i;
            i += 1;
            while i < n
                && (bytes[i].is_ascii_digit()
                    || bytes[i] == b'.'
                    || bytes[i] == b'-'
                    || bytes[i] == b'+')
            {
                i += 1;
            }
            if let Ok(s) = std::str::from_utf8(&bytes[st..i]) {
                if let Ok(v) = s.parse::<f64>() {
                    nums.push(v);
                    continue;
                }
            }
            nums.clear();
            continue;
        }
        // 算子
        let st = i;
        i += 1;
        while i < n && !is_ws(bytes[i]) && !b"[]<>/()%".contains(&bytes[i]) {
            i += 1;
        }
        let op = &bytes[st..i];
        match op {
            b"q" => {
                let cur = *stack.last().unwrap_or(&[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
                stack.push(cur);
                depth += 1;
                if depth > 256 {
                    return Err("q 嵌套过深".into());
                }
                nums.clear();
                last_name = None;
            }
            b"Q" => {
                if stack.len() > 1 {
                    stack.pop();
                    depth = depth.saturating_sub(1);
                }
                nums.clear();
                last_name = None;
            }
            b"cm" => {
                if nums.len() >= 6 {
                    let m = &nums[nums.len() - 6..];
                    let cm = [m[0], m[1], m[2], m[3], m[4], m[5]];
                    if let Some(cur) = stack.last_mut() {
                        *cur = mat_mul(cm, *cur);
                    }
                }
                nums.clear();
                last_name = None;
            }
            b"Do" => {
                // 只关心"是不是那张整页图"这件事：用当前 CTM 的单位方框换算到页面坐标。
                if last_name.is_some() {
                    if let Some(cur) = stack.last() {
                        let (w, h) = unit_square_size(*cur);
                        if w >= pt_w * 0.95 && h >= pt_h * 0.95 {
                            covered = true;
                        }
                    }
                }
                nums.clear();
                last_name = None;
            }
            b"Tr" => {
                if let Some(v) = nums.last() {
                    tr = *v as i64;
                }
                nums.clear();
                last_name = None;
            }
            b"BT" => {
                tr = 0;
                nums.clear();
                last_name = None;
            }
            b"Tj" | b"TJ" | b"'" | b"\"" => {
                if tr != 3 {
                    text_visible = true;
                }
                nums.clear();
                last_name = None;
            }
            b"BI" => {
                // 内联图像：跳到 `ID` 之后的数据，再找 `EI`。解析不了就判 fallback。
                let mut j = i;
                let mut found_id = false;
                while j + 1 < n {
                    if bytes[j] == b'I' && bytes[j + 1] == b'D' {
                        found_id = true;
                        j += 2;
                        break;
                    }
                    j += 1;
                }
                if !found_id {
                    return Err("内联图像缺 ID".into());
                }
                let mut k = j;
                let mut found_ei = false;
                while k + 1 < n {
                    if bytes[k] == b'E'
                        && bytes[k + 1] == b'I'
                        && (k == 0 || is_ws(bytes[k - 1]))
                        && (k + 2 >= n || is_ws(bytes[k + 2]))
                    {
                        found_ei = true;
                        k += 2;
                        break;
                    }
                    k += 1;
                }
                if !found_ei {
                    return Err("内联图像缺 EI".into());
                }
                i = k;
                nums.clear();
                last_name = None;
            }
            _ => {
                // 其它算子一律忽略（颜色/线宽/路径/标注…），清空操作数栈。
                nums.clear();
                last_name = None;
            }
        }
    }
    Ok((covered, text_visible))
}

/// 矩阵乘法：`a × b`（PDF 的 cm 约定，先 a 后 b）。
fn mat_mul(a: [f64; 6], b: [f64; 6]) -> [f64; 6] {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

/// 单位方框经 CTM 变换后的宽高（绝对值）。
fn unit_square_size(m: [f64; 6]) -> (f64, f64) {
    (m[0].abs().max(m[2].abs()), m[1].abs().max(m[3].abs()))
}

// ============================ 页数据输出 ============================

#[derive(Clone, serde::Serialize)]
pub struct ScanOpen {
    pub pages: usize,
    pub image_only: bool,
    /// 探测到的问题（用于前端提示；image_only=true 时为空）
    pub reason: String,
}

#[derive(Clone, serde::Serialize)]
pub struct ScanPage {
    /// Indexed 颜色空间的调色板（RGB 三元组；非 Indexed 为空）。
    /// 本册尾部约 900 页是 `/ColorSpace [/Indexed …]` + 4bpc，前端靠它把索引映射成 RGB。
    pub palette: Vec<u8>,
    /// Indexed 的最大索引值（hival）。
    pub hival: u32,
    /// jpeg / raw / unsupported
    pub kind: String,
    pub pt_w: f64,
    pub pt_h: f64,
    pub rotate: i64,
    /// 图像像素（`/Width` `/Height`；rot90/270 时按显示方向交换）
    pub width: u32,
    pub height: u32,
    pub bpc: u32,
    pub color_space: String,
    pub predictor: i64,
    pub bytes_len: usize,
    /// unsupported 时的原因码
    pub reason: String,
}

fn norm_filter(d: &Dict) -> Vec<String> {
    match d.get("Filter") {
        Some(Val::Name(n)) => vec![n.clone()],
        Some(Val::Arr(a)) => a.iter().filter_map(|v| v.as_name().map(|s| s.to_string())).collect(),
        _ => vec![],
    }
}

fn norm_decodeparms(d: &Dict) -> Vec<Dict> {
    match d.get("DecodeParms") {
        Some(Val::Dict(dd)) => vec![dd.clone()],
        Some(Val::Arr(a)) => a
            .iter()
            .map(|v| v.as_dict().cloned().unwrap_or_default())
            .collect(),
        _ => vec![],
    }
}

fn val_kind(v: &Val) -> &'static str {
    match v {
        Val::Null => "null",
        Val::Bool(_) => "bool",
        Val::Num(_) => "num",
        Val::Str(b) => if b.is_empty() { "str(empty)" } else { "str" },
        Val::Name(_) => "name",
        Val::Arr(_) => "array",
        Val::Dict(_) => "dict",
        Val::Ref(..) => "ref",
        Val::Stream(..) => "stream",
    }
}

fn color_space_name(v: Option<&Val>) -> String {
    match v {
        Some(Val::Name(n)) => n.clone(),
        Some(Val::Arr(a)) => match a.first() {
            Some(Val::Name(n)) => n.clone(),
            _ => "unknown".into(),
        },
        _ => "unknown".into(),
    }
}

/// JPEG 基本检查：`FFD8` 开头、能找到 SOF、分量数 1 或 3（4 分量 CMYK/YCCK 直接占位）。
fn jpeg_ok(b: &[u8]) -> Result<(u32, u32), String> {
    if b.len() < 4 || b[0] != 0xFF || b[1] != 0xD8 {
        return Err("不是 JPEG（缺 FFD8）".into());
    }
    let mut i = 2usize;
    while i + 3 < b.len() {
        if b[i] != 0xFF {
            i += 1;
            continue;
        }
        let m = b[i + 1];
        if m == 0xD8 || m == 0x01 || (0xD0..=0xD7).contains(&m) {
            i += 2;
            continue;
        }
        if m == 0xD9 || m == 0xDA {
            break;
        }
        let seg = ((b[i + 2] as usize) << 8) | b[i + 3] as usize;
        if seg < 2 || i + 2 + seg > b.len() {
            break;
        }
        // SOF0..SOF15（排除 DHT=C4, JPG=C8, DAC=CC）
        if (0xC0..=0xCF).contains(&m) && m != 0xC4 && m != 0xC8 && m != 0xCC {
            let comps = b[i + 9] as u32;
            let h = ((b[i + 5] as u32) << 8) | b[i + 6] as u32;
            let w = ((b[i + 7] as u32) << 8) | b[i + 8] as u32;
            if comps != 1 && comps != 3 {
                return Err(format!("JPEG 分量数 {comps}（仅支持灰度/RGB）"));
            }
            return Ok((w, h));
        }
        i += 2 + seg;
    }
    Err("JPEG 缺少 SOF".into())
}

// ============================ 缓存 ============================

struct Cached {
    doc: Doc,
    open: ScanOpen,
}

/// 探测期的三个计数（进程内累计，日志与报告用）：
/// `TEXT_OVERLAID` = 采样页里有可见文字（仍直出），`FALLBACK_SINGLE` = 内容流不可解析退回单图近似。
static TEXT_OVERLAID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static FALLBACK_SINGLE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn cache() -> &'static Mutex<HashMap<i64, Arc<Mutex<Cached>>>> {
    static C: OnceLock<Mutex<HashMap<i64, Arc<Mutex<Cached>>>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

fn get_book_path(state: &AppState, book_id: i64) -> Result<String, String> {
    let conn = state.db.0.lock().unwrap();
    crate::db::get_book(&conn, book_id)
        .map(|b| b.path)
        .ok_or_else(|| "书籍不存在".to_string())
}

/// 探测：首页 + 均匀 8 个采样点（≤9 页）判定这本能不能走直读通道。
///
/// **分级策略（B2 方案 1）**：`>1.9 GiB` 的书一旦整本回落原生，原生可能永久不返回
/// （3.50 GiB 那册就是），等于把用户关在门外。所以这里区分两类失败：
/// - **结构不支持**（开文件/xref/页树解析失败）⇒ 整本不可用，回落原生（这是唯一安全的判断）；
/// - **个别页不支持**（多图/含字体/带遮罩/编码不支持/JPEG 无效）⇒ **仍进直读**，
///   这些页由前端显示占位（`kind="unsupported"` 带原因），不因为几页而放弃整本；
///   只有当**采样页一个都不支持**（或支持率为 0）时才回落原生。
///
/// 返回 `(usable, image_only, unsupported_sampled, reason)`：
/// `usable`=通道可用；`image_only`=全部采样页都合格（用于日志与旧口径兼容）。
fn probe(doc: &mut Doc) -> (bool, bool, usize, String) {
    let n = doc.page_count();
    let mut samples: Vec<usize> = vec![0];
    for k in 1..=8 {
        let p = (n as f64 - 1.0) * (k as f64 / 9.0);
        let p = p.round() as usize;
        if !samples.contains(&p) && p < n {
            samples.push(p);
        }
    }
    let mut bad: Vec<String> = Vec::new();
    let mut good = 0usize;
    for &p in samples.iter() {
        let verdict: Result<(), String> = (|| {
            // 与取页路径**同一个函数**：探针说合格 ⇒ 取页必定能出图。
            let v = doc.page_verdict(p)?;
            let (d, s, l) = doc.image_stream(v.image_obj)?;
            let filters = norm_filter(&d);
            if !(filters.len() == 1 && matches!(filters[0].as_str(), "DCTDecode" | "FlateDecode")) {
                return Err(format!("编码不支持 /Filter {filters:?}"));
            }
            if filters[0] == "DCTDecode" {
                let bytes = read_stream_bytes(doc, &d, s, l)?;
                jpeg_ok(&bytes)?;
            }
            // 三档判定（/Font 不参与）：可见文字不阻止直出，只计数。
            if v.text_visible {
                TEXT_OVERLAID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            if v.fallback {
                FALLBACK_SINGLE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                crate::memlog::stage_note(
                    "pdf_scan:fallback",
                    &format!("page={} reason={}", p + 1, v.reason),
                );
            }
            Ok(())
        })();
        match verdict {
            Ok(()) => good += 1,
            Err(e) => bad.push(format!("p{}:{e}", p + 1)),
        }
    }
    let unsupported = bad.len();
    if good == 0 {
        return (
            false,
            false,
            unsupported,
            format!("采样页均不可直读（{}）", bad.join("; ")),
        );
    }
    let image_only = unsupported == 0;
    let reason = if image_only {
        String::new()
    } else {
        format!("{unsupported}/{} 个采样页不支持，将显示占位（{}）", samples.len(), bad.join("; "))
    };
    (true, image_only, unsupported, reason)
}

fn read_stream_bytes(doc: &mut Doc, _d: &Dict, start: u64, len: usize) -> Result<Vec<u8>, String> {
    doc.src.read_at(start, len)
}

// ============================ 命令 ============================

#[tauri::command]
pub async fn pdf_scan_open(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
) -> Result<ScanOpen, String> {
    let path = get_book_path(state.inner(), book_id)?;
    let key = book_id;
    if let Some(entry) = cache().lock().unwrap().get(&key).cloned() {
        let g = entry.lock().unwrap();
        return Ok(g.open.clone());
    }
    let t0 = std::time::Instant::now();
    let mut doc = Doc::open(Path::new(&path))?;
    let (usable, image_only, unsupported_sampled, reason) = probe(&mut doc);
    let open = ScanOpen {
        pages: doc.page_count(),
        // 前端只看 `image_only`：它表示"这条通道可用"（个别页占位不算不可用，见 probe 的注释）。
        image_only: usable,
        reason,
    };
    crate::memlog::stage_note(
        "pdf_scan:probe",
        &format!(
            "book={book_id} usable={usable} all_sampled_ok={image_only} unsupported_sampled={unsupported_sampled} text_overlaid={} fallback_single_image={}",
            TEXT_OVERLAID.load(std::sync::atomic::Ordering::Relaxed),
            FALLBACK_SINGLE.load(std::sync::atomic::Ordering::Relaxed),
        ),
    );
    if !usable {
        crate::memlog::stage_note(
            "pdf_scan:rejected",
            &format!("book={book_id} reason={}", open.reason),
        );
    }
    crate::memlog::stage_note(
        "pdf_scan:open",
        &format!(
            "book={book_id} pages={} image_only={} elapsed_ms={}",
            open.pages,
            open.image_only,
            t0.elapsed().as_millis()
        ),
    );
    cache().lock().unwrap().insert(key, Arc::new(Mutex::new(Cached { doc, open: open.clone() })));
    Ok(open)
}

fn with_doc<T>(book_id: i64, f: impl FnOnce(&mut Doc) -> Result<T, String>) -> Result<T, String> {
    let entry = cache()
        .lock()
        .unwrap()
        .get(&book_id)
        .cloned()
        .ok_or("尚未打开该 PDF 的直读通道")?;
    let mut g = entry.lock().unwrap();
    f(&mut g.doc)
}

#[tauri::command]
pub async fn pdf_scan_page(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
    page: i64,
) -> Result<ScanPage, String> {
    if page < 0 {
        return Err("页号越界".into());
    }
    let _ = state;
    with_doc(book_id, |doc| {
        let p = page as usize;
        let geom = match doc.page_geom(p) {
            Ok(g) => g,
            Err(e) => {
                return Ok(ScanPage {
                    palette: Vec::new(),
                    hival: 0,
                    kind: "unsupported".into(),
                    pt_w: 1.0,
                    pt_h: 1.0,
                    rotate: 0,
                    width: 0,
                    height: 0,
                    bpc: 8,
                    color_space: "unknown".into(),
                    predictor: 1,
                    bytes_len: 0,
                    reason: format!("GEOM:{e}"),
                })
            }
        };
        let mut base = ScanPage {
            palette: Vec::new(),
            hival: 0,
            kind: "unsupported".into(),
            pt_w: geom.pt_w,
            pt_h: geom.pt_h,
            rotate: geom.rotate,
            width: 0,
            height: 0,
            bpc: 8,
            color_space: "unknown".into(),
            predictor: 1,
            bytes_len: 0,
            reason: String::new(),
        };
        // 与探针共用同一份判定（page_verdict）：探针合格 ⇒ 这里必定出图，不会分裂。
        let obj = match doc.page_verdict(p).map(|v| v.image_obj) {
            Ok(o) => o,
            Err(e) => {
                base.reason = format!("NOT_IMAGE:{e}");
                return Ok(base);
            }
        };
        let (d, start, len) = match doc.image_stream(obj) {
            Ok(v) => v,
            Err(e) => {
                base.reason = format!("NO_STREAM:{e}");
                return Ok(base);
            }
        };
        let w = d.get("Width").and_then(|v| v.as_num()).unwrap_or(0.0).max(0.0) as u32;
        let h = d.get("Height").and_then(|v| v.as_num()).unwrap_or(0.0).max(0.0) as u32;
        base.width = w;
        base.height = h;
        base.bpc = d.get("BitsPerComponent").and_then(|v| v.as_num()).unwrap_or(8.0) as u32;
        base.color_space = color_space_name(d.get("ColorSpace"));
        // Indexed：[ /Indexed base hival lookup ] —— lookup 可能是字符串（调色板字节）或间接引用。
        if base.color_space == "Indexed" {
            if let Some(cs) = d.get("ColorSpace").cloned() {
                let cs = doc.resolve(&cs);
                if let Some(arr) = cs.as_arr() {
                    if let Some(hv) = arr.get(2).and_then(|v| v.as_num()) {
                        base.hival = hv.max(0.0) as u32;
                    }
                    if let Some(lookup) = arr.get(3).cloned() {
                        let raw_kind = val_kind(&lookup);
                        let lookup = doc.resolve(&lookup);
                        match lookup {
                            Val::Str(bytes) => base.palette = bytes,
                            // 有的生成器把 lookup 写成**流对象**（`16 0 R` 是 stream）：无滤镜时直接取字节。
                            Val::Stream(sd, start) => {
                                let lens = match sd.get("Length").cloned() {
                                    Some(Val::Num(n)) => Some(n as usize),
                                    Some(r @ Val::Ref(..)) => doc.resolve(&r).as_num().map(|n| n as usize),
                                    _ => None,
                                };
                                let filters = norm_filter(&sd);
                                match (lens, filters.is_empty()) {
                                    (Some(n), true) if n > 0 && n <= 4096 => {
                                        match doc.src.read_at(start, n) {
                                            Ok(bytes) => base.palette = bytes,
                                            Err(e) => base.reason = format!("INDEXED_STREAM_READ:{e}"),
                                        }
                                    }
                                    (Some(_n), false) => {
                                        base.reason = format!("INDEXED_LOOKUP_FILTERED {:?}", filters)
                                    }
                                    _ => base.reason = format!("INDEXED_LOOKUP_BAD_LEN {raw_kind}"),
                                }
                            }
                            other => base.reason = format!("INDEXED_LOOKUP {raw_kind} -> {}", val_kind(&other)),
                        }
                    } else {
                        base.reason = format!("INDEXED_NO_LOOKUP n={}", arr.len());
                    }
                }
            }
            if base.palette.is_empty() || !matches!(base.bpc, 1 | 2 | 4 | 8) {
                if base.reason.is_empty() {
                    base.reason = format!("INDEXED_PALETTE_MISSING bpc={}", base.bpc);
                }
                return Ok(base);
            }
        } else if !(matches!(
            base.color_space.as_str(),
            "DeviceGray" | "DeviceRGB" | "CalRGB"
        ) && base.bpc == 8)
        {
            // 非 Indexed 只支持灰度/RGB 的 8bpc：其它组合先判掉，避免解码出花屏。
            base.reason = format!("COLOR_UNSUPPORTED {} bpc={}", base.color_space, base.bpc);
            return Ok(base);
        }
        let filters = norm_filter(&d);
        let parms = norm_decodeparms(&d);
        base.predictor = parms
            .first()
            .and_then(|p| p.get("Predictor"))
            .and_then(|v| v.as_num())
            .map(|n| n as i64)
            .unwrap_or(1);
        if filters.len() != 1 {
            base.reason = format!("FILTER_CHAIN {:?}", filters);
            return Ok(base);
        }
        match filters[0].as_str() {
            "DCTDecode" => {
                let bytes = read_stream_bytes(doc, &d, start, len)?;
                if let Err(e) = jpeg_ok(&bytes) {
                    base.reason = format!("JPEG_INVALID:{e}");
                    return Ok(base);
                }
                base.kind = "jpeg".into();
                base.bytes_len = bytes.len();
                Ok(base)
            }
            "FlateDecode" => {
                // 图像流只回未解压字节 + 参数；解压交给前端 DecompressionStream（内容流另在 decode_stream 用 flate2 解）
                base.kind = "raw".into();
                base.bytes_len = len;
                Ok(base)
            }
            other => {
                base.reason = format!("FILTER_UNSUPPORTED:{other}");
                Ok(base)
            }
        }
    })
}

/// 页图像原始字节（jpeg 原始 JPEG / raw 未解压 zlib）。走 ipc 原始字节通道，避免 base64 往返。
#[tauri::command]
pub async fn pdf_scan_page_bytes(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
    page: i64,
) -> Result<tauri::ipc::Response, String> {
    if page < 0 {
        return Err("页号越界".into());
    }
    let _ = state;
    let bytes = with_doc(book_id, |doc| {
        let p = page as usize;
        let obj = doc.page_verdict(p)?.image_obj;
        let (d, start, len) = doc.image_stream(obj)?;
        read_stream_bytes(doc, &d, start, len)
    })?;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
pub async fn pdf_scan_outline(
    state: State<'_, Arc<AppState>>,
    book_id: i64,
) -> Result<Vec<OutlineItem>, String> {
    let _ = state;
    with_doc(book_id, |doc| {
        let items = doc.outline();
        crate::memlog::stage_note(
            "pdf_scan:outline",
            &format!("book={book_id} items={}", items.len()),
        );
        Ok(items)
    })
}

#[tauri::command]
pub async fn pdf_scan_close(state: State<'_, Arc<AppState>>, book_id: i64) -> Result<(), String> {
    let _ = state;
    cache().lock().unwrap().remove(&book_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_objects() {
        let v = parse_val(b"<< /Type /Page /MediaBox [0 0 612 792] /Resources << /XObject << /Im1 5 0 R >> >> >>").unwrap();
        let d = v.as_dict().unwrap();
        assert_eq!(d.get("Type").unwrap().as_name(), Some("Page"));
        let mb = d.get("MediaBox").unwrap().as_arr().unwrap();
        assert_eq!(mb.len(), 4);
    }

    #[test]
    fn indirect_length_and_stream_start() {
        let v = parse_val(b"<< /Length 2 0 R /Filter /DCTDecode >>\nstream\r\nABC").unwrap();
        match v {
            Val::Stream(d, start) => {
                assert!(d.contains_key("Length"));
                // `parse_val` 返回**缓冲内**偏移；`Doc::resolve_obj` 才换算成文件绝对偏移。
                assert!(start > 0 && start < 64, "流起点应在缓冲内（实际 {start}）");
            }
            _ => panic!("应当解析为流"),
        }
    }

    #[test]
    fn rejects_non_jpeg_and_cmyk() {
        assert!(jpeg_ok(&[0u8; 16]).is_err());
        // 最小 JPEG 头 + SOF0(3 分量)
        let mut b = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x06, 0x7A, 0x04, 0x7E, 0x03];
        b.extend_from_slice(&[0u8; 32]);
        let (w, h) = jpeg_ok(&b).unwrap();
        assert_eq!((w, h), (1150, 1658));
        // 4 分量（CMYK/YCCK）⇒ 拒绝：分量数在 SOF 段内偏移 +9 处（本例为下标 11）
        let mut c = b.clone();
        c[11] = 4;
        assert!(jpeg_ok(&c).is_err());
    }

    #[test]
    fn utf16_outline_title() {
        let mut b = vec![0xFE, 0xFF];
        for ch in "第一页".encode_utf16() {
            b.push((ch >> 8) as u8);
            b.push((ch & 0xFF) as u8);
        }
        assert_eq!(decode_pdf_text(&b), "第一页");
    }
}
