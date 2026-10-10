<div align="center">

<img src="docs/images/logo.png" width="112" alt="TidePage">

# TidePage

**A desktop e-book reader built for local libraries with 100,000+ books**

TXT · EPUB · MOBI · AZW3 · PRC　｜　Comfortable typography · Comic mode · AI assistant · Text-to-speech

**English** · [简体中文](README.md)

<img src="docs/images/reader-page.png" width="820" alt="TidePage reading view">

</div>

---

## Overview

TidePage is a **Windows desktop reader** built with **Tauri 2 + SvelteKit (Svelte 5) + Rust**, focused on two things: **large libraries** and **comfortable reading**.

Books, covers, progress, highlights, notes and statistics all live in a local SQLite database. No account, nothing uploaded to a server — the app goes online only when you actively use AI, online TTS, or WebDAV.

## Features

### Library: multiple folders · 100k+ books

- Register any number of local folders; your files are never moved or modified.
- Virtualized scrolling with rank-based paging, **designed for 100,000+ books** — smooth even with hundreds of thousands.
- Grid and list views; sort by last read, date added, title, file time or size, with reverse.
- Filter by format, reading state and folder; search by title / author.
- Multi-select, batch delete (to recycle bin or remove only), open a random book, drag & drop to add.
- Open each book in its **own window**.

### Reading experience

- **Reading modes**: page turn, vertical scroll, two-column paging.
- **Chapter transitions**: seamless splice or buffered, with adjustable buffer strength.
- **5 reader themes** (Paper / White / Eye-care green / Night / Black) plus custom background & text colors and a custom background image with opacity.
- **10 UI skins** (Cream, Clay, Pine, Lagoon, Lilac, Graphite, Amber, Abyss, Crimson, Nocturne) with independent light/dark choices.
- **Color schemes**: 5 built-in, save your own, apply globally or per book.
- **Typography**: font size, line height, paragraph spacing, weight, alignment, first-line indent; horizontal and vertical margins are **remembered per mode**.
- **Fonts**: list system fonts, install custom TTF/OTF/TTC fonts, set a separate Latin font.
- Table of contents, bookmarks, in-book search, progress rings, reading stats (last 14 days; per-book characters, time, speed, sessions).
- **Per-book overrides**: font, image scale, show/hide images, block specific images, override CSS, manga toggle.
- Rebindable shortcuts and click/wheel gestures (5 click zones with 3 bounce strengths).

### Comic / manga mode

- **Automatic detection** of comic-like books (by image count and sparse body text), or toggle manually.
- **Vertical continuous (webtoon-style)** and **horizontal continuous (drag to pan)** display.
- Reading direction left-to-right / right-to-left; image scale, fit width/height, margins, image gap (0 = seamless stitching).
- Show or hide images, block individual images with restore; built-in image viewer (wheel paging, click to zoom).

### AI assistant

- Chat with the **current book / chapter**, with chapter content injected as context.
- Selection **translate / explain** popup; results can be added to a highlight note.
- **Custom prompts**: bind each to an F1–F9 hotkey, drag to reorder, optionally auto-run the first prompt on every new chapter.
- Global system prompt, streaming output, two display styles (chat bubbles / reader-style pagination).
- **Providers**: OpenAI-compatible, Ollama (local), Anthropic, Google Gemini, Azure OpenAI; **multi-endpoint failover** and connection test.
- **Network proxy**: auto / direct / system / manual, with SOCKS support, applied to both AI and online TTS.
- Export AI conversations to TXT / EPUB.

### Text-to-speech (TTS)

- System speech and online speech engines; online supports OpenAI-compatible, Azure, DashScope, MiniMax, Fish Audio, GPT-SoVITS, Breeze, Volcano (Doubao), Baidu, Edge TTS and more.
- Rate / pitch / volume, sleep timer, **highlight and scroll-follow the current sentence**, auto-continue to the next chapter.

### Highlights & notes

- 7 highlight styles (underline / wavy / dotted / double / background / strikethrough / box) with custom colors.
- Notes aggregated per book, searchable, jump-to; highlights can survive book deletion.

### Backup & sync

- **WebDAV**: reading-progress sync, upload / download books, browse remote folders, remote settings backup and restore.
- Copy / import settings via clipboard; export config with secrets stripped.
- **Local credentials (AI / TTS / WebDAV keys) are encrypted with Windows DPAPI** per user — ciphertext on disk, plaintext never written to local storage.

### More

- Full-text index (SQLite FTS5) for **whole-library search**, with one-click rebuild.
- Book detail view / edit (title, author, optionally written back into the EPUB), reveal in Explorer, reading history.
- Storage usage view and cache cleanup; system tray, window-state persistence, immersive mode.
- UI languages: Simplified Chinese / English (instant switch).

## Supported formats

- **E-books**: TXT, EPUB, MOBI, AZW3, PRC
- **Text encoding**: automatic detection (UTF-8 / UTF-16 / GB18030 / Big5, …) with a manual override
- **Comic / cover images**: PNG, JPG/JPEG, GIF, BMP, WebP

> Note: PDF and CBZ/CBR archives are not supported; comic content comes from image pages inside EPUB / MOBI.

## Screenshots

### Reading · TOC · Themes

<p>
<img src="docs/images/reader-page.png" width="48%" alt="Reading view">
<img src="docs/images/reader-toc.png" width="48%" alt="Table of contents">
</p>
<p>
<img src="docs/images/reader-ai.png" width="48%" alt="AI assistant">
<img src="docs/images/reader-theme.png" width="48%" alt="Reader themes">
</p>

### Settings

<p>
<img src="docs/images/set-appearance.png" width="48%" alt="Appearance">
<img src="docs/images/set-reader.png" width="48%" alt="Reader layout">
</p>
<p>
<img src="docs/images/set-manga.png" width="48%" alt="Comic mode">
<img src="docs/images/set-tts.png" width="48%" alt="Text-to-speech">
</p>
<p>
<img src="docs/images/set-ai.png" width="48%" alt="AI settings">
<img src="docs/images/set-other.png" width="48%" alt="About">
</p>

## Tech stack

- **Tauri 2** + Rust (`rusqlite` / `reqwest` / `image` / `zip` / `quick-xml` / `mobi`, …)
- **SvelteKit 2** + **Svelte 5 (runes)** + TypeScript + Vite
- Local storage: SQLite (WAL); credential encryption: Windows DPAPI
- Hand-written CSS design-token system, no Tailwind / no preprocessor

## Run & build

Requires [Node.js](https://nodejs.org/) and the [Rust](https://www.rust-lang.org/) toolchain.

```bash
npm install

npm run tauri dev        # development
npm run release          # bundle + exe
npm run release:exe      # exe only, for quick checks
```

> Always build releases with the Tauri CLI (`npm run release`). A bare `cargo build --release` will not build or embed the frontend assets, and the resulting binary points at the dev URL and won't run.

## Requirements

- **Windows 10 / 11 (x64)**
- WebView2 runtime (built into Windows 11; may need a separate install on Windows 10)
- Credential encryption relies on Windows DPAPI, so only a Windows build is published

## Feedback & support

- Project / issues: <https://github.com/miipa/TidePage>
- QQ group: <https://qm.qq.com/q/5bbjW2NIUU>
- If TidePage helps you, feel free to scan the QR code in **Settings → About** to support development.

## License

[MIT](LICENSE)
