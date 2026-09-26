# Jasper

轻量级 Joplin 兼容客户端：**本地 Rust(axum) 服务 + 浏览器 SPA**，不依赖 Electron/Tauri/WebView。
目标：启动快、内存小（后端常驻 ~10MB）、跨平台（macOS/Windows/Linux）、可读可写。

> 注意：仓库根目录下的 `joplin/` 是**上游 Joplin v3.6.15 源码（仅作格式参考，自带独立 git，已 gitignore）**；
> `JopinData/` 是**用户真实笔记测试数据（隐私，已 gitignore，勿提交/勿误删/勿在其上做破坏性测试）**。
> 本项目代码在 `core/`（纯逻辑）、`server/`（原生后端）、`web/`（前端）、`wasm/`（浏览器 demo）。
> 三个 Rust crate 用 path 依赖串联，**无 workspace**（`cd server && cargo run` 照常）。

## 目录结构

```
core/          纯逻辑 crate (jasper-core)：无 IO，可编译到 WASM
  src/
    model.rs       Note/Folder/Resource/Tag/NoteTag + ItemType/MarkupLanguage 枚举
    parser.rs      解析 Joplin .md 条目（含真实数据单测）
    serialize.rs   写回：update_note_md / new_note_md / new_resource_md / new_id / ISO 时间
    library.rs     内存索引（笔记本树/笔记/资源/标签/搜索 + 增删改）；Library::from_contents
server/        Rust 后端 (axum)，依赖 jasper-core
  src/
    main.rs        启动：加载配置→建索引→挂 API + 托管前端；pub use jasper_core::{...} 重导出
    config.rs      SQLite 配置存储(ConfigStore) + build_storage 工厂 + source_key
    cache.rs       SQLite 增量缓存(CacheStore)：按数据源缓存条目原始内容+mtime
    indexer.rs     从存储拉取(rayon 并行)+增量缓存协调 → 调 Library::from_contents（IO 在此，不进 core）
    storage/       StorageBackend trait
      mod.rs         trait 定义 + is_item_filename + DEFAULT_INFO_JSON
      local.rs       本地文件夹后端
      webdav.rs      WebDAV 后端 (ureq + roxmltree, PROPFIND/GET/PUT/DELETE/MKCOL)
    plugins/       插件宿主（feature = "plugins"；mod.rs 含 feature-off 零成本桩）
      mod.rs         PluginHost 入口/桩、before_save 钩子入口、api_router
      manifest.rs    manifest.toml 解析与校验（含 [[contributes.storage]] / [[contributes.sidebar]] / config_schema）
      runtime.rs     wasmi 沙箱：fuel 切片 + resumable 检查点（CPU-only 墙钟）、StoreLimits、NotesCtx
      host_api.rs    host_call 方法表（log/settings/http.request/notes.*，能力门控 + 提案回传）
      ai.rs          ai.complete 宿主实现（genai：anthropic/openai 兼容 + 自定义 base_url）
      install.rs     zip 安装（zip-slip 防护/体量上限/版本比较）、扫描、卸载
      routes.rs      /api/plugins* + /api/ai/config 路由（列表/安装/启停/设置/资产/命令/ui/editor-transform/auto-approve）
      storage.rs     PluginStorage（storage.* dispatch → StorageBackend 适配）+ 配置校验
      hooks.rs       before-save 串联（插件失败不丢数据）
    api.rs         axum 路由与 handler，AppState
    mcp.rs         MCP server（feature = "mcp"；含 feature-off 零成本桩）：rmcp 的 Streamable HTTP
                   挂在 /mcp，13 个工具全是对 api.rs handler 的薄包装；门控在工具层（见 docs/mcp-server.md）
plugin-sdk/    插件作者 SDK (jasper-plugin-sdk)：ABI 胶水(rt.rs)/宿主封装(host.rs)/
               Storage trait(storage.rs)/register! 宏；共享 jasper-core(serde) 类型
plugins-examples/  宿主测试夹具/参考实现（cdylib → wasm32-unknown-unknown；build-wasm.sh 一键构建；path 依赖仓内 SDK，随宿主共演进；**不对外分发**——用户装的插件见「插件生态仓库」）
  trim-trailing/   before-save 去行尾空白（spec 附录 B 参考实现）
  testbed/         测试夹具（echo/spin/alloc_bomb/bad_json/call_http + command 槽 echo-args/relay/
                   read-note/write-note/make-note/ai-echo/chat + ui 方法（main/notes 两 view），
                   喂宿主限额/能力门控/命令链路/notes 提案/ai/ui 测试；手写 ABI 不用宏；测试自写 manifest 只复用其 wasm）
  webdav-storage/  存储 provider 参考实现：WebDAV over host:http（对照内置 webdav.rs 的等价测试 = PluginStorage 适配器覆盖）
wasm/          浏览器 demo crate (jasper-wasm)：jasper-core + 内置演示库 → wasm-bindgen
  src/lib.rs / demo.rs   暴露 folders/notes/note/search（只读），内置纯文本演示库
web/           Svelte 5 (runes) + Vite + TS 前端
  src/
    App.svelte         三栏布局（+右侧插件 dock 第四栏）+ 状态/配置流程
    lib/
      api.ts           后端 API 客户端 + 类型
      render.ts        markdown 渲染（markdown-it + 插件 + highlight.js + KaTeX + DOMPurify）
      FolderTree.svelte / NoteList.svelte / NoteView.svelte
      CodeMirrorEditor.svelte  CodeMirror6 编辑器（懒加载；源码/实时预览两模式、同一 EditorView，切模式只重配隔间）
      editor/          编辑器内核：build(建 view)/setup(基础扩展+mode/readOnly 隔间)/livePreview(实时预览装饰)
                       /widgets(图片·音视频·PDF·附件·表格·公式内联 widget)/handle/commands/theme/types/markdown
      Settings.svelte  首次配置向导 / 设置页（数据源段含插件 provider 动态表单）
      messages.ts      中/英文案字典（zh 基准；`const en: typeof zh` 强制不漏键）
      i18n.svelte.ts   rune 存当前语言 + t() 取词/插值（localStorage 持久化 + 浏览器自动选）
      plugins.svelte.ts  插件列表 rune store + 主题 <link> 注入 + sidebar/命令贡献过滤 + 管理动作
      schema.ts / SchemaForm.svelte  字段词汇(spec §10)的校验与表单渲染（向导/插件设置/form widget 共用）
      PluginPanel.svelte / PluginConsent.svelte  插件管理面板（顶栏 plug）+ 能力授权弹窗
      PluginSidebar.svelte / UiWidget.svelte / ChatWidget.svelte  插件侧栏 dock + server-driven UI 渲染器 + 聊天（多会话持久化）
      selection.svelte.ts / chatSessions.ts  笔记内容区选区捕获（并入命令 args）+ 聊天会话模型(localStorage 多会话)
      pendingWrites.svelte.ts / PendingWriteDialog.svelte / diff.ts  写提案队列 + diff 确认框（spec 0.3）
    shims.d.ts         无类型 markdown-it 插件的最小声明
docs/joplin-data-format.md   逆向出的 Joplin 数据格式规范（解析层依据）
docs/plugin-spec.md          插件规范 v0.3（契约）；docs/plugin-design.md 架构决策
docs/mcp-server.md           MCP server（工具清单 / 门控为何在工具层 / 为何复用 handler）
Dockerfile / docker-compose.yml / .dockerignore
```

## 开发与运行

```bash
# 后端（默认读已保存配置；无配置则进入“未配置态”，由前端向导配置）
cd server && cargo run                 # 监听 127.0.0.1:27583
cd server && cargo run -- <路径或URL>   # 首次引导用数据源（会存进配置）
cd server && cargo test                # 运行单元测试（parser/serialize/webdav）

# 前端
cd web && pnpm dev                     # 开发热更新(5173)，/api 代理到 27583
cd web && pnpm build                   # 产出 web/dist，由后端在 27583 直接托管

# 生产：构建前端后访问 http://127.0.0.1:27583/

# 单文件打包（前端内嵌进二进制，运行时不依赖磁盘上的 web/dist）
cd web && pnpm build                            # 必须先有 web/dist
cd server && cargo build --release --features embed   # 产物 server/target/release/jasper

# MCP server（把笔记库暴露给 Claude Code 等客户端；默认构建不含，见 docs/mcp-server.md）
cd server && cargo build --features mcp
claude mcp add --transport http jasper http://127.0.0.1:27583/mcp   # 设了访问密码再加 Bearer 头

# 本地起一个 WebDAV 服务端联调（hacdias/webdav，端口 8081，账号 joplin/joplin）
docker compose -f docker-compose.dev.yml up -d
cd server && JASPER_SOURCE=http://127.0.0.1:8081/ \
  JASPER_WEBDAV_USER=joplin JASPER_WEBDAV_PASS=joplin cargo run
docker compose -f docker-compose.dev.yml down -v   # 用完清理（含数据卷）
```

环境变量（见 `server/src/main.rs` 头注释）：
- `JASPER_HOST`（默认 127.0.0.1；局域网/容器设 0.0.0.0）
- `JASPER_PORT`（默认 27583）
- `JASPER_CONFIG_DIR`（配置库目录；默认平台配置目录 `jasper/config.db`）
- `JASPER_WEB_DIR`（前端静态目录；默认相对源码，容器里指向打包路径）
- `JASPER_READ_ONLY`（truthy=1/true/yes/on → 只读引导；仅当尚无保存配置时生效，之后以配置库为准）
- `RUST_LOG`（日志级别过滤，`tracing_subscriber::EnvFilter` 语法；不设时默认 `jasper=debug,tower_http=debug,info`——本项目内部与每条 HTTP 请求默认就是 debug 级可见，排障不用先设环境变量；调小用 `RUST_LOG=warn` 等）
- 首次引导：`JASPER_SOURCE` / `JASPER_WEBDAV_USER` / `JASPER_WEBDAV_PASS`

