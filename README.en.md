<div align="center">

<img src="docs/images/readme-hero2-en.jpg" width="100%" alt="TidePage — Books stay local. Reading, unhurried.">

<br>

**A desktop e-book reader that keeps your books on your machine**

TXT · EPUB · MOBI · AZW3 · PRC · PDF　｜　Typography · Comic mode · AI · Text-to-speech

[![Version](https://img.shields.io/badge/version-0.1.31-C79A3B?style=flat-square&labelColor=0A111E)](https://github.com/miipa/TidePage/releases/latest)
[![Platform](https://img.shields.io/badge/platform-Windows%2010%20%2F%2011-4E7FB5?style=flat-square&labelColor=0A111E)](#system-requirements)
[![License](https://img.shields.io/badge/license-MIT-8A94A6?style=flat-square&labelColor=0A111E)](LICENSE)

[Download](#download-and-install) · [Features](#features) · [Screenshots](#screenshots) · [简体中文](README.md)

</div>

---

## Before anything else

TidePage is not an online bookstore and it does not keep a cloud library. It does one job: **organize the e-books you already own, render them comfortably, and keep your reading history on your own machine.**

There is no sign-up and no upload. Only four things reach the network: AI chat, online text-to-speech, WebDAV sync, and the update check that runs automatically after launch.

- **Local first** — books, covers, progress, highlights, notes and stats live in a local SQLite database; credentials are encrypted with Windows DPAPI.
- **Built for volume** — virtual scrolling and rank-based paging are designed for libraries of 100,000+ books; search and filtering run locally.
- **Made for long reading** — Chinese serif typography, a separate Latin font, five reading themes, comic mode, an AI assistant and text-to-speech.

## Features

### Library

- Register several local folders at once. Existing files are never moved or renamed (the only exception is an explicit title / author write-back to EPUB).
- Virtual scrolling and rank-based paging for large libraries; grid and list views, with instant search and filters.
- Sort by last read, date added, title, file time or size, in either direction.
- Multi-select and batch actions, open a random book, drag and drop to import.
- Open each book in its own window; windows can snap together and move as a group.
- A live status line shows progress while folders are scanned or removed.
- Book details: edit title and author (written back to EPUB), suggest metadata from the file name, reveal in Explorer, reading history.

### Reading

- Three presentations: vertical scroll, single-page and two-column paging. Margins are remembered per mode.
- **Five reading themes** (Paper / White / Eye-care green / Night / Black), plus custom background color, text color, background image and opacity.
- Font size, line height, paragraph spacing, weight, alignment and first-line indent are all adjustable; install TTF / OTF / TTC fonts.
- Table of contents, bookmarks, in-book search, progress rings; reading stats for the last 14 days and per book (characters, time, average speed, sessions).
- Different chapter-transition effects for scrolling and paging.
- Per-book overrides for font, image scale, image display and CSS; rebindable shortcuts and click zones.

### Chinese and English typography

- Chinese text can use a serif face with first-line indents; English can use its own Latin font without changing the Chinese glyphs.
- Font size, line height and line breaks keep one reading rhythm when the two scripts are mixed.
- Custom fonts can be applied per book; typography and margins stay independent.

### Comic mode

- Detects comic-like books from image count and sparse body text, or switch manually from the toolbar.
- Vertical continuous (webtoon-style) and horizontal continuous (drag to pan) modes; horizontal supports left-to-right and right-to-left.
- Adjustable image scale, fit width / height, margins and gap (0 = seamless); built-in image viewer with wheel paging and click-to-zoom.
- Hide all images, block individual pages and restore them later.

### AI assistant

- Chat with the current book or chapter, with the chapter injected as context and streaming output.
- Select text to translate or explain; results can be added to a highlight note.
- Custom prompts can be bound to shortcuts, reordered by dragging, and the first prompt can run automatically on each new chapter.
- Supports OpenAI-compatible, Ollama (local), Anthropic, Google Gemini and Azure OpenAI; multi-endpoint failover and connection tests.
- Proxy modes: direct / system / manual, with SOCKS5; applies to AI and online TTS.
- Export conversations as TXT or EPUB.

### Text-to-speech

- System speech and online speech engines.
- Online protocols include OpenAI-compatible, VoiceStudio, Azure, DashScope, MiniMax, Fish Audio, GPT-SoVITS, Breeze, Volcano (Doubao), Baidu and Edge TTS.
- Rate, pitch and volume controls; sleep timer, sentence highlighting with scroll-follow, and automatic continuation to the next chapter.

### Highlights, notes and conversion

- Seven highlight styles (underline, wavy, dotted, double underline, highlighter, strikethrough, box) and eight preset colors.
- Notes are grouped per book, searchable and jumpable; highlights survive book deletion.
- Convert TXT → EPUB, EPUB → TXT, and MOBI / AZW3 / PRC → TXT or EPUB. Conversion writes a new file and never changes the original.
- AI conversations can also be exported as TXT or EPUB.

### Sync and backup

- WebDAV sync for reading progress, book upload / download, remote folder browsing and settings backup / restore.
- Copy or import settings through the clipboard; exported settings have secrets stripped.
- Local credentials (AI / TTS / WebDAV keys and proxy URLs with accounts) are encrypted with Windows DPAPI.
- Cloud settings backups use a key derived from the WebDAV account (PBKDF2-HMAC-SHA256 + AES-256-GCM).

### Privacy and network

- Book files and reading data live in the system app-data directory by default; override it with the `TIDEPAGE_DATA_DIR` environment variable. The database file is `tidepage.db`.
- No account, no telemetry; only AI, online TTS, WebDAV and the update check use the network.
- The update check runs three seconds after launch and reads only public GitHub Releases information, without credentials.
- When Volcano or Baidu TTS is selected, custom base URLs are ignored and the official endpoint is used.

### More

- SQLite FTS5 full-text index for whole-library search, with one-click rebuild.
- Storage usage and cache cleanup; window-state persistence and immersive mode.
- UI languages: Auto / Simplified Chinese / English, switchable instantly.
- Built-in update check at launch and a manual check in Settings.

## Supported formats

| Type | Support |
| --- | --- |
| E-books | TXT, EPUB, MOBI, AZW3, PRC, PDF |
| Text encoding | Auto-detected (UTF-8 / UTF-16 / GB18030 / Big5, …), with manual override |
| Covers / comic images | PNG, JPG / JPEG, GIF, BMP, WebP; EPUB / MOBI content images also support SVG |


## Screenshots

<p align="center">
  <img src="docs/images/readme-reading-zh.jpg" width="48%" alt="Chinese reading view">
  <img src="docs/images/readme-reading-en.jpg" width="48%" alt="English reading view">
</p>

<p align="center">
  <img src="docs/images/readme-appearance.jpg" width="72%" alt="Appearance settings">
</p>

The two books shown are Lu Xun's *Nahan · Guxiang* and Thoreau's *Walden · Economy*; the interface language is Simplified Chinese.

## Download and install

| Architecture | File | Notes |
| --- | --- | --- |
| 64-bit | `TidePage-0.1.31-windows-x64-portable.zip` | Most Windows 10 / 11 devices |
| 32-bit | `TidePage-0.1.31-windows-x86-portable.zip` | 32-bit systems or x86 compatibility environments |

1. Download the matching archive from [Releases](https://github.com/miipa/TidePage/releases/latest).
2. Extract it and run `TidePage.exe`; no installation is required.
3. The first launch needs the **WebView2 runtime** (built into Windows 11; usually a separate install on Windows 10).
4. To upgrade, extract the new version over the old program files. Library and reading data are not stored in the program folder.

> Only Windows builds are published: credential encryption relies on Windows DPAPI and the UI relies on WebView2.

### System requirements

- Windows 10 / 11, 64-bit or 32-bit
- WebView2 runtime
- Portable build: no installer; data goes to the app-data directory by default

## Development

Requires [Node.js](https://nodejs.org/) and the [Rust](https://www.rust-lang.org/) toolchain.

```bash
npm install

npm run tauri dev    # desktop development
npm run check        # svelte-check
npm run test         # vitest
npm run check:name   # legacy-name guard
```

Build both portable architectures:

```bash
npm run release:exe                                       # x64
npx tauri build --no-bundle --target i686-pc-windows-msvc # x86
```

- x64 output: `src-tauri/target/release/tidepage.exe`
- x86 output: `src-tauri/target/i686-pc-windows-msvc/release/tidepage.exe`
- When packaging, rename `tidepage.exe` to `TidePage.exe` and place it at the archive root.

> Do not publish a bare `cargo build --release` binary: it does not embed frontend assets and the result points at the dev URL.

## Tech stack

- **Tauri 2** + Rust (`rusqlite`, `reqwest`, `image`, `zip`, `quick-xml`, `mobi`, and more)
- **SvelteKit 2** + **Svelte 5 (runes)** + TypeScript + Vite
- **PDF**: pdf.js (`pdfjs-dist`) in the frontend + native `Windows.Data.Pdf` on x64
- **Local storage**: SQLite (WAL); **credentials**: Windows DPAPI; **cloud settings backup**: PBKDF2-HMAC-SHA256 + AES-256-GCM
- Hand-written CSS design tokens, no Tailwind or preprocessor

## Feedback & support

- Issues and suggestions: [GitHub Issues](https://github.com/miipa/TidePage/issues)
- Project home: <https://github.com/miipa/TidePage>
- QQ group (Chinese): <https://qm.qq.com/q/5bbjW2NIUU>
- If TidePage is useful, you can support development via the QR code in **Settings → About**.

## License

[MIT](LICENSE)