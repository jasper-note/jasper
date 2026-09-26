# Jasper

**English** · [中文](README.zh-CN.md)

> 🌐 **Live demo (runs entirely in your browser via WASM, no server):** https://jasper-note.github.io/jasper/

A lightweight, **read-write** [Joplin](https://joplinapp.org/)-compatible client: a local **Rust (axum) server + browser SPA** — no Electron, Tauri, or WebView. It reads and writes your existing Joplin sync library directly, so edits are picked up by Joplin on its next sync.

> [!NOTE]
> **Not affiliated with Joplin.** Jasper is an independent, unofficial project that is merely *compatible* with the open [Joplin](https://joplinapp.org/) sync format. It is **not** produced, sponsored, endorsed by, or otherwise legally affiliated with Joplin or its authors. "Joplin" is a trademark of its respective owner and is used here only nominatively, to describe data-format compatibility.

![Reading view](docs/screenshots/01-reading.png)

## Why

- **Tiny & fast** — the Rust backend stays resident at ~10 MB and starts almost instantly. No 200 MB Electron runtime.
- **Cross-platform** — a single binary on macOS / Windows / Linux; the UI is just a browser tab.
- **Your data, your files** — operates directly on the Joplin sync format (`<id>.md` items + `.resource/` blobs). Nothing is locked into a new database.

## Features

### 📂 Reads & writes real Joplin libraries
- Data sources: **local folder** or **WebDAV** (Nextcloud, etc.).
- Three-pane UI: notebook tree (nested, with note counts) · note list · reader/editor.
- Full-text search across note titles and bodies.
- Create a brand-new library, or connect to an existing one — chosen in a first-run setup wizard. Switch data source at any time from ⚙.

### ✍️ Editing
- **Two editor modes, one click apart** (like Joplin): a **Markdown source** editor (CodeMirror 6) and a **WYSIWYG** rich editor (Milkdown / Crepe). Source is the default; flip to WYSIWYG from the toolbar and your choice is remembered. Both are lazy-loaded — neither bloats first paint.
- **Autosave** while you type; create / update / delete notes. Opening or switching to WYSIWYG never auto-saves on its own — only your edits do. **Cmd/Ctrl+S** saves the current note on the spot (no browser "Save page" dialog).
- Source mode preserves bytes exactly; WYSIWYG reformats the whole note's Markdown on save (an inherent round-trip trade-off) but keeps Joplin `:/id` resource links intact. HTML notes always use source mode.
- Write-back preserves every metadata field verbatim and only refreshes the timestamps — minimal diff, friendly to Joplin's conflict handling.

### 🎨 Rich Markdown rendering
- Code highlighting, tables, task lists, blockquotes.
- **Math** via KaTeX (inline `$…$` and block `$$…$$`).
- Embedded images and attachments via Joplin's `:/resourceId` links.
- HTML notes are sanitized (DOMPurify) and shown as-is.

### 🖼️ Resources & image upload
- **Paste**, **drag-and-drop**, or use the **📎 attach** button in the editor — the file is uploaded as a Joplin resource and a `:/id` reference is inserted at the cursor.
- **Resource manager** panel (🖼 in the top bar): thumbnails, type & size, **reference counts**, rename, delete, and **one-click orphan cleanup** for resources no note links to.

![Resource manager](docs/screenshots/03-resources.png)

### ⚡ Incremental cache
- A local SQLite cache stores each item's content + modification time, scoped per data source.
- On startup it only fetches **new or changed** items — over WebDAV the second launch makes **zero** `GET` requests for unchanged notes, just one directory listing.

### 🔒 Access control
- Optional **access password** (Settings → Access control): once set, every write requires login; visitors without a session get read-only access or nothing, per the rule below.
- **Passwordless read** toggle: on lets logged-out visitors browse read-only (scoped by a per-notebook allow/deny list); off shows a login gate to anyone without a session — handy for sharing an instance for others to read.
- Sessions are Bearer tokens held in memory only (cleared on restart or logout); the password itself is salted + hashed, never stored in plaintext.

![Login gate shown when access control is on](docs/screenshots/06-access-control.png)

### 📝 Editor & 🔎 Search

| Editor (paste / drag / attach) | Full-text search |
| --- | --- |
| ![Editor](docs/screenshots/02-editor.png) | ![Search](docs/screenshots/04-search.png) |

## Quick start

The quickest way to run Jasper is the pre-built Docker image — a single self-contained binary (frontend baked in) on `debian-slim`:

```bash
docker run -p 27583:27583 -v jasper-config:/config \
  ghcr.io/jasper-note/jasper:latest
```

Then open **http://127.0.0.1:27583/**. On first run a setup wizard lets you pick *existing library* / *new library* × *local folder* / *WebDAV*. The `/config` volume keeps your data-source settings and cache across restarts. To expose it on a LAN, add `-e JASPER_HOST=0.0.0.0` (see the [limitations](#compatibility--limitations) first).

Prefer to build the image yourself?

```bash
docker compose up --build   # then open http://localhost:27583/
```

> **Running from source, the single-binary build, the demo library, the browser-only WASM preview, and all environment variables** live in **[dev.md](dev.md)**.

## How it works

```
Browser SPA (Svelte 5 + CodeMirror)  ──HTTP──▶  Rust server (axum)  ──read/write──▶  Local folder / WebDAV
```

The server scans the sync directory, parses every `<id>.md` item into an in-memory index, and exposes a small JSON API (`/api/folders`, `/api/notes`, `/api/resources`, `/api/search`, …). The SPA renders Markdown client-side and rewrites `:/id` resource links to `/api/resources/id`.

## Compatibility & limitations

- Targets the **Joplin v3.x sync format** (sync target version 3), **unencrypted** libraries.
- Handles notes, notebooks, resources, tags, and note-tags; revisions and other internal types are skipped.
- This client does **not** participate in Joplin's sync lock protocol — fine for personal use; if both sides edit the same note, Joplin creates a conflict copy as usual.
- Access-password auth (see **Access control** above) is built in for casual sharing; sessions are in-memory only (lost on restart) — for anything more sensitive, still front it with your own auth/reverse proxy.
- WebDAV passwords are stored **in plaintext** in the local config DB.

## Tech stack

Rust · axum · rusqlite · rayon · ureq — Svelte 5 (runes) · Vite · TypeScript · CodeMirror 6 · markdown-it · KaTeX · highlight.js · DOMPurify.

## License

The application (server, web UI, WASM demo) is licensed under **AGPL-3.0-or-later** (see [LICENSE](LICENSE)).
[`core/`](core) (jasper-core), [`plugin-sdk/`](plugin-sdk) (jasper-plugin-sdk) and [`plugins-examples/`](plugins-examples) are **MIT OR Apache-2.0** — plugins statically link the SDK and copy from the examples, so those parts stay permissive for plugin authors.