日志：用 **tracing**（+ `tracing-subscriber` env-filter）而非裸 `println!`/`eprintln!`；`api::router()` 挂了 `tower_http::trace::TraceLayer` 自动记每个 HTTP 请求（method/uri/status/latency）。写路径的单一咽喉 `persist_note_blocking`、插件调用的单一咽喉 `PluginHost::dispatch_with_notes` 都打了 debug 级日志，覆盖 API 写入/插件免确认直写/前端全部请求/一切插件调用，无需在各处分别加日志。测试里的 `跳过：...`/`skipping: ...` 提示仍用裸 `eprintln!`（多为无 subscriber 的场景，tracing 会静默不可见）。

## 开发工作流：新任务用独立分支 + worktree

- 每次开始一个新的 fix/feat，先用 `EnterWorktree`（`name` 用简短描述，如 `fix/xxx`/`feat/xxx`）开一个独立分支 + worktree，任务内独立提交、互不干扰；跨会话想回去用 `EnterWorktree({ path })` 重新进入。任务完成合并或废弃后用 `ExitWorktree`（`keep` 保留现场 / `remove` 连分支一并清理）。
- **进入新 worktree 后先接好构建缓存**，否则每个 worktree 要重新全量编译/装依赖：
  - Rust：在 worktree 根写一份 `.cargo/config.toml`（已加入 `.gitignore`，不会被提交、也不会自动出现在其它 worktree）指向仓库外的共享 target 目录：
    ```toml
    [build]
    target-dir = "/Users/xvan/agent-home/joplin-lite/.worktree-cache/cargo-target"
    ```
    之后 `server/core/plugin-sdk/wasm/plugins-examples/*` 里跑 `cargo build/test` 都会自动复用这份编译缓存，只增量编译改动的部分。
  - 前端：`web/node_modules` 仍需在每个 worktree 里跑一次 `pnpm install`，但 pnpm 全局 store 内容寻址、装得很快，不用额外配置。

## 浏览器 WASM 构建（无 server）：只读展示 + 可写本地应用

`jasper-core`（model/parser/serialize/library）编译到 wasm，做成**零服务器**的两种前端构建（都无后端、走 WASM 内核）。设计文档见 `docs/wasm-app-design.md`。三态 api（`web/src/lib/api.ts`）：`WASM_WRITABLE?localApi:IS_WASM?demoApi:httpApi`。

- **两个门闸标志**：`IS_WASM`（= DEMO||LOCAL，无后端构建，events/plugins 据此关闭）、`WASM_WRITABLE`（= LOCAL，可写持久）、`IS_DEMO`（= IS_WASM && !WASM_WRITABLE，只读展示横幅 + 只读闸门）。`readOnly = IS_DEMO || serverReadOnly || locked`。
- **只读展示站**（`pnpm build:demo` = `build:wasm` + `VITE_DEMO=1 vite build`）：内置纯文本演示库（`wasm/src/demo.rs`，含标签），GitHub Pages 可挂；隐藏所有写入入口，顶部「演示预览」横幅。`demoApi` 走 `new Demo()`（内置库）。截图 `docs/screenshots/05-wasm-demo.png`。
- **可写本地应用**（`pnpm build:local` = `build:wasm` + `VITE_LOCAL=1 vite build`；`pnpm dev:local` 热更）：笔记/笔记本/标签增删改移 + 资源图片，经 WASM 内核读写。`localApi` 走 `Demo.fromRaws(...)`；持久后端**运行时二选一**（`LocalBackend` 抽象：`loadRaws`/`persist`/资源读写）。设置页（数据源/鉴权/AI 无从谈起）隐藏，语言/主题在顶栏独立切换。
  - **idbBackend**（默认，`web/src/lib/localStore.ts`）：从 IndexedDB 恢复/首次用 `seedItems` 播种；每次写后 `saveRaws(inst.snapshot())` 全量快照落库；资源二进制存 IndexedDB。刷新不丢。
  - **fsaBackend**（真实 Joplin 文件夹，`web/src/lib/fsStore.ts`，仅 Chromium）：`showDirectoryPicker` 选文件夹 → 读 `<id>.md`（过滤 `^[0-9a-fA-F]{32}\.md$`）→ `fromRaws`；写 = **per-file 增量**（`idOfRaw` 解析 id + `prevSnapshot` diff，只写改动/删已移除的 `<id>.md`，不整目录重写）；`.resource/<id>` 二进制直读写。写回同 `serialize.rs` 格式 → Joplin 可同步。句柄存 IndexedDB `jasper-fs` 持久，回访 `queryPermission` 自动重连、过期 UI 提示「重新连接」（`requestPermission` 需用户手势）。顶栏「打开文件夹」控制挂 `export const localFolder = WASM_WRITABLE ? {...} : undefined`（native 折叠）。Firefox/Safari 无目录选择器 → 显示不支持提示（导入/导出降级待做，见设计文档 §9.1）。
  - `resourceUrl` 同步返回 blob URL（启动/切后端时按当前后端预载资源建映射）。
- **WASM 内核写方法**（`wasm/src/lib.rs`）：镜像 `api.rs` handler（createNote/updateNote/moveNote/deleteNote/create|rename|move|deleteFolder/add|removeNoteTag/upsertResourceMeta/rename|deleteResource），`Demo` 除 `Library` 外另存全类型 `raws: id→原始.md`（rename/move 笔记本/标签/资源需原始内容、也是 snapshot 来源）。**坑：`serialize::now_ms()` 用 `SystemTime::now()` 在 wasm32 上 panic**——所有写方法把 `now:f64` 当参数由 JS 注入 `Date.now()`，本层禁调 `now_ms()`；`new_id()` 走 getrandom js 后端可用。
- **不影响原生**：`DEMO`/`LOCAL` 是编译期常量，`false` 时三元折叠、`demoApi`/`localApi`/`localFolder` 连同 wasm 动态 import 与 fsStore 一起被 tree-shake 掉，原生构建既不打包也不依赖 `web/src/wasm-pkg`（wasm-pack 生成、已 gitignore）。需先装 `rustup target add wasm32-unknown-unknown` 与 `wasm-pack`。
- 测试：`localStore.test.ts`（IndexedDB 往返/播种，fake-indexeddb）+ `fsStore.test.ts`（fake `FileSystemDirectoryHandle` 内存实现，读/写/删/资源/文件名过滤）；端到端 `build:local` 实机验证（IndexedDB：读+写+刷新持久；FSA：Playwright 注入 fake `showDirectoryPicker` 驱动 打开文件夹→读→新建→per-file 写回）。

## README 预览图（截图）

- `docs/screenshots/` 下 `01-reading`/`02-editor`/`03-resources`/`04-search` 各出**中英两套**：英文用基名（`*.png`，供英文 `README.md`），中文加 `.zh` 后缀（`*.zh.png`，供 `README.zh-CN.md`）。UI 语言与笔记内容都随之切换（英文页配英文笔记，中文页配中文笔记）。`05-wasm-demo.png` 单独手工出，不在此流程内。
- 一键重出：`cd web && pnpm shoot`（`scripts/shoot.mjs`）。前置：先 `pnpm build`（要 `web/dist`）+ `cd server && cargo build`（要 debug 二进制）。它对 en/zh 各自：`scripts/demo-library.mjs` 生成对应语言的演示库（临时目录，隔离配置）→ 起真后端 → Playwright（1280×820 @2x=2560×1640，浅色）截 4 张图。
- 浏览器：默认用 Playwright 自带 chromium；本机未下载对应版本时设 `SHOOT_CHANNEL=chrome` 用系统 Chrome（`pnpm shoot` 亦可 `SHOOT_CHANNEL=chrome pnpm shoot`）。
- `scripts/demo-library.mjs` 是 `docs/gen-demo-library.py` 的双语 JS 版（结构对齐、字段对齐 `serialize.rs`），仅供出图；改文案时两语言都要改。
- `06-access-control(.zh).png`（访问控制生效时的登录闸门）走独立脚本 `cd web && pnpm shoot:auth`（`scripts/shoot-auth.mjs`）：起后端后先 `PUT /api/auth/settings` 设一个演示密码（`passwordless_read:false`）→ 全新浏览器 context（无 `jasper.token`）→ 截 `.locked-gate`。与 `shoot.mjs` 分开是因为其余 4 张图要保持「未配置鉴权」的默认状态，不能共用同一次启动。

## 数据源与配置

- 数据源支持**本地文件夹**和 **WebDAV**，配置（含 WebDAV 密码，**明文**）存到 SQLite（rusqlite bundled）。
- 启动时：已保存配置优先；否则用命令行/环境变量引导；都没有则**未配置态**（前端弹全屏向导：现有库/新建库 × 本地/WebDAV）。
- `AppState` 用 `RwLock<Option<Arc<dyn StorageBackend>>>` 支持**运行时切换数据源**（设置页 ⚙）。
- 新建库 = `StorageBackend::init_new()`：建根目录 + `.resource/` + 写默认 `info.json`(v3)。
- **增量缓存**：配置目录下另有 `cache.db`，按 `source_key`（不含密码）隔离缓存每条 `<id>.md` 的 `mtime+原始内容`。
  启动/切换数据源时只对 `list_items()` 里新增或 mtime 变化的条目发起拉取（WebDAV 省去逐个 GET），未变的复用缓存，已删的清理。
  缓存陈旧无害（任何写入都会刷新 mtime → 下次视为变化）；`cache.db` 删除最坏退化为一次全量拉取。`AppState.cache` 持有 `CacheStore`。

