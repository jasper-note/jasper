# Jasper — Development & Build Guide

**English** · [中文](#中文)

For everyday use, see the [Docker quick start](README.md#quick-start). This guide
covers running from source, the single-binary build, the demo library, the
browser-only WASM preview, and the environment variables.

## Running from source

Requirements: a Rust toolchain and [pnpm](https://pnpm.io/).

```bash
# Backend — serves the API and the built UI on http://127.0.0.1:27583
cd server && cargo run                       # uses saved config, or the setup wizard
cd server && cargo run -- /path/to/JoplinDir # bootstrap with a local folder
cd server && cargo run -- https://host/dav/  # or a WebDAV URL

# Frontend
cd web && pnpm install
cd web && pnpm build      # produces web/dist, served by the backend at :27583
cd web && pnpm dev        # or hot-reload dev server on :5173 (proxies /api)

# Tests (see the Tests section below for the full suite)
cd server && cargo test   # Rust unit tests
cd web && pnpm test       # frontend unit tests (Vitest)
```

Then open **http://127.0.0.1:27583/**. On first run the wizard lets you pick
*existing library* / *new library* × *local* / *WebDAV*.

## Tests

Three layers, all run in CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)):

```bash
# 1) Rust unit tests — core (parser / serialize / library) + server (config / storage / cache / webdav)
cd core && cargo test
cd server && cargo test

# 2) Frontend unit tests (Vitest + jsdom): render, i18n, api helpers,
#    and the editor's markdown helpers (web/src/lib/editor/markdown.test.ts)
cd web && pnpm test
cd web && pnpm check         # type-check (also covers *.test.ts)

# 3) Full-stack e2e (Playwright): builds the UI + a real Rust backend against a
#    generated Joplin fixture, then drives a browser end-to-end.
cd web && pnpm build                 # UI must be built (served via JASPER_WEB_DIR)
cd server && cargo build             # backend binary the e2e launcher runs
cd web && pnpm e2e:install           # one-time: download the Chromium browser
cd web && pnpm e2e
```

The e2e harness lives in `web/e2e/`: `make-fixture.mjs` writes a tiny Joplin
library (a notebook, a note with an image, a to-do, a resource), `server.mjs`
is the Playwright `webServer` launcher (fresh temp data + isolated
`JASPER_CONFIG_DIR` each run), and the `*.spec.ts` files cover load/search,
read-view rendering (`smoke.spec.ts`), and the editor. Editing goes through a
single CodeMirror 6 instance with two modes — `source` and `live` preview
(default) — over one buffer that is saved byte-exact (opening or saving never
reformats the markdown): `edit.spec.ts` (a source-mode edit persists),
`editor-hook.spec.ts` (an `input`-phase plugin rewrites the buffer),
`autosave-race.spec.ts`, and `save-shortcut.spec.ts`.

## Single binary

The frontend can be **embedded into the executable** (via
[rust-embed](https://crates.io/crates/rust-embed)) so the whole app ships as one
self-contained file — no separate `web/dist` at runtime:

```bash
cd web && pnpm build                                   # build web/dist first
cd server && cargo build --release --features embed    # → server/target/release/jasper (~5 MB)
```

Copy that single binary anywhere and run it. The `embed` feature is opt-in;
without it the backend serves `web/dist` from disk as before (so plain
`cargo run` still works without building the frontend).

## Building the Docker image

```bash
docker compose up --build   # then open http://localhost:27583/
```

Multi-stage build: Node builds the frontend → Rust embeds it with
`--features embed` → the runtime image is a single self-contained binary on
`debian-slim`. Published images (via
[`.github/workflows/docker.yml`](.github/workflows/docker.yml)) go to
`ghcr.io/jasper-note/jasper`: `main` builds `latest` (+ `sha-…`), and version
tags (`v1.2.3`) get semver tags.

## Try it with demo content

`docs/gen-demo-library.py` generates a small sample library (the one in the
screenshots):

```bash
python3 docs/gen-demo-library.py /tmp/jasper-demo
cd server && cargo run -- /tmp/jasper-demo
```

## Browser-only demo (WASM, no server)

The core (model / parser / serializer / index) is a dependency-light
`jasper-core` crate that also compiles to WebAssembly. With a bundled sample
library it powers a **read-only, server-less preview** that runs entirely in the
browser tab — handy for a static demo (e.g. GitHub Pages).

```bash
# needs: rustup target add wasm32-unknown-unknown + wasm-pack
cd web && pnpm build:demo   # builds the wasm, then a static demo bundle into web/dist
```

It does not affect the native build: when not in demo mode the wasm import is
tree-shaken out entirely.

![Browser WASM demo](docs/screenshots/05-wasm-demo.png)

## Environment variables

| Variable | Default | Purpose |
| --- | --- | --- |
| `JASPER_HOST` | `127.0.0.1` | Bind address; set `0.0.0.0` for LAN / containers |
| `JASPER_PORT` | `27583` | Listen port |
| `JASPER_CONFIG_DIR` | platform config dir | Where `config.db` / `cache.db` live (`/config` in Docker) |
| `JASPER_WEB_DIR` | next to source | Static frontend dir (overrides embedded/disk defaults) |
| `JASPER_SOURCE` | — | First-run bootstrap data source (local path or WebDAV URL) |
| `JASPER_WEBDAV_USER` / `JASPER_WEBDAV_PASS` | — | WebDAV credentials for the bootstrap source |

## Architecture

```
Browser SPA (Svelte 5 + CodeMirror)  ──HTTP──▶  Rust server (axum)  ──read/write──▶  Local folder / WebDAV
```

The server scans the sync directory, parses every `<id>.md` item into an
in-memory index, and exposes a small JSON API (`/api/folders`, `/api/notes`,
`/api/resources`, `/api/search`, …). The SPA renders Markdown client-side and
rewrites `:/id` resource links to `/api/resources/id`. See
[`CLAUDE.md`](CLAUDE.md) for the full module layout and
[`docs/joplin-data-format.md`](docs/joplin-data-format.md) for the data format.

---

<a id="中文"></a>

# 开发与构建指南（中文）

[English](#jasper--development--build-guide) · **中文**

日常使用请看 [Docker 快速开始](README.zh-CN.md#快速开始)。本文覆盖：源码运行、
单文件打包、演示库、浏览器 WASM 预览，以及环境变量。

## 源码运行

需要：Rust 工具链与 [pnpm](https://pnpm.io/)。

```bash
# 后端 —— 在 http://127.0.0.1:27583 提供 API 并托管前端
cd server && cargo run                       # 读已保存配置，或进入向导
cd server && cargo run -- /路径/JoplinDir     # 用本地文件夹引导
cd server && cargo run -- https://host/dav/   # 或一个 WebDAV 地址

# 前端
cd web && pnpm install
cd web && pnpm build      # 产出 web/dist，由后端在 :27583 托管
cd web && pnpm dev        # 或开发热更新(:5173，/api 代理到后端)

# 测试（完整套件见下方「测试」一节）
cd server && cargo test   # Rust 单元测试
cd web && pnpm test       # 前端单元测试（Vitest）
```

然后打开 **http://127.0.0.1:27583/**。首次启动可在向导里选择 *现有库* / *新建库*
× *本地* / *WebDAV*。

## 测试

三层，全部在 CI（[`.github/workflows/ci.yml`](.github/workflows/ci.yml)）里跑：

```bash
# 1) Rust 单元测试 —— core（parser / serialize / library）+ server（config / storage / cache / webdav）
cd core && cargo test
cd server && cargo test

# 2) 前端单元测试（Vitest + jsdom）：渲染、i18n、api 助手，
#    以及编辑器的 markdown 助手（web/src/lib/editor/markdown.test.ts）
cd web && pnpm test
cd web && pnpm check         # 类型检查（也覆盖 *.test.ts）

# 3) 全栈 e2e（Playwright）：构建前端 + 真 Rust 后端（指向生成的 Joplin 测试库），
#    再用浏览器端到端驱动。
cd web && pnpm build                 # 需先构建前端（经 JASPER_WEB_DIR 托管）
cd server && cargo build             # e2e 启动器要运行的后端二进制
cd web && pnpm e2e:install           # 一次性：下载 Chromium 浏览器
cd web && pnpm e2e
```

e2e 相关代码在 `web/e2e/`：`make-fixture.mjs` 写出一个最小 Joplin 库（一个笔记本、
一篇带图笔记、一条待办、一个资源）；`server.mjs` 是 Playwright 的 `webServer`
启动器（每次重建临时数据 + 隔离的 `JASPER_CONFIG_DIR`）；各 `*.spec.ts` 覆盖
加载/搜索、阅读视图渲染（`smoke.spec.ts`）与编辑器。编辑器是单个 CodeMirror 6
实例、两种模式（`source` 源码 / `live` 实时预览，默认后者），共用一份缓冲、保存
逐字节一致（打开与保存都不重排 markdown）：`edit.spec.ts`（源码模式编辑落盘）、
`editor-hook.spec.ts`（`input` 相位插件改写缓冲）、`autosave-race.spec.ts`、
`save-shortcut.spec.ts`。

## 单文件打包

前端可经 [rust-embed](https://crates.io/crates/rust-embed) **编译进二进制**，整个
应用就是一个自带前端的可执行文件 —— 运行时不再需要 `web/dist`：

```bash
cd web && pnpm build                                  # 先构建出 web/dist
cd server && cargo build --release --features embed    # → server/target/release/jasper（约 5 MB）
```

把这个文件拷到任意位置直接运行即可。`embed` 是可选 feature；不带它时后端照旧从磁盘
`web/dist` 托管（所以不构建前端也能直接 `cargo run`）。

## 自行构建 Docker 镜像

```bash
docker compose up --build   # 然后访问 http://localhost:27583/
```

多阶段构建：Node 构建前端 → Rust 用 `--features embed` 把前端内嵌进二进制 →
运行镜像是 `debian-slim` 上的单个自带前端二进制。发布镜像（由
[`.github/workflows/docker.yml`](.github/workflows/docker.yml)）推到
`ghcr.io/jasper-note/jasper`：`main` 构建 `latest`（+ `sha-…`），版本 tag
（`v1.2.3`）打语义化版本标签。

## 用演示内容试一试

`docs/gen-demo-library.py` 会生成一个小型示例库（即这些截图所用）：

```bash
python3 docs/gen-demo-library.py /tmp/jasper-demo
cd server && cargo run -- /tmp/jasper-demo
```

## 浏览器 demo（WASM，无 server）

核心逻辑（数据模型 / 解析 / 序列化 / 索引）抽成了依赖很轻的 `jasper-core` crate，
它也能编译到 WebAssembly。配上内置的演示库，就能做一个**只读、零服务器**的预览——
整页跑在浏览器标签里，适合做静态演示站（如 GitHub Pages）。

```bash
# 需先装：rustup target add wasm32-unknown-unknown + wasm-pack
cd web && pnpm build:demo   # 先编 wasm，再产出静态 demo 到 web/dist
```

不影响原生构建：非 demo 模式下 wasm import 会被整体 tree-shake 掉。

![浏览器 WASM demo](docs/screenshots/05-wasm-demo.png)

## 环境变量

| 变量 | 默认值 | 用途 |
| --- | --- | --- |
| `JASPER_HOST` | `127.0.0.1` | 绑定地址；局域网 / 容器设 `0.0.0.0` |
| `JASPER_PORT` | `27583` | 监听端口 |
| `JASPER_CONFIG_DIR` | 平台配置目录 | `config.db` / `cache.db` 位置（Docker 里为 `/config`） |
| `JASPER_WEB_DIR` | 源码旁 | 前端静态目录（覆盖内嵌/磁盘默认） |
| `JASPER_SOURCE` | —— | 首次引导用的数据源（本地路径或 WebDAV 地址） |
| `JASPER_WEBDAV_USER` / `JASPER_WEBDAV_PASS` | —— | 引导数据源的 WebDAV 账号密码 |

## 工作原理

```
浏览器 SPA (Svelte 5 + CodeMirror)  ──HTTP──▶  Rust 服务 (axum)  ──读写──▶  本地文件夹 / WebDAV
```

服务扫描同步目录，把每个 `<id>.md` 条目解析进内存索引，对外暴露一组精简 JSON API
（`/api/folders`、`/api/notes`、`/api/resources`、`/api/search` 等）。前端在浏览器侧
渲染 Markdown，并把 `:/id` 资源链接改写为 `/api/resources/id`。完整模块划分见
[`CLAUDE.md`](CLAUDE.md)，数据格式见
[`docs/joplin-data-format.md`](docs/joplin-data-format.md)。
