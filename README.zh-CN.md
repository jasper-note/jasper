# Jasper

[English](README.md) · **中文**

> 🌐 **在线 demo（全程在浏览器内由 WASM 运行，无后端）：** https://jasper-note.github.io/jasper/

一个轻量、**可读可写**的 [Joplin](https://joplinapp.org/) 兼容客户端：**本地 Rust (axum) 服务 + 浏览器 SPA**，不依赖 Electron / Tauri / WebView。直接读写你已有的 Joplin 同步库，改动会被 Joplin 下次同步自动拾取。

> [!NOTE]
> **与 Joplin 无关联声明。** Jasper 是一个独立的、非官方的项目，仅与开放的 [Joplin](https://joplinapp.org/) 同步格式**兼容**；本项目与 Joplin 及其作者**没有任何**制作、赞助、背书或法律上的关联。“Joplin” 为其各自所有者的商标，此处仅作指代性使用，用以说明数据格式上的兼容性。

![阅读视图](docs/screenshots/01-reading.zh.png)

## 为什么

- **小而快** —— 后端常驻内存约 10 MB，几乎瞬时启动，没有 200 MB 的 Electron 运行时。
- **跨平台** —— macOS / Windows / Linux 单个二进制，界面就是一个浏览器标签页。
- **数据即文件** —— 直接操作 Joplin 同步格式（`<id>.md` 条目 + `.resource/` 二进制），不把数据锁进新数据库。

## 功能

### 📂 读写真实的 Joplin 库
- 数据源：**本地文件夹** 或 **WebDAV**（Nextcloud 等）。
- 三栏界面：笔记本树（嵌套 + 篇数） · 笔记列表 · 阅读/编辑。
- 按标题与正文全文搜索。
- 可新建空库，也可连接现有库 —— 首次启动由向导引导；随时可在 ⚙ 切换数据源。

### ✍️ 编辑
- **同一个编辑器，两种视图，一键切换**：**实时预览**（Obsidian 风格——光标移到哪句就露出哪句的源码，图片 / PDF / 表格 / 公式就地渲染）与 **Markdown 源码**。两者是**同一个 CodeMirror 6 实例**，切换视图既不重建也不重排；**默认实时预览**，选择会被记住，编辑器整包懒加载、不拖慢首屏。
- 输入即**自动保存**；支持新建 / 修改 / 删除笔记。**仅打开笔记不会触发任何写回**，只有真正编辑才写。**Cmd/Ctrl+S** 立即保存当前笔记（不会弹出浏览器的「保存网页」对话框）。
- 两种视图都**按字节无损保留**：打开与保存都不重排 Markdown，只有你显式执行工具栏的「格式化」命令才会重排。HTML 笔记一律走源码视图。
- 写回时**逐字保留**所有元数据字段、只刷新时间戳 —— diff 最小，对 Joplin 的冲突处理友好。

### 🎨 富 Markdown 渲染
- 代码高亮、表格、任务清单、引用块。
- **数学公式**（KaTeX，行内 `$…$` 与独立 `$$…$$`）。
- 通过 Joplin 的 `:/资源id` 链接内嵌图片与附件。
- HTML 笔记经 DOMPurify 净化后原样显示。

### 🖼️ 资源与图片上传
- 在编辑器里**粘贴**、**拖拽**，或点 **📎 附件** 按钮 —— 文件作为 Joplin 资源上传，并在光标处插入 `:/id` 引用。
- **资源管理**面板（顶栏 🖼）：缩略图、类型与大小、**引用计数**、重命名、删除，以及对无人引用资源的**一键清理孤儿**。

![资源管理](docs/screenshots/03-resources.zh.png)

### ⚡ 增量缓存
- 本地 SQLite 缓存按数据源记录每个条目的内容与修改时间。
- 启动时只拉取**新增或变化**的条目 —— WebDAV 场景下第二次启动对未变笔记发起**零次** `GET`，仅一次目录列举。

### 🔒 访问控制
- 可选**访问密码**（设置 → 访问控制）：一旦设置，一切写操作都需要登录；未登录访客按下方规则只读浏览或什么都看不到。
- **无密码阅读**开关：开启后未登录访客可按笔记本黑白名单只读浏览；关闭则未登录访客一律看到登录闸门 —— 适合把实例分享给别人只读。
- 会话是仅存于内存的 Bearer token（重启或登出即清除）；密码本身加盐哈希存储，绝不明文落库。

![开启访问控制后未登录看到的登录闸门](docs/screenshots/06-access-control.zh.png)

### 📝 编辑 & 🔎 搜索

| 编辑（粘贴 / 拖拽 / 附件） | 全文搜索 |
| --- | --- |
| ![编辑](docs/screenshots/02-editor.zh.png) | ![搜索](docs/screenshots/04-search.zh.png) |

## 快速开始

最快的方式是拉取预构建的 Docker 镜像 —— 一个自带前端的单文件二进制，跑在 `debian-slim` 上：

```bash
docker run -p 27583:27583 -v jasper-config:/config \
  ghcr.io/jasper-note/jasper:latest
```

然后打开 **http://127.0.0.1:27583/**。首次启动可在向导里选择 *现有库* / *新建库* × *本地文件夹* / *WebDAV*。`/config` 数据卷会持久化数据源设置与缓存。要暴露到局域网，加 `-e JASPER_HOST=0.0.0.0`（请先看[限制](#兼容性与限制)）。

想自己构建镜像？

```bash
docker compose up --build   # 然后访问 http://localhost:27583/
```

> **源码运行、单文件打包、演示库、浏览器 WASM 预览，以及全部环境变量** 都在 **[dev.md](dev.md)**。

## 工作原理

```
浏览器 SPA (Svelte 5 + CodeMirror)  ──HTTP──▶  Rust 服务 (axum)  ──读写──▶  本地文件夹 / WebDAV
```

服务扫描同步目录，把每个 `<id>.md` 条目解析进内存索引，对外暴露一组精简 JSON API（`/api/folders`、`/api/notes`、`/api/resources`、`/api/search` 等）。前端在浏览器侧渲染 Markdown，并把 `:/id` 资源链接改写为 `/api/resources/id`。

## 兼容性与限制

- 面向 **Joplin v3.x 同步格式**（同步目标版本 3）、**未加密**的库。
- 处理笔记、笔记本、资源、标签、note-tag；修订(revisions) 等内部类型跳过。
- 本客户端**不参与** Joplin 的同步锁协议 —— 个人使用足够；若两端同时改同一篇，Joplin 会照常生成冲突副本。
- 内置访问密码鉴权（见上文「访问控制」），适合临时/轻量分享；会话仅存内存（重启即失效）—— 更敏感的场景仍建议自行在前面加一层反向代理鉴权。
- WebDAV 密码以**明文**存于本地配置库。

## 技术栈

Rust · axum · rusqlite · rayon · ureq —— Svelte 5 (runes) · Vite · TypeScript · CodeMirror 6 · markdown-it · KaTeX · highlight.js · DOMPurify。

## 许可协议

应用本体（server、web 前端、WASM demo）采用 **AGPL-3.0-or-later**（见 [LICENSE](LICENSE)）。
[`core/`](core)（jasper-core）、[`plugin-sdk/`](plugin-sdk)（jasper-plugin-sdk）与 [`plugins-examples/`](plugins-examples) 采用 **MIT OR Apache-2.0** —— 插件会把 SDK 静态链接进产物、且通常从示例复制起步，这几部分保持宽松协议以免向插件作者传染 copyleft。