## 插件系统（spec 0.3，feature = "plugins"）

- **规范/设计**：`docs/plugin-spec.md`（契约，apiVersion 0.3）+ `docs/plugin-design.md`（决策）。核心决策：**wasmi 沙箱**（纯 Rust 解释器）、JSON ABI（`plugin_dispatch`/`host_call`，指针+长度）、插件 Rust-only 编到 `wasm32-unknown-unknown`、zip 包（`.jplug`）+ `manifest.toml`、能力默认全拒。
- **构建**：`cd server && cargo build --features plugins`（默认构建**零新增依赖、行为完全不变**；wasmi/zip/toml/genai 全 optional）。示例插件：`plugins-examples/build-wasm.sh`（需 `rustup target add wasm32-unknown-unknown`；产物 `plugin.wasm` 已 gitignore，缺失时相关测试自动跳过）。
- **能力（capabilities）**：`settings`（插件作用域 KV）、`host:http`（宿主 ureq 代理的 HTTP(S)，响应 ≤128MiB、重定向 ≤5、**非 2xx 以 ok 带 status 返回**）、`notes:read`（get/search/list_folders，读内存索引）、`notes:write`（upsert/create，提案回传见下）、`host:ai`（ai.complete 见下）；免能力的 `log`、`time.now`（沙箱无时钟，SigV4 等签名协议用）、`system.locale`（0.4，读当前 UI 语言代码——插件据此本地化**自己运行时产出的文字**；宿主持久化 UI 语言，见下）。无裸 socket——**SMB/NFS 是非目标**（OS 挂载 + local 数据源覆盖）。
- **notes/ai 仅 command/ui 上下文（0.3）**：路由层把 `NotesCtx`（library Arc / 存储快照 / read_only / 免确认 / tokio Handle / AI 配置 / 提案累积器 `Arc<Mutex<Vec>>`）经 `dispatch_with_notes` 传入；hooks/storage 分发无此上下文 → `unsupported`（防写入重入）。`AppState.library` 为此 Arc 化。
- **写确认 = 提案回传（0.3）**：`notes.upsert/create` 默认**不落盘**，返回 `{note, pending:true}`，提案随本次 command/ui 响应顶层 `pending_writes[]` 带回 → 前端 `PendingWriteDialog` 弹行级 diff（`web/src/lib/diff.ts`，LCS 带规模预算）确认，同意走现有 `PUT/POST /api/notes*`（before-save 钩子照常）。「免确认」开关**宿主托管**（`PUT /api/plugins/{id}/auto-approve`，存 settings 表键 `plugin_write_auto_approve:<id>`——**不进 plugin_settings**，否则插件可经 settings 能力自改绕过确认；插件面板内切换）；开启后直写（共享保存原语 `api.rs::persist_note_blocking`，**跳过 before-save 钩子**防重入，两路径字节可能不同——spec §11 记录在案）。pending create 的 `note.id` 为空串（批准时生成）。只读模式写方法全 forbidden。
- **host:ai（0.3）**：宿主用 **genai 0.6** 代理（reqwest+rustls，仅 plugins 构建承担）；`AiConfig{provider: anthropic|openai, base_url?, api_key, model}` 存 config.db（`GET/PUT /api/ai/config`，api_key 回显同 webdav_pass 姿势；设置页「AI」段配置）。openai = chat-completions 兼容协议 + 自定义 base_url（Ollama/DeepSeek/中转都走它）。genai 是 async → 在 spawn_blocking 里 `NotesCtx.handle.block_on`；网络等待计入 io_time（CPU 墙钟豁免）。未配置 → `internal` 码带设置页指引（不用 `unsupported`——会被 SDK 误读为宿主太旧）。options 钳制 temperature 0..=2、max_tokens 1..=32768。**测试坑**：开发机 HTTP_PROXY 会把 127.0.0.1 stub 请求送进代理（502）——AI stub 测试须设 `NO_PROXY`（与 e2e playwright 配置同一坑）。
- **侧边栏 + widget 宿主（0.3）**：manifest `[[contributes.sidebar]]{id,title,icon?,widget,command?,view?}`——**静态模式**（如 widget=chat + command：消息**前端持有**，发送 args=`{messages,input,note_id,selection?}`，`result.reply` 追加为 assistant 消息）或**动态模式**（view → `POST /api/plugins/{id}/ui/{view}` 调 wasm `ui` 方法取 UiNode 树 `{type,props,children}`；命令 `result.ui` 换树）。前端：左栏底部入口按钮 → 右侧第四栏 dock（App.svelte `.panes.with-dock` 320px + `PluginSidebar.svelte`）；`UiWidget.svelte` 递归渲染 widget（0.3 六种 chat/list/tree/form/markdown/button；**0.4 阶段 4 扩至十种**，加独立控件 `checkbox`/`select` + 布局原语 `divider`/`heading`；form 复用 SchemaForm；文本字段走 §9.2.1 locale map；未知 type 连 children 安全忽略）；一切 sidebar 命令并入当前 `note_id`。事件契约钉死在 spec §9.2。
- **选区上下文 + 会话持久化（spec §9.2 统一约定，2026-07-04）**：① **选区**——`selection.svelte.ts` 全局监听 `selectionchange`、记住**笔记内容区**（`data-ai-selectable` 标在 NoteView 阅读 `.content` + 编辑 `.editor-wrap`）里选中的文字（点进聊天不丢、切换笔记 App `$effect` 清空）；`PluginSidebar::runCommand` 非空时并入 `selection={text}`（所有 sidebar 命令，不止 chat）。ChatWidget 顶部 composer 显示「已选中 N 字」chip（可 ×）。老宿主不带此键，插件须容忍缺失。② **多会话 + 持久化**——`chatSessions.ts`（纯模型：`SessionStore{sessions[],activeId}` + localStorage 按 `pluginId/panelId` 隔离 + 不可变增删改）；`ChatWidget.svelte` 加会话头（切换/新建/删除/重命名，`storageKey` 空则退化纯内存单会话，旧行为不变）。对插件透明——契约仍是每次把完整 `messages` 传给命令、插件无状态。测试 `selection.test.ts`/`chatSessions.test.ts`/`ChatWidget.test.ts`（持久化+隔离+新建）。**消费方**：`jasper-plugins` 的 ai-chat 0.2.0（注入选区提示词 + `replace_in_note` 精确替换工具）。
- **命令 + 工具栏贡献**：manifest `[[contributes.command]]`（target=backend 调 wasm `command` 方法）+ `[[contributes.toolbar]]`（location=note-toolbar）→ 编辑态工具栏出现按钮（源码/实时预览两模式均可）。宿主 `POST /api/plugins/{id}/commands/{cmd}` 以 `args={note_id,title,body}` 调用；返回 `result.body` 则替换编辑缓冲（走自动保存）。只读模式下该端点被写守卫拦截。参考实现 `ai-polish`（一键优化，在 jasper-plugins 仓库）；宿主链路测试夹具是 testbed 的 `relay` 命令（routes.rs::backend_command_end_to_end）。SDK 用 `register! { command: fn(&str,Value)->Result<Value,PluginError> }` 接入（宏可组合 before_save/storage/command/ui/editor 任意子集，0.3 增 `ui` 槽、0.4 增 `editor` 槽）。
- **编辑期钩子（0.4 阶段 4）**：manifest `[[contributes.editor]]{on}`（`on`∈before-save|input，需 `[backend]`）→ 编辑期把编辑缓冲文本交给 wasm 的 `editor.transform`（spec §3.7/§6.5/§8）改写。宿主 `POST /api/plugins/{id}/editor/transform` 以 `{phase,text}` 调、返回 `{text}`——**纯文本 in/out，走无 notes/ai 上下文的 `PluginHost::dispatch`**（notes.*→unsupported，防写入重入），任一插件失败即跳过（不丢输入）。前端**源码与实时预览两模式都接**（统一到 CM6 后是同一实例，接线在 `CodeMirrorEditor.svelte`）：700ms debounce 输入停顿后依次调 `editorInputPlugins()`（`plugins.svelte.ts`）里各插件、保守整篇替换缓冲——**仅真实用户输入**（`tr.isUserEvent('input'|'delete')`）触发（程序化 dispatch/外部同步不触发 → 天然防环；回填本身不带 userEvent，故不会二次触发）、等待期间用户又敲字（当前 doc≠送出文本）则丢弃变换（不覆盖新输入）。`before-save` 相位前端接入 + spec §3.7 可选 `command` 复用后置（现有服务端 `hook.before_save` 已覆盖保存前改写）。SDK 槽签名 `editor: fn(&str /*phase*/, String /*text*/)->Result<String,PluginError>`。宿主链路夹具 = testbed 的 `editor.transform`（标相位+全大写，`routes.rs::editor_transform_end_to_end`）。
- **限额两档**（fuel 切片 + wasmi resumable 检查点；墙钟只算 CPU，host_call 的 IO 不计）：Normal 64MiB/fuel 1e9/CPU 2s；Storage 256MiB/fuel 5e9/CPU 10s。内存超限 trap 不崩宿主；每次调用新建实例（状态隔离）。
- **存储 provider 扩展点**：manifest `[[contributes.storage]]` + `config_schema`（字段词汇同 settings.schema）→ 插件实现 `storage.*` 8 方法（SDK 的 `Storage` trait）→ 宿主 `PluginStorage` 适配成 `StorageBackend`（rayon 并发安全）。数据源配置存 `SourceConfig`（source_type="plugin" + plugin_id/plugin_storage/plugin_config），`source_key` 用剔除 secret 的 `plugin_config_key`。参考实现 `plugins-examples/webdav-storage`（与内置 webdav 等价对照测试）。
- **语言包扩展点（spec 0.4，`[[contributes.locale]]`）**：插件给应用**增加一门界面语言**（如法语），与主题贡献同构（零代码、纯数据、自动启用）。manifest `[[contributes.locale]]{code,name,base?,messages}`，`messages` 指向 catalog JSON（扁平 `message key → 译文`）；缺失键回落 `base`(en|zh) 再回落内置，未知键忽略；`code` 与内置 `en`/`zh` 相同者被忽略（防劫持）。宿主侧仅 `manifest.rs`（`LocaleContribution` + 校验，`HOST_API_VERSIONS` 加 `0.4`）——catalog 走既有资产端点，`Contributes.locale` 随 `PluginInfo` 透传，无新端点。前端：`plugins.svelte.ts::syncPluginLocales` 在插件列表加载后 fetch catalog → `i18n.svelte.ts::registerPluginLocales`（current locale 泛化为 `string`、`t()` 回落链 当前语言→base→en→zh→键名、来源插件消失回落浏览器默认语言）；顶栏 `LangPicker.svelte` 列内置 + 插件语言。夹具 `web/e2e/fixtures/locale.jplug`（`locale.spec.ts`）。
- **插件运行时读「系统语言」（spec 0.4，`system.locale`）**：与语言包**正交**——语言包翻宿主界面，`system.locale` 让**任意功能型插件**（如 AI 聊天）本地化**自己运行时产出的文字**（chat 回复 / 动态 UI 文案）。落点：UI 语言持久化到 `config.db`（`config.rs::ui_locale`/`set_ui_locale`，前端 `App.svelte` 的 locale $effect 在启动+切换时 `PUT /api/locale`，只读守卫豁免、鉴权守卫照常）；插件经**免能力** host_call `system.locale`（`host_api.rs`，读 `ctx.config.ui_locale()`，未设回落 `en`）取值。SDK `host::system_locale()`（native-host 替身 `set_locale` 注入）。夹具 testbed 的 `read-locale` 命令（`routes.rs::plugin_reads_system_locale` 全链路）。
- **插件返回多语言 UI（spec 0.4 §9.2.1）**：与 `system.locale` 互补的**纯前端**层——server-driven UI / 命令 result 的用户可见文本字段（`markdown.source`/`button.label`/`form.submit_label`/`chat.placeholder`/`list|tree` 的 title/subtitle/命令 `reply`）**可为 locale map** `{ [locale]: string }`，前端按当前语言挑。`i18n.svelte.ts::resolveText`（回落 当前→en→zh→首个非空→''，读 `current` rune 故切换即时重解析）；`UiWidget.svelte`（`text()` 助手替原 `str()` 于文本字段；命令/图标仍走 `str()`）与 `PluginSidebar.svelte`/`UiWidget` 的 chat `reply` 都过 `resolveText`。服务端把 `ui`/`reply` 当不透明 JSON 原样透传，**无端点/格式变更**。测试 `i18n.test.ts::resolveText` + `UiWidget.test.ts`（locale map 渲染 + 回落）。
- **生命周期**：安装（`<config>/plugins/<id>/`）→ 含 `[backend]` 默认 **disabled**，启用=能力授权（前端 PluginConsent 弹窗，host:http 带联网警告）；纯零代码插件（如纯主题/纯语言包）自动启用。活动数据源引用的插件禁止停用/卸载（409 in_use）。能力集扩大后旧授权失效（回禁用态）。
- **前端**：`plugins.svelte.ts`（列表 + 主题 `<link>` 注入 + `registerPluginThemes` 喂 ThemePicker）；顶栏 plug 按钮 → PluginPanel（两 tab：已安装/市场）；向导数据源段动态渲染 provider（SchemaForm）。**探测坑**：feature off 时 SPA fallback 对 `/api/plugins` 回 200 的 HTML——`api.plugins()` 必须查 content-type。
- **插件市场（纯前端，零新端点）**：`market.ts`（纯逻辑可单测：索引解析（强制 `{zh,en}` 双语对象）/`cmpVersion`（移植 manifest.rs）/兼容判断/`sha256Hex`）+ `market.svelte.ts`（rune store：懒加载索引 + 条目状态推导 + 安装动作）。流程：fetch registry 静态索引（raw.githubusercontent 带 CORS，URL 在 `MARKET_INDEX_URL`）→ 按 UI 语言取词 → apiVersion 大版本（前端 `HOST_PLUGIN_API_MAJORS` 镜像宿主）+ `minHostVersion` vs `/api/status` 的 `version` 过滤 → 浏览器下载 `.jplug` → WebCrypto 校验 sha256（不匹配即中止，不给宿主喂可疑字节）→ POST 现有 `/api/plugins/install` → 切回已安装 tab 走 enable/consent。已装同 id 比版本给「更新」按钮（更新=普通 install，同版本重装才要 force）。只读模式隐藏安装按钮。
- **只读模式**：插件管理写操作被 `guard_read_only` 一并拦截；GET（列表/主题资产）放行 → 只读下已装主题继续生效。
- **写插件**：cdylib crate 依赖 `jasper-plugin-sdk`，实现业务后 `sdk::register! { before_save: f, storage: T, command: g, ui: h, editor: e }` 一行接入（五槽任意组合）；不要给插件 crate 引入会带 wasm-bindgen 的依赖（如 chrono 默认 feature——core 已裁掉 wasmbind，getrandom 由 SDK 注册报错桩）。**完整作者指南（脚手架/wasm 工具链坑/测试配方/打包）见 skill `.claude/skills/jasper-plugin/SKILL.md`**——新建或调试插件时先读它。**仓库外插件**用模板仓库 [jasper-note/jasper-plugin-template](https://github.com/jasper-note/jasper-plugin-template)（脚手架 + `scripts/package.py` 校验打包（对齐 manifest.rs/install.rs 规则、查 wasm import 干净、确定性 zip）+ CI：推 `v*` tag 自动出 GitHub Release 挂 `.jplug`）；其 CI 依赖 crates.io 上的 `jasper-plugin-sdk`，SDK 有破坏性改动时要同步更新模板。
- **插件生态仓库**（本机同在 `~/agent-home/jasper-note/` 下）：[jasper-plugins](https://github.com/jasper-note/jasper-plugins)（官方插件 monorepo，**s3-storage/ai-polish/ai-chat 的分发源**；cargo workspace + 共享 package.py；发版=推 `<插件>-v<版本>` tag → GitHub Release 挂 `.jplug`+sha256）、[jasper-plugin-template](https://github.com/jasper-note/jasper-plugin-template)（社区模板）、[jasper-plugin-registry](https://github.com/jasper-note/jasper-plugin-registry)（市场索引 `plugins.json`；**name/description 是 `{zh,en}` 双语对象、两键必填**——生态所有用户可见面（索引、将来的市场 UI 等）都必须中英双语，市场 UI 按当前语言取词）。s3-storage/ai-polish/ai-chat **只存在于 jasper-plugins**（主仓库副本已删，2026-07-02）：插件行为测试随行（SDK 的 `native-host` feature：host_call 在 native 测试下走本地实现——http→ureq、settings 内存注入，ai-polish 打本地 stub、s3 直连 MinIO）；主仓库 `plugins-examples/` 只留 trim-trailing/testbed/webdav-storage 三个宿主测试夹具。
- **before-save 改写不回显编辑器**（易误判为"插件没生效"）：钩子在服务端保存链路里跑，改写落 API 响应与磁盘；`NoteView` 保存后不回填编辑缓冲（自动保存频繁，回填会跳光标）。验证：切走再切回笔记、或看磁盘 `<id>.md`（两种视图都按字节写回，不必再挑模式）。

## API

```
GET    /api/status            是否已配置 + 计数 + 数据源类型 + read_only + auth_enabled/authenticated/passwordless_read
GET    /api/config            当前配置（含 secret；受鉴权保护——匿名 401）
PUT    /api/config            设置/切换数据源（连接+校验+建索引+持久化）
GET/PUT /api/locale           UI 语言持久化（前端切换/启动时 PUT；插件经 host_call system.locale 读同一值；只读豁免）
GET    /api/folders           笔记本树（嵌套 + 篇数）
POST   /api/folders           新建笔记本 { parent_id?, title }（空名 400；父须存在）
PUT    /api/folders/{id}      重命名笔记本 { title }（空名 400；仅改标题+刷新时间，parent_id 等逐字保留）
PUT    /api/folders/{id}/move 移动笔记本 { parent_id }（空=根；防环：禁自身/后代）
DELETE /api/folders/{id}      删除笔记本（级联：后代笔记本 + 其中全部笔记；未分类根/不存在 → 404）
GET    /api/notes?folder=ID   笔记列表（含 task_done/task_total 任务清单进度）
GET    /api/tags              标签列表（含篇数 note_count，按标题排序）
GET    /api/tags/{id}/notes   打了某标签的笔记（按更新时间倒序，未知标签→空数组）
GET    /api/notes/{id}/tags   某笔记的标签（按标题排序）
POST   /api/notes/{id}/tags   给笔记打标签 { title }（trim+不区分大小写复用，无则新建；幂等）
DELETE /api/notes/{id}/tags/{tid}  从笔记去掉某标签（删 note_tag 关联，保留标签本身）
GET    /api/notes/{id}        笔记详情
POST   /api/notes             新建笔记 { parent_id, title?, body?, is_todo? }
PUT    /api/notes/{id}        更新笔记 { title, body }
PUT    /api/notes/{id}/move   移动笔记到另一笔记本 { parent_id }（校验目标存在；仅改 parent_id+刷新时间）
DELETE /api/notes/{id}        删除笔记
GET    /api/resources         资源清单（含 used_by 引用计数，孤儿在前）
POST   /api/resources         上传资源（原始二进制为体，?filename= + Content-Type=mime）→ {id,markdown,…}
GET    /api/resources/{id}    资源二进制（带 mime 头；按 Scope 过滤，resource→note→folder）
PUT    /api/resources/{id}    重命名资源 { title }
DELETE /api/resources/{id}    删除资源（二进制 + 元数据条目）
GET    /api/search?q=...      标题/正文全文搜索
POST   /api/auth/login        访问鉴权登录 { password } → { token } | 401；未设密码 → 400
POST   /api/auth/logout       吊销当前会话 token（读 Bearer；幂等）→ 204
GET/PUT /api/auth/settings    访问控制设置（需 Full）；GET {password_set,passwordless_read,list_mode,folder_list}（不回哈希）；
                                PUT {password?,clear_password?,passwordless_read,list_mode,folder_list} 改/清密码会吊销全部会话
GET    /api/events            SSE 变更流（事件 `change`：{kind: note|folder|library, op: upsert|delete|reload, id}）；
                                一切写路径（API/插件免确认直写/外部 curl）在 persist_note_blocking 等咽喉广播，
                                前端 events.ts 订阅 → App.svelte 去抖合并按需刷新；打开中笔记走 NoteView.applyExternal
                                的 §5.3 保守规则（无未保存输入且内容确不同才替换缓冲）；
                                慢消费者 lagged / 断线重连 → 折算 library reload 全量刷新兜底

（以下仅 --features plugins 构建存在）
GET    /api/plugins                    已装列表：manifest 摘要+contributes+capabilities+enabled+error+write_auto_approve
POST   /api/plugins/install            裸 zip body；201 / 400 bad_manifest / 409 version_conflict(?force=true 覆盖)
DELETE /api/plugins/{id}               卸载（活动数据源引用中 → 409 in_use）
POST   /api/plugins/{id}/enable        { enabled }；enable=能力授权；wasm 加载失败 422
GET/PUT /api/plugins/{id}/settings     GET secret 不回显（secret_set 标记）；PUT 缺键=不变、空串=清除
GET    /api/plugins/{id}/assets/{path} 插件静态资产（仅 enabled；防路径逃逸；no-cache + ?v= 破缓存）
POST   /api/plugins/{id}/commands/{cmd} 执行 backend 命令 { args } → { result, pending_writes }（0.3 起恒带
                                        pending_writes；写提案见「插件系统」节）；插件业务错误按 code 映射状态
POST   /api/plugins/{id}/ui/{view}     server-driven UI（0.3）：{ state? } → { ui: UiNode, pending_writes }；
                                        view 未在 contributes.sidebar 声明 / 插件禁用 → 404
POST   /api/plugins/{id}/editor/transform  编辑期文本变换（0.4 阶段 4）：{ phase, text } → { text }；
                                        phase∈before-save|input（非法 400），该相位未在 contributes.editor 声明/禁用 → 404；
                                        纯文本 in/out（无 notes/ai 上下文、无 pending_writes）；只读被写守卫拦截
PUT    /api/plugins/{id}/auto-approve  notes:write「写入免确认」开关（0.3，宿主托管）{ enabled } → PluginInfo
GET/PUT /api/ai/config                 宿主级 AI 配置（0.3）{ provider, base_url, api_key, model }；api_key 回显

（以下仅 --features mcp 构建存在）
/mcp                                   MCP server 端点（Streamable HTTP，rmcp）。JSON-RPC 全压这一个路径且
                                        一律 POST → **两道守卫都放行它**（否则只读时连 tools/list 都 403、
                                        匿名连工具都列不出），门控下沉到各工具：读工具按 Scope 过滤、
                                        写工具查只读 + 要求 Access::Full。详见 docs/mcp-server.md
PUT    /api/mcp/config                 MCP 开关 + 独立 API key { enabled?, regenerate_key?, clear_key? }
                                        （普通 /api/* 写端点，**不**豁免守卫 → 匿名 401 / 只读 403；
                                        两个 key 动作同时传 → 400。设了 key 后 /mcp 只认它，会话 token 也不行）
```

> **只读模式**：`read_only` 开启时，`api::guard_read_only` 中间件按 HTTP 方法拦截，凡写方法（POST/PUT/DELETE/PATCH）一律返回 `403 {"error":"read_only"}`，**`PUT /api/config` 与 `/api/auth/*` 豁免**（用于在设置页把只读关回去 / 登录改密码）。`/api/status` 与 `/api/config` 返回 `read_only` 供前端遮蔽写入入口。

## 访问鉴权 / 访问控制（access control，`server/src/auth.rs`）

面向「把容器分享给他人阅读」：设一个**访问密码**（设置页配置、存 `config.db` 的**加盐迭代 SHA-256 哈希**，明文永不落库；`sha2` 是 server **直接依赖**——它此前只在 `embed` feature 下传递存在）。

- **两档 `Access`**：未设密码 → 恒 `Full`（行为与无鉴权时完全一致，向后兼容）；设了密码 → 带**有效会话 token**（`Authorization: Bearer`，`getrandom` 随机 hex 存**内存** `HashSet`，重启失效/改密码全清/登出移除）为 `Full`（读全部 + 写），否则 `Anonymous`。
- **匿名可见范围 `Scope`**（`AuthState::scope`）：`passwordless_read`（总开关）**关** → `None`（什么都看不到，前端 `App.svelte` 显示登录闸门）；**开** → 据笔记本黑白名单 `list_mode`：`none`=全库、`whitelist`=仅名单笔记本**及子树**、`blacklist`=名单**及子树**以外（子树用 `Library::subtree_folder_ids` 展开）。
- **两道中间件**：`guard_auth`（最外层）据 Bearer 定 `Access` 塞进请求扩展，拦未授权的写（`/api/auth/login|logout` 豁免）与机密读（`/api/config`、`/api/ai/config`、`GET /api/auth/settings`、`GET /api/resources` 资源清单）；`guard_read_only`（内层）照旧。读 handler（folders/notes/note_detail/search/events_sse **及标签读端点** `tags_list`/`tag_notes`/`note_tags_list`）经 `Extension<Access>` 按 `Scope` 过滤内容，`events_sse` 对受限匿名把事件折算为 reload（不泄露私有 id）。**资源二进制**（`GET /api/resources/{id}`）同样按 `Scope` 过滤：资源本身不知道所在笔记本，靠 `Library::notes_referencing_resource`（据笔记正文 `:/<id>` 引用建的反向索引）找到引用它的笔记 → 取其 `parent_id` 过 `scope.allows_folder`，任一可见即放行；孤儿资源（无笔记引用）在受限范围下判定不可见。
- **前端**：`api.ts` 存 token 于 `localStorage['jasper.token']`、`authHeaders()` 注入所有写请求、401 清 token 触发 `setAuthErrorHandler`；`AuthDialog.svelte` 登录框；`App.svelte` 的 `readOnly = IS_DEMO || serverReadOnly || (authEnabled && !authenticated)` 统一写闸门 + 顶栏解锁/登出按钮 + 私有空态登录闸门；`Settings.svelte`「访问控制」段（设/改/清密码、无密码阅读开关、黑白名单笔记本勾选；设/改密码后自动用新密码重登以保持管理员在线）。
- **已知权衡**：会话内存态（重启需重登）；`config.db` 本就明文存 webdav_pass/AI key，访问密码则哈希存。

## Joplin 格式要点（详见 docs/joplin-data-format.md）

- 条目文件 `<32hex>.md` 平铺在同步根目录，三段式：`标题 \n\n 正文 \n\n key:value 元数据块`。
  **解析从文件末尾往上扫**，结尾连续非空行=元数据，遇第一个空行停；标题=正文段第一行。
- 只读/写处理的 type_：1 笔记 / 2 笔记本 / 4 资源 / 5 标签 / 6 note_tag。
- 资源二进制在 `.resource/<id>`（无扩展名），mime 来自对应 type_=4 元数据条目。
- 笔记 `markup_language`：1=Markdown、2=HTML（**HTML 笔记须净化直显，不能走 markdown 渲染**）。
- 时间戳是 ISO UTC `YYYY-MM-DDTHH:mm:ss.SSSZ` ↔ Unix 毫秒。
- WebDAV 只用 PROPFIND/GET/PUT/DELETE/MKCOL + Basic Auth；忽略 locks/temp/.sync。

## 写回与 Joplin 同步的关系

- 编辑保存 = 读原 `.md` → 改标题/正文 → 刷新 `updated_time`/`user_updated_time` → 其余元数据**逐字保留** → 写回（本地原子 rename / WebDAV PUT）。`library` 缓存了笔记原始内容(raw_notes)，保存无需重新拉取。
- 写回的是**同一个数据源**，Joplin 下次同步会拾取改动；两端都改会由 Joplin 生成冲突副本。
- 本项目**不参与 Joplin 的同步锁协议**（个人使用低风险）。

## 约定与注意点（gotchas）

- **提交信息**：commit message **一律用英文**（后续提交遵循此约定；正文/PR 描述可中文）。
- **许可协议**：应用本体（server/web/wasm）AGPL-3.0-or-later（根 LICENSE 带子目录例外前言）；`core/`、`plugin-sdk/`、`plugins-examples/` 是 **MIT OR Apache-2.0**（各目录有 LICENSE-MIT/LICENSE-APACHE）——插件静态链接 SDK、从示例复制起步，这三处**绝不能混入 AGPL 代码**；上游 `joplin/` 是 AGPL，只作格式参考，**任何目录都不得从中复制代码**。
- **crates.io 发布**：`jasper-core` + `jasper-plugin-sdk` 已发布（SDK 版本 **minor 对齐 spec apiVersion**：0.4.x ↔ apiVersion 0.4；**当前两者同为 0.4.0**——0.3.0 起版本可脱钩，但本次 core 有 tag 序列化改动、SDK 有 `system.locale`，两者一并发到 0.4.0）。发版：改版本（core/plugin-sdk 的 Cargo.toml + 刷各 Cargo.lock，`cargo update -p jasper-core -p jasper-plugin-sdk`）→ 推 `crates-v<SDK 版本>` tag（如 `crates-v0.4.0`）→ `publish-crates.yml` 先 core 后 sdk（幂等，已发布则跳过；需仓库 secret `CRATES_IO_TOKEN`）。SDK 对 core 的依赖 **path+version 双写**，本地走 path、发布走 version（core 升 0.4 时该 version 也要跟到 `"0.4"`）。server/wasm/示例插件均 `publish = false`（对 core/sdk 用**纯 path**依赖，无 version 需改）。
- Rust：tab 缩进、单引号、避免 `any`、注释用 `//`；网络/磁盘 IO 在 axum handler 里走 `spawn_blocking`；`ConfigStore`(rusqlite) 用 `Mutex` 包裹。
- 前端：Svelte 5 **runes**（`$state/$props/$derived/$effect`），事件用 `onclick=` 不是 `on:click`；`NoteView` 按笔记 id 以 `{#key}` 重挂载（先取详情再切 id）。
- **Cmd/Ctrl+S ＝立即保存**：浏览器默认是「保存网页」（另存为对话框），`App.svelte` 顶层 `<svelte:window onkeydown>` 全页 `preventDefault` 拦下（任何界面都不再弹），再调 `NoteView.saveNow()` 冲刷防抖中的自动保存；判定在纯逻辑 `saveShortcut.ts`（只认 Cmd/Ctrl+S，带 Alt/Shift 的组合不抢）。e2e：`save-shortcut.spec.ts`（断言 defaultPrevented + PUT 早于 800ms 防抖）。
- **多语言**：自建轻量 i18n（无第三方包）。新增/改文案要**同时**写 `messages.ts` 的 `zh` 和 `en`（漏键编译报错）。组件里用 `t('key', {插值})`；模板内调用 `t()` 会读 `i18n.svelte.ts` 的 rune→切换语言即时重渲染。纯 `.ts`（如 api.ts）里调 `t()` 取当时语言即可。顶栏「中/EN」按钮切换，localStorage 持久化。
- **编辑器：CodeMirror 6 单实例、两模式**（此前的 Milkdown/Crepe 富文本编辑器已删除，`e9fed5c` 起统一）：`NoteView` 编辑态挂 `CodeMirrorEditor.svelte`，内核在 `lib/editor/`；模式只有 `source`（原始 markdown + 语法高亮）与 `live`（实时预览）——同一个 `EditorView`，切模式走 `Compartment.reconfigure`，不重建、不丢内容与光标（`localStorage('jasper.editor')` 记忆，**默认 `live`**，旧值 `'wysiwyg'` 视为 `live`）；**HTML 笔记(markup_language=2)强制 `source`** 且隐藏切换按钮。整包 `import('./editor/build')` 懒加载，不进首屏。
  - **实时预览**（`editor/livePreview.ts`）：按 Lezer markdown 树打 hide/mark/replace 装饰——光标不在的节点隐去语法标记，光标/选区触及才露出原始 markdown；`:/id` 图片/视频/音频/内联 PDF/附件卡片、水平线、表格、KaTeX 公式、任务复选框替换为内联 widget（`editor/widgets.ts`，widget 区间是 atomicRanges）；围栏代码不替换（只加 `.cm-code-block` 底色，保持可编辑）。
  - **数据安全：两模式都按字节无损**——没有序列化层：文档按原文载入（构造时给定 `doc`、不经事务）、按 `doc.toString()` 原样写回，打开与保存都不重排；唯一重排入口是工具栏显式命令 `md.format`→`formatMarkdown`（`editor/markdown.ts`）。故**仅打开或切模式都不会触发自动保存**（updateListener 只看 `docChanged`），也不再有「富文本 ready 闸门」——两种模式同等无损，无需兜底。
  - **资源图片**：`:/id` 原样留在 markdown 里，只在显示层改写（编辑器内联 widget 与阅读视图 `render.ts` 都走 `api.resourceUrl(id)`）；粘贴/拖拽/📎 上传后插入 `:/id`，`parseResourceId()` 在 `api.ts`。图片 alt 只喂 widget 的 `alt`/说明文字，**不写回缓冲**（旧 Milkdown 的 alt↔caption、图片缩放比例那套改写已随其删除，CRLF 笔记也不再被富文本重排）。
- **依赖哲学：少造轮子但也别堆包**（用户偏好），渲染优先用成熟 markdown-it 插件。
- markdown-it 插件存在 CJS/ESM 默认导出差异，`render.ts` 用 `P()` 助手统一（否则 `md.use()` 抛 `e.apply is not a function`，白屏）。
- 代码高亮固定用 **github-dark** 主题 + 深色代码块背景（浅色模式也清晰）。
- 资源链接 `:/id`（含笔记内嵌的原始 `<img src=":/id">`）在**最终 HTML 上用 DOMParser 统一改写**为 `/api/resources/id`，覆盖 markdown 与 HTML 笔记。
- CodeMirror 经 `import()` 懒加载，单独成 chunk，不进首屏包。

## 测试

三层，均在 CI（`.github/workflows/ci.yml`：`rust-test` / `web-unit` / `e2e`）跑：
- **Rust 单元**：`cd core && cargo test`（parser/serialize/library；`--features serde` 再跑一遍含 serde 往返）+ `cd plugin-sdk && cargo test`（ABI 信封/存储路由/register! ui 槽，native；`--features native-host` 再跑一遍含宿主替身：settings/http/notes 内存库/ai 预置回复）+ `cd server && cargo test`（config/storage/cache/webdav）**及** `cargo test --features mcp`（MCP 描述符下发模板不含 token / 端点豁免两道按方法的守卫 / 开关关掉后 503 /
**独立 API key**：生成回显+拼命令、不带/带错/拿有效会话 token 均 401、revoke_all 后仍可用、重新生成使旧 key 失效、
清除后回落、管理端点匿名 401 只读 403）
**及** `cargo test --features plugins`（manifest/zip 安装/能力门控/限额/before-save/命令链路/存储适配/**notes.* 提案与直写含跳钩子证明/ai.complete 双协议 stub/ui 端点与 pending_writes 全链路/editor.transform 编辑期钩子全链路含相位/守卫/只读分支**）。测试写在各 `.rs` 的 `#[cfg(test)] mod tests`。三类自动跳过（CI 安全）：`parser::parses_all_real_data` 缺 `JopinData/`；wasm 夹具测试缺 `plugins-examples/*/plugin.wasm`（先跑 `plugins-examples/build-wasm.sh`）；webdav 存储插件集成测试未设 `JASPER_TEST_WEBDAV_URL=http://127.0.0.1:8081/`（`docker compose -f docker-compose.dev.yml up -d` 起 hacdias/webdav）。s3/ai-polish 的插件行为测试在 jasper-plugins 仓库（MinIO 也在那边的 CI/compose 里）。
- **前端单元**（`cd web && pnpm test`，Vitest + jsdom）：`src/**/*.test.ts` 与源码同目录。覆盖 `api`(parseResourceId/taskProgress)、`render`(markdown/`:/id`改写/HTML 净化/renderMarkdown)、`i18n`(t 插值/切换/zh-en 键与占位符对齐/**插件语言注册+回落链+可选语言列表+来源消失收敛+resolveText 多语言 UI 字段解析**)、`editor/markdown`(块级切换/格式化纯逻辑)、`schema`/`SchemaForm`(字段词汇校验+渲染)、`plugins`(探测含 SPA-fallback 坑/provider 过滤/sidebar 过滤/主题 link 注入/**editorInputPlugins 过滤**)、`UiWidget`(十 widget 契约含 checkbox/select/divider/heading/未知 type 忽略/**locale map 文本字段按语言渲染+回落**)、`api`(**editorTransform 往返+错误**)、`ChatWidget`(发送往返/**多会话持久化+隔离+新建**)、`selection`(**笔记内容区选区捕获/聊天区不覆盖/清空**)、`chatSessions`(**会话模型增删改+localStorage 往返+损坏兜底**)、`pendingWrites`(确认队列)、`diff`(行级 LCS/超预算退化)、`TagList`(标签区渲染/空时不渲染/点击回调/选中态)、`NoteTags`(加载/打标签 trim/去标签/只读)。`pnpm check` 也会类型检查测试文件。
- **全栈 e2e**（`cd web && pnpm e2e`，Playwright，真起 Rust 后端）：代码在 `web/e2e/`。`make-fixture.mjs` 生成最小 Joplin 库（字段对齐 `serialize.rs`）；`server.mjs` 是 `webServer` 启动器——每次重建临时数据源 + 隔离 `JASPER_CONFIG_DIR`（**否则会读到开发机指向 JopinData 的已存配置**），起 `server/target/debug/jasper` 且经 `JASPER_WEB_DIR` 托管 `web/dist`；`playwright.config.ts` 里把 `127.0.0.1` 加进 `NO_PROXY`（有代理环境时健康检查才连得上）、`webServer.env` 必须并入 `process.env`。specs 覆盖 加载/搜索/渲染/编辑写回、**笔记本级联删除**（`delete-notebook.spec.ts` 自建笔记本+笔记再删掉，不碰 fixture 的 'Notebook'）、**编辑器**（`edit.spec.ts` 源码模式写回落盘、`editor-hook.spec.ts` 输入相位插件改写缓冲、`autosave-race.spec.ts` 在途保存竞态、`save-shortcut.spec.ts` Cmd/Ctrl+S 拦截+立即保存），以及**插件流**（`plugins.spec.ts` 装 `e2e/fixtures/*.jplug`：主题自动启用→ThemePicker→卸载回落 + consent 弹窗；`locale.spec.ts` 装 `locale.jplug`：语言包自动启用→LangPicker 出现法语→选中后搜索占位符切法语→卸载回落；后端须带 `--features plugins` 构建，否则该组自动跳过；`wizard-plugin-source.spec.ts` 用 page.route 伪造 provider 断言向导 payload，无需真插件；`market.spec.ts` 用 page.route 伪造 registry 索引与下载 URL（真夹具字节+真 sha256）覆盖 浏览→安装→已装 + 坏 sha 中止 + 不兼容置灰；`sidebar.spec.ts` 用 page.route 伪造插件列表/ui 树/命令响应，但**写提案批准路径走真 PUT /api/notes** 断言落盘——提案目标用 todoNote（edit.spec 会改 plainNote，避免顺序污染））。夹具由 `e2e/make-plugin-fixtures.py` 生成（zip 已入库）。前置：先 `pnpm build` + `cargo build --features plugins` + `pnpm e2e:install`（下载 Chromium；本机没装时设 `E2E_CHANNEL=chrome` 直接用系统 Chrome）。**端口坑**：本地 `reuseExistingServer` 会复用已占 27599 的进程——若你自己的调试服务恰好挂在该端口，测试会跑在你的库上大片失败（症状：应用能开但找不到夹具笔记）；用 `JASPER_E2E_PORT=27601 pnpm e2e` 换端口，别杀自己的服务。

## 测试数据

`JopinData/`（用户真实数据，gitignore）：约 340 笔记 / 88 笔记本 / 135 资源 / 3 标签 / 4 note_tag，未加密。
parser 单测对其做全量解析校验（计数断言）。**写入类测试务必用临时空目录，不要在 JopinData 上做破坏性操作**（e2e 用生成的临时库，绝不碰 JopinData）。

## 单文件打包（rust-embed 内嵌前端）

- `server` 的可选 feature `embed`（`server/Cargo.toml`）开启后，用 **rust-embed** 在编译期把 `web/dist` 整个塞进二进制；运行时 axum 的 fallback 直接吐内嵌资源（`server/src/web_assets.rs`），不再依赖磁盘上的前端目录 → **单个可执行文件即完整应用**。
- 构建：`cd web && pnpm build` 后 `cd server && cargo build --release --features embed`。**必须先有 `web/dist`**，否则 rust-embed 编译期校验文件夹存在会直接报错。
- 默认（不带 `--features embed`）行为**完全不变**：开发/源码构建仍从磁盘 `../web/dist` 托管，`cargo run` 无需先构建前端也能编译（dev 期前端跑 Vite）。
- 静态托管优先级（`main.rs::attach_web`）：① `JASPER_WEB_DIR` 指定的磁盘目录（两种构建都支持，便于热替换前端） → ② `embed` 构建用内嵌资源 → ③ 源码旁 `../web/dist`。
- SPA 回退：未命中路径回 `index.html`（与原 `ServeDir.not_found_service` 一致）；mime 由 rust-embed 的 `mime-guess` feature 在编译期定。

## Docker

多阶段构建（node 构建前端 → rust 用 `--features embed` 把前端**内嵌**进二进制 → debian-slim 运行）。
运行镜像里**只有一个自带前端的二进制**（不再单独 COPY dist、不再设 `JASPER_WEB_DIR`）。配置目录挂卷 `/config` 持久化。
- 本地：`docker compose up --build`，访问 `http://localhost:27583/`。
- 发布 GHCR：`.github/workflows/docker.yml` **只在推 `v*` tag（前缀匹配）或手动**时构建并推到 `ghcr.io/<owner>/<repo>`（不再每次提交 main 都发包，省资源）。**发版约定（日期式）**：先把 `server/Cargo.toml` version 改成 `YYYY.M.D`（`/api/status` 的 `version` 与市场 minHostVersion 过滤都用它）——**日期一律取 UTC+8（Asia/Shanghai）当天**，不看操作者本机时区或 CI runner 时区（GitHub-hosted runner 默认 UTC，零点前后会跟 UTC+8 差一天）：`TZ=Asia/Shanghai date +%Y.%-m.%-d`（Linux；macOS 无 `%-m/%-d` 用 `date -j -v+8H -u +%Y.%-m.%-d` 或直接 `TZ=Asia/Shanghai date +%Y.%-e.%-d | tr -d ' '`）+ `cargo update -w` 刷 lock，再 `git tag v2026.7.2-1 && git push origin v2026.7.2-1`（`-N` 为同日序号，同一 UTC+8 日期内第二次及以后发版递增）→ 打对应 tag + `latest`。镜像构建带 `--features embed,plugins,mcp`（plugins 自 2026.7.2、mcp 自 2026.9.21；缺 plugins 则插件系统/市场全不可用，缺 mcp 则容器里没有 `/mcp` 端点与设置页 MCP 段）。用内置 `GITHUB_TOKEN`，无需额外 secret。
- WASM demo（`pages.yml`）仍在推 main 时部署，但 `paths-ignore` 掉纯文档/截图提交（`**/*.md`、`docs/**`、`.github/**`），只有前端/core/wasm 改动才重建。
- 拉取运行：`docker run -p 27583:27583 -v jasper-config:/config ghcr.io/<owner>/jasper:latest`。
（注意：当前未做鉴权，容器 0.0.0.0 暴露时谨慎；网络受限地区拉取 Docker Hub 基础镜像可能需镜像加速。）

## 路线 / TODO

已完成：本地+WebDAV 读、增删改编辑（CodeMirror+自动保存）、SQLite 配置+向导、Docker 打包、
**增量缓存**（cache.db 按数据源缓存条目原始内容+mtime，启动只拉取变化项；见 cache.rs / library::build_cached）、
**资源/图片上传**（POST /api/resources 写 `.resource/<id>` 二进制 + `<id>.md`(type_=4) 元数据；编辑器粘贴/拖拽/📎 上传后插入 `:/id` 引用）、
**资源管理面板**（顶栏 🖼：清单+缩略图+引用计数、重命名、删除、一键清理孤儿；见 web/src/lib/ResourcePanel.svelte，引用计数 library::resource_usage 扫正文 `:/id`）、
**单文件打包**（`--features embed` 用 rust-embed 内嵌 web/dist；见上「单文件打包」节）、
**GHCR 发布**（`.github/workflows/docker.yml` 推 main/v* tag 时构建并推 `ghcr.io/<owner>/<repo>`）、
**前端多语言**（中/英，自建 runes i18n；见 messages.ts / i18n.svelte.ts）、
**CodeMirror 6 编辑器**（源码 ⇄ 实时预览两模式、写回按字节无损；见上「约定」里的说明）、
**拖拽移动笔记**（NoteList 项拖到 FolderTree 笔记本→ PUT /api/notes/{id}/move 改 parent_id；core::serialize::move_note_md）、
**记忆上次打开的笔记**（localStorage `jasper.lastNote`，重载按 id 恢复并选中所在笔记本；见 App.svelte restoreLastNote）、
**全站微动效**（Svelte 内置过渡，零依赖：树展开 slide / 列表 fly+flip / 弹层 scale；app.css 有 prefers-reduced-motion 兜底）、
**笔记本创建/重命名/拖拽**（侧栏 folder-plus 建顶层笔记本；行悬停 edit 图标重命名走 `PUT /api/folders/{id}`（core::serialize::rename_folder_md 只改标题+刷新时间）；笔记本可拖到其它笔记本下或「移到顶层」放置区，防环；new_folder_md / move_folder_md，dnd.svelte 共享拖拽态）、
**笔记本删除（级联）**（行悬停 trash 图标 → confirm 报清子笔记本/笔记条数 → `DELETE /api/folders/{id}`；
  服务端据 `library::subtree_folder_ids`/`subtree_note_ids` 展开子树逐条删存储，再 `remove_folders_and_notes` 一次性重建索引；
  部分失败只同步已删成功的部分并报 500；事件超过 64 条折算成一条 library reload。WASM 本地构建同语义 `deleteFolder`）、
**创建待办**（笔记列表 check-square 按钮建 is_todo 笔记）、
**任务清单进度**（core::library::count_tasks 数 GFM `- [ ]/[x]`；列表项进度条 + 笔记头部实时进度）、
**只读模式**（初始化向导/设置页开关，或 `JASPER_READ_ONLY` 引导；`config.db` 存 `read_only`，`AppState.read_only` 运行时可切换。
  前端把 demo 的 `IS_DEMO` 写入闸门推广为 `readOnly = IS_DEMO || 服务端只读`，隐藏一切新建/编辑/删除/拖拽/资源写入并显示「只读」徽标；
  **后端 `api::guard_read_only` 中间件按 HTTP 方法硬拦截**：只读时 POST/PUT/DELETE/PATCH 一律 403（唯 `/api/config` 豁免以便关掉只读），
  确保即使绕过前端也不会意外写仓库；见 api.rs 测试 `read_only_blocks_writes_*`）。
**插件系统阶段 2 + 存储扩展点**（2026-07-02，spec 升 0.2；详见上「插件系统」节）：
plugin-sdk + wasmi 宿主（fuel 切片/内存上限/能力门控）+ zip 安装/启停/设置 + before-save 钩子（trim-trailing 示例）+
`[[contributes.storage]]` 存储 provider（`host:http` 能力；webdav-storage 参考实现，与内置后端等价对照 + 增量缓存集成测试）+
前端（插件面板/consent 授权/主题贡献接入 ThemePicker/向导动态数据源表单 SchemaForm）。默认构建零变化。
**插件生态 + 市场**（2026-07-02；详见上「插件系统」节与「插件生态仓库」）：
jasper-core/jasper-plugin-sdk 发布 crates.io（锁步 0.2.2，SDK 含 native-host 测试替身）+ 模板/官方插件/registry 三仓库 +
前端市场 tab（fetch 双语索引 → 兼容过滤 → sha256 校验安装，零新端点）、
**插件系统阶段 3**（2026-07-02，spec 升 0.3；详见上「插件系统」节）：
notes:read/notes:write（写=提案回传 + 宿主托管免确认）+ host:ai（genai：anthropic/openai 兼容 + 自定义 base_url，设置页 AI 段）+
`[[contributes.sidebar]]` 侧边栏（左栏入口 + 右侧 dock）+ UiWidget 六 widget 渲染器（server-driven UI）+ ChatWidget +
PendingWriteDialog 写确认（自研行级 diff）+ ui/auto-approve/ai-config 端点 + SDK 0.3.0（notes/ai 封装、register! ui 槽、native 替身扩容）。

**变更推送/自动刷新（SSE）**（2026-07-02 落地）：`GET /api/events`（见上 API 节）——服务端 events.rs 广播总线挂 AppState/NotesCtx，
persist_note_blocking 为事件单一咽喉（API 写入/插件免确认直写/外部 curl 全覆盖）；前端 events.ts（EventSource，断线重连合成 reload）+
App.svelte 去抖合并刷新 + NoteView.applyExternal 保守回显（design doc §5.3）。e2e：sse-refresh.spec.ts。
**标签视图 + 打标签（兼容 Joplin）**（2026-07-03 落地）：
- 浏览（只读）：`core::library` 的 `notes_by_tag` 索引（据 note_tags 关联表建，剔除悬挂关联+去重）+ `tags_sorted`/`tag_note_count`/`notes_with_tag`；
  `GET /api/tags` 与 `GET /api/tags/{id}/notes`；前端 `TagList.svelte`（侧栏笔记本树下方的标签区，`tag` 图标）+ App.svelte `selectedTagId`（与 selectedFolderId 互斥、搜索清空可恢复）。
- 打标签（读写）：`core::serialize::new_tag_md`(type_=5)/`new_note_tag_md`(type_=6) **字段集/顺序逐字对齐 Joplin**（含空 parent_id/user_data，无 deleted_time；
  note_tag 纯元数据无标题）；`core::library` 的 `upsert_tag`/`upsert_note_tag`/`remove_note_tag`/`tag_id_by_title`(trim+不区分大小写)/`note_has_tag`/`note_tag_ids_for`/`tags_of_note`；
  语义对齐 Joplin `Tag`（打标签复用/新建 + hasNote 幂等；去标签只删关联保留标签）。API：`GET/POST /api/notes/{id}/tags` + `DELETE .../tags/{tid}`（受只读守卫拦截）。
  SSE 新增 `kind:"tag"` 事件（id=受影响笔记 → 前端刷侧栏标签区/标签视图列表/打开笔记的标签行）。前端 `NoteTags.svelte`（笔记头部 chips + 「添加标签」输入 + datalist 补全）接进 NoteView。
**访问鉴权 / 访问控制**（2026-07-03 落地；详见上「访问鉴权 / 访问控制」节）：容器级访问密码（设置页配置、加盐迭代 SHA-256 存 config.db）+
会话 token（Bearer，内存态）+ `guard_auth` 中间件（写/机密读门控 + `Extension<Access>`）+ 读路径按 `Scope` 过滤——覆盖 folders/notes/search/detail **及标签读端点（tags_list/tag_notes/note_tags_list）**（无密码阅读总开关 + 笔记本黑白名单子树）+
`/api/auth/{login,logout,settings}` + 前端 AuthDialog/统一 readOnly 闸门/设置页「访问控制」段。未设密码时行为零变化。
**资源二进制访问控制补齐**（2026-07-04 落地）：`GET /api/resources/{id}` 接入 `Extension<Access>` + `Scope`——`core::library` 新增
`resource_notes` 反向索引（`build_indexes()` 里据笔记正文 `:/<id>` 引用构建）+ `notes_referencing_resource`，handler 据此把
resource→note→folder 的权限链路接进既有黑白名单规则（同一资源被多篇笔记引用取并集、有一篇可见即放行；孤儿资源受限范围下默认拒绝；
`Scope::All` 零开销跳过）。消除了此前"资源 id 猜不出来"的唯一防线。
**插件系统阶段 4 —— 编辑期钩子 + 扩 widget 词汇表**（2026-07-04 落地，apiVersion 不变仍 0.4——落地既有契约）：
`contributes.editor` → `editor.transform`（spec §3.7/§6.5/§8）编辑期文本变换（manifest 解析校验 + `host.has_editor_transform` 守卫 +
`POST /api/plugins/{id}/editor/transform` 端点，纯文本 in/out、无 notes/ai 上下文，任一插件失败即跳过）+ SDK `register!` 增 `editor` 槽 +
前端**源码与实时预览两模式都接** `input` 相位（接线在 `CodeMirrorEditor.svelte`，700ms debounce → 保守替换缓冲，仅真实用户输入触发、天然防环、陈旧丢弃；`api.editorTransform`/`plugins.editorInputPlugins`）+
`UiWidget.svelte` widget 词汇从六扩到十（补 `checkbox`/`select`/`divider`/`heading`；`WIDGET_TYPES` 同步）。testbed 加 `editor.transform` 夹具；三层测试全绿。详见「插件系统」节与 spec §12.4。
**MCP server**（2026-09-21 落地；详见 `docs/mcp-server.md`）：`--features mcp` 用官方 rmcp SDK 的 Streamable HTTP
把笔记库暴露给 Claude Code 等客户端，`nest_service` 挂在现有 axum router 的 `/mcp`（不另起进程/端口）。
13 个工具（搜索/读/列举 6 + 写 5 + 删 2，删除类带 destructiveHint）**全是对 api.rs 现有 handler 的薄包装**
——axum 提取器都是可手工构造的 tuple struct，故可见范围过滤 / before-save 钩子 / 事件广播 / Joplin 字节格式
都是同一份实现，两条路径不会漂移（代价：api.rs 若干 pub(crate)）。
**门控在工具层不在中间件**：MCP 的 JSON-RPC 全压一个路径且一律 POST，按方法拦截的两道守卫对它失真
（只读会让 MCP 整个不可用、匿名连工具都列不出），故 `is_mcp_path` 放行两道守卫，改由各工具
`deny_read_only` + `require_full`（写）/ 传 `Access` 给 handler 按 `Scope` 过滤（读）。
**独立 API key**（`jasper_mcp_<64hex>`，`auth::gen_mcp_key`，明文存 config.db 同 webdav_pass/AI key 口径——
要能在设置页回显给用户复制；`secret_eq` 常数时间比较）：与浏览器会话**解耦**，服务重启/登出/改密码都不失效
（会话 token 是内存态 HashSet，这三种情况全失效，而 MCP 客户端的请求头写死在配置文件里，没法像浏览器那样随手重登）。
一旦设置，`/mcp` **只认它**——不带/带错/拿有效会话 token 都 401（否则实例没设访问密码时，设了 key 等于没设）；
没设则回落既有行为。管理走普通的 `PUT /api/mcp/config`（受两道守卫约束），故 key 丢了也锁不死自己；日志绝不记 key。
运行时开关存 config.db（默认开，设置页可关 → 端点 503）；设置页 MCP 段走既有 server-driven 描述符，
为此给字段词汇加了 `copy`（只读+复制）/`note`（纯展示）两型、`show_if.truthy:false`（没值时才显示）与
`on_success:"reload-section"`（重拉描述符+重挂分区，回显服务端新生成的 key）。端点地址由服务端下发 `{origin}`
模板+前端填（服务端不知道客户端从哪个地址访问）；认证头在配了 key 时由服务端直接拼死、没配才留 `{header}`
让前端填会话 token（服务端不该知道后者）。Docker 镜像已带 mcp。

待办：全局改标签名/删标签（写/删 tag 条目 + 级联 note_tag）、E2EE 解密（按需）；插件阶段 4 后续（`before-save` 相位前端接入 + `contributes.editor` 的可选 `command` 复用，见 docs/plugin-design.md §11）。
