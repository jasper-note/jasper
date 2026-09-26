# jasper 插件系统设计方案（草案）

> 状态：**方案讨论中，未实现、未承诺落地**。本文档只做架构决策存档，不含代码、不改构建。
> 目标读者：项目作者本人。
> **配套规范**（给作者/宿主照做的契约）：[plugin-spec.md](./plugin-spec.md)。

### 已定决策（2026-06-30）
- 后端运行时：**wasmi**（纯 Rust 解释器，贴合轻量哲学）。
- ABI 编码：**JSON**（先求好调试，体积后续再说）。
- 后端插件**只用 Rust**；不引入 WIT / 组件模型。
- 前端**不支持任意 JS/iframe 插件**：所有 UI 一律走**我们封装的 widget 词汇表**（见 §7），插件只声明/驱动，不写 UI。
- AI 改笔记：**默认 UI 二次确认**；用户可在设置里改为"默认通过"跳过。
- 分发：**固定 zip 打包格式**；当前只做**本地加载**，远程 URI / 市场索引后置（暂不设计）。
- `host:fetch`（插件网络出口）：**暂不做**（AI 已由 host 代理覆盖）。→ 2026-07-02 被 `host:http` 部分取代，见下。
- 整体 `--features plugins` gate，默认轻量二进制不变。

### 新增决策（2026-07-02，spec 升 0.2）
- **存储 provider 扩展点** `[[contributes.storage]]`：文件后端（OneDrive/Dropbox/云盘…）由插件提供而非全部写进核心；宿主侧 `PluginStorage` 适配成 `StorageBackend`，配置进 `SourceConfig`（source_type="plugin"）。首个参考实现 **WebDAV-as-plugin**（内置 webdav 保留作对照 + 等价测试）。
- **`host:http`**：宿主代理的 HTTP(S)（ureq 执行，限额受控），是插件唯一网络出口；启用确认显式提示联网。
- **SMB/裸 TCP 非目标**：wasm 永远拿不到裸 socket；SMB/NFS 走 OS 挂载 + 内置 local 数据源。
- **安装后默认 disabled**（含 [backend] 时）：enable = 能力授权（consent 弹窗在前端）；纯零代码插件自动启用。
- **provider 暴露**：走 `GET /api/plugins` 的 contributes，前端过滤；不加专用端点。
- **插件管理 UI**：顶栏面板（仿 ResourcePanel），非设置页内嵌。
- **限额两档 + CPU-only 墙钟**：Normal（64MiB/fuel 1e9/CPU 2s）、Storage（256MiB/fuel 5e9/CPU 10s）；host_call 的 IO 时间不计墙钟（wasmi fuel 切片 + resumable call 检查点实现）。

### 新增决策（2026-07-03，spec 升 0.4）
- **语言包扩展点** `[[contributes.locale]]`：应用界面语言可由插件运行时新增（法语/日语等），不把每门语言编进主体。与主题（用例 A）同构——**零代码、纯数据、自动启用、经资产端点托管**，无新端点、无 wasm、无能力。
- **catalog = 扁平 `message key → 译文`**：宿主 message key 集合是宿主内部契约（不进规范冻结）；缺失键回落 `base`(en|zh) 再回落内置，未知键忽略（宿主只增不删旧 key 时旧语言包继续可用）。作者以宿主内置 `en` 目录为权威 key 清单照译。
- **落点**：宿主仅 `manifest.rs`（`LocaleContribution` + 校验 + `HOST_API_VERSIONS += 0.4`），`Contributes.locale` 随 `PluginInfo` 透传；前端 i18n 去静态化——current locale 泛化为 `string`、内置 `en`/`zh` 之外挂一张插件语言表（`registerPluginLocales`），`LangPicker` 列内置 + 插件语言。内置 `en`/`zh` 不可被同名 `code` 劫持。

## 1. 背景与目标

### 1.1 动机
当前 UI 完全是**静态**的：想加侧边栏、换主题、接 AI 面板，都得改代码 + 重新构建 + 发布。希望引入可插拔扩展，让这些能力免重编译地装卸。

### 1.2 三个目标用例
| # | 用例 | 前端 | 后端 | 特点 |
|---|---|---|---|---|
| A | **主题** | ✅ 仅 CSS | ❌ | **零代码**插件，只有样式 |
| B | **AI 插件** | ✅ 对话侧边栏 | ✅ 调 AI、读/改笔记 | 前后端都要、要网络、要笔记访问 |
| C | **编辑插件** | 视情况 | 视情况 | 保存前格式化 / 输入检测 |

### 1.3 关键观察
三用例横跨两个扩展面：
- **后端可计算面**（沙箱安全）：AI 编排、格式化、笔记读写 → **WASI/Rust(wasmi)**。
- **前端贡献面**（碰 DOM，难沙箱）：主题、侧边栏、按钮、编辑钩子 → **WASI 干不了**。

> 解决"UI 去静态化"必须有前端扩展机制；但本项目**不开放任意前端代码**——前端扩展一律收敛成"主题 CSS"+"声明式 widget"两种安全形态（§7）。

### 1.4 非目标
- 不做多语言插件生态（后端 Rust-only，去掉 WIT 包袱）。
- **不做任意 JS/iframe 插件**：始终走封装能力。
- 不参与 Joplin 同步锁协议。

## 2. 核心判断（前提）
1. **真的需要插件吗？** 价值只在 (a) 第三方写扩展 或 (b) 用户免重编译装卸。本方案目标是 (b)。
2. **后端 Rust-only** → host 与插件**共享 `jasper-core` 类型 + JSON serde ABI**（§6.2），无需 IDL。
3. **双层架构**：声明式前端贡献 + 后端 wasmi 沙箱，靠 manifest 串联。
4. **封装而非放权**：前端不给任意 JS，只给主题 + widget 词汇表；后端不给裸 IO，只给受控能力。
5. **feature-gate**：`--features plugins` 才链入 wasmi，默认构建不变。

## 3. 总体架构

```
┌─────────────────────────── web (SPA) ───────────────────────────┐
│  贡献宿主 (Contribution Host)                                    │
│   • 读 manifest → 渲染贡献点：主题/侧边栏/工具栏/命令/设置        │
│   • 主题：注入 CSS / 切 data-theme（零代码）                      │
│   • 面板/表单：用内置 widget 渲染（插件给“声明”，不给 UI 代码）   │
│   • 命令分发：→ 内置处理 或 → 后端 wasm 导出                      │
└───────────────────────────────┬─────────────────────────────────┘
                                 │ HTTP /api/plugins/*
┌───────────────────────────────┴───────── server (axum) ─────────┐
│  PluginHost  (feature = "plugins", 运行时 = wasmi)               │
│   • 启动扫描插件目录 → 解析 manifest → 实例化 .wasm              │
│   • 钩子注入：on-before-save（改 update_note/create_note）        │
│   • 命令调用：POST /api/plugins/{id}/commands/{name}            │
│   • 受控能力：notes:read/write、host:ai、settings                │
│   • 安装/卸载：本地 zip / 远程 URI；fuel+内存限额；spawn_blocking │
└───────────────────────────────┬─────────────────────────────────┘
                                 │ 共享 jasper-core 类型 + JSON ABI
┌───────────────────────────────┴───────── 插件 (.wasm, Rust) ────┐
│  依赖 jasper-core，拿同一套 Note/Folder；默认零能力              │
│  导出：metadata() / on_before_save() / command_*() / ui_*()      │
└──────────────────────────────────────────────────────────────────┘
```

三层：**Manifest（声明式契约）** ⟶ **前端贡献宿主** ⟶ **后端 wasmi 宿主**。

## 4. 插件包格式 与 Manifest

### 4.1 打包格式（固定 zip）
一个插件 = 一个 zip，固定布局：
```
my-plugin.zip
├── manifest.toml        # 必需，根目录
├── plugin.wasm          # 可选（纯主题插件没有）
└── assets/
    ├── themes/nord.css
    └── icon.svg
```
- **本地加载**（当前唯一形态）：UI 上传 zip / 或丢进插件目录 → 解包到 `<config>/plugins/<id>/` → 解析 manifest → 加载。
- **远程 URI / 插件市场**：后置，暂不设计。zip 格式已为远程下载留好口子（将来加“下载→校验→解包”即可），现在不实现。

### 4.2 manifest.toml
后端解析后以 JSON 暴露给 SPA。
```toml
id = "ai-assistant"
name = "AI 助手"
version = "0.1.0"
description = "与笔记对话，整理更新笔记"

# ---- 后端 WASI 模块（可选）----
[backend]
wasm = "plugin.wasm"
capabilities = ["notes:read", "notes:write", "host:ai", "settings"]  # 默认全无，逐项申请
hooks = ["on-before-save"]

# ---- 前端贡献点（声明式；UI 由我方 widget 渲染）----
[[contributes.theme]]            # 用例 A：零代码主题
id = "nord"
name = "Nord 暗色"
css = "assets/themes/nord.css"

[[contributes.sidebar]]          # 用例 B：AI 侧边栏
id = "ai-chat"
title = "AI 对话"
icon = "💬"
widget = "chat"                  # 选用内置 widget（见 §7.2 词汇表），非自写 UI

[[contributes.command]]
id = "ai.summarize"
title = "总结本笔记"
target = "backend"               # backend(调 wasm 导出) | builtin

[[contributes.toolbar]]
command = "ai.summarize"
location = "note-toolbar"

[[contributes.editor]]           # 用例 C
on = "before-save"               # before-save(后端格式化) | input(后端处理 debounce 输入)
command = "fmt.run"

[settings.schema]                # 渲染成设置表单，值存 host(plugin 作用域)
api_key = { type = "secret", label = "API Key" }
model   = { type = "string", default = "claude-opus-4-8" }
auto_approve_write = { type = "bool", default = false, label = "AI 改笔记免确认" }
```

## 5. 三个用例怎么落地

### 5.1 用例 A — 主题（CSS + 图标，最先做，零运行时）

**A-1 颜色 / CSS 令牌化 — ✅ 已落地（2026-06-30）**
- `web/src/app.css` 改为**两层令牌**：基础调色板 `--palette-*` ← 语义令牌 `--bg/--text/--danger/--success/--overlay/--shadow-modal/--code-block-*` 等；散落各组件的写死色（`#c0392b`/`#2e7d32`/`rgba(0,0,0,.4)`/代码块色…）全部收编为 `var(--x)`。
- 主题切换由 `<html data-theme="light|dark">` 驱动（`index.html` 内联脚本防首屏闪烁 + `theme.svelte.ts` 运行时 rune）；auto 跟随系统。顶栏加了 Auto/Light/Dark 三态切换按钮（内联 SVG 图标）。源码编辑器(CodeMirror)明暗改为跟随选定主题而非系统。
- 待外接插件主题时：`contributes.theme` 的 `.css` 覆盖语义层令牌（`:root[data-theme="xxx"]{ --bg:…; }`）+ 宿主切 `data-theme` 即可。语义令牌清单 = 主题 API。

**A-2 图标可替换 — ✅ 已落地（2026-06-30，选定「图标即 CSS」）**
- **全部 emoji 清除**（模板里的 + 烤进 i18n 文案里的图标都拆掉，i18n 只剩纯文字）。新增图标层：
  - `web/src/icons.css`：每个图标一条 `--icon-<名>` 令牌（内联 SVG data URI，Lucide/Feather 线性风格；**整段 URL 编码**以过 lightningcss minify；由生成脚本产出，勿手改编码串）。
  - `Icon.svelte`：CSS `mask-image: var(--icon-<名>)` + `background: currentColor` → 单色、跟随文字色、主题可覆盖 `--icon-*`。
  - `Button.svelte`：统一按钮 = 图标 + 文字（variant: default/ghost/danger/primary；iconOnly 强制仅图标）。
  - `ui.svelte.ts`：全局按钮显示模式 `both|icon|text`（rune + localStorage），Button 据此决定显示图标/文字 → 后续加设置开关即可。
- 全站按钮已迁移到 Button/Icon：顶栏、笔记本树、笔记工具栏、编辑器、资源面板、设置页。
- **主题契约现含两块**：颜色语义令牌（A-1）+ 图标令牌 `--icon-*`（A-2）；插件主题一个 `.css` 即可同时换色 + 换图标。
- **风险**：低（纯前端、零运行时）。

**A-3 多主题 + 选择器 — ✅ 首个示例主题（2026-06-30）**
- 主题 = 一份自包含 `.css`：`:root[data-theme='<id>'] { 覆盖颜色令牌 + 图标令牌 }`。内置两套示例：`Nord`(深)、`Solarized Light`(浅)，**都演示换色 + 换图标**（Nord 把设置图标换成滑块、Solarized 把笔记本换成书）。
- `theme.svelte.ts` 扩成多主题：内置 `auto/light/dark` + 自定义主题表 `CUSTOM_THEMES`（id/名/base）；`data-theme` 直接用主题 id。
- 顶栏 `ThemePicker.svelte`（调色板图标下拉）列出全部主题，选中即时生效、localStorage 记忆、无首屏闪烁。
- 资产由 `web/scripts/gen-theme-assets.py` 生成（icons.css + themes/*.css，可复现）。
- **现状 vs 插件**：示例主题现在是**打包内置**；插件宿主就位后改成「下载 zip → 解包 → 注入这份 `.css`」即同样生效——**契约不变，只换交付方式**。即「装个 css 包换肤换图标」闭环的验证。

### 5.2 用例 B — AI 插件（贯穿三层）
- **前端**：`contributes.sidebar { widget = "chat" }` → 用内置「对话」widget（消息流+输入框+流式渲染），后端插件驱动，**无任何自写 UI**。
- **后端（wasmi/Rust）**：实现对话编排——`notes:read` 取上下文(RAG)、`host:ai` 发起补全、`notes:write` 整理回写。
- **密钥归 host**：插件只调 `ai_complete(messages)`，看不到 key/端点；host 持密钥、做 allowlist/限流；用户在 host 设置里配一次。端点设计成 OpenAI/Claude 兼容（示例默认 `claude-opus-4-8`）。
- **AI 改笔记**：产出 diff → `notes:write` → 复用 `serialize::update_note_md`（元数据逐字保留）→ 刷新索引。**默认弹 UI 二次确认**；插件设置 `auto_approve_write=true` 时跳过（用户自行选择默认通过）。

### 5.3 用例 C — 编辑插件
- **保存前格式化** → 后端钩子 `on-before-save(note) -> note`，注入 `api.rs::update_note`/`create_note` 在 `serialize::*` 之前。干净、可沙箱，**优先**。
- **回显边界**（2026-07-02 落地时确认）：改写只体现在 API 响应与磁盘上；`NoteView` 保存后**不回填**编辑器缓冲（自动保存频繁，回填跳光标）。切走再切回笔记即可见。若将来要「保存后立即回显」，做保守版：仅当响应正文 ≠ 提交值**且此后无新输入**时替换缓冲。
- **输入时检测** → 前端编辑器（CodeMirror 6 的两种模式：源码 / 实时预览）对 debounce 后的输入调一次后端 WASI 变换（`editor.on=input`），结果回填。**不放前端任意 JS**，仍走封装能力。完全实时的复杂交互暂不支持。

## 6. 后端 WASI 层（wasmi, Rust-only）

### 6.1 运行时：**wasmi**（已定）
纯 Rust 解释器，无 JIT、体积小，贴合"轻量+少堆包"。笔记处理调用不频繁，解释执行无感。不用组件模型。

### 6.2 ABI：共享类型 + **JSON**（已定）
- host 与插件都依赖 `jasper-core`，拿**完全相同**的 `Note`/`Folder`。
- ABI = 几个 `extern "C"` 导出函数（指针+长度收发字节）；编解码 **serde_json**（先求好调试，体积后续可换 bincode）。
- 前置：给 `core` 模型补 `Serialize/Deserialize`（若未加）。
- `plugin-sdk` crate 提供宏/胶水（`#[plugin_export]`），插件作者只写业务函数。

### 6.3 受控能力（默认零能力，manifest 逐项授权）
| 能力 | host 暴露 | 说明 |
|---|---|---|
| `notes:read` | `get_note`/`search`/`list_folders` | 只读 |
| `notes:write` | `upsert_note`/`create_note` | 默认 UI 确认（可设免确认） |
| `host:ai` | `ai_complete(messages)` | 密钥/端点在 host，插件不可见 |
| ~~`host:fetch`~~ | — | **暂不做**；AI 网络需求已由 `host:ai` 覆盖 |
| `settings` | `get_setting`/`set_setting` | 插件作用域 KV |

> 相对 Joplin JS 插件（裸跑 Node）的核心安全卖点：能力白名单 + 密钥不下放。

### 6.4 兜底
- 每次调用 **fuel/epoch 中断 + 内存上限**，防跑飞；调用走 `spawn_blocking`。
- 预编译 module + per-call 实例化（轻量）。

## 7. 前端贡献层（去静态化 UI，但不开放任意代码）

### 7.1 贡献宿主
SPA 启动拉 `/api/plugins` → 各 manifest(JSON) → 渲染贡献点到固定**插槽**（侧边栏/工具栏/设置/主题/编辑钩子）。现有三栏布局(`App.svelte`)预留插槽 → 加侧边栏改 manifest 即可，不动源码。

### 7.2 widget 词汇表（"提供 UI 封装，避免插件写 UI"）
我方提供一组**内置 widget**（Svelte 组件），插件**只声明用哪个 + 给数据**，不写 HTML/CSS/JS：

| widget | 用途 |
|---|---|
| `chat` | 消息流 + 输入框 + 流式（AI 面板） |
| `list` / `tree` | 列表/树 |
| `form` | 由 `settings.schema` 或后端返回的字段定义渲染表单 |
| `markdown` | 渲染一段 markdown/HTML（经 DOMPurify） |
| `button` / `toolbar` | 动作触发 |
| `checkbox` | 勾选 / 开关（也作 `form` 字段类型） |
| `select` | 下拉单选 / 多选 selection（也作 `form` 字段类型） |

两种驱动方式：
- **静态**：manifest 里 `widget = "chat"` 声明结构。
- **动态**：后端 wasm 导出 `ui_*()` 返回一棵 widget 声明树(JSON)，宿主用内置组件渲染（server-driven UI）。
- **代价**：插件能力受词汇表上限约束；按需扩词汇表。**换来的是**：零前端代码沙箱风险、UI 风格统一。

### 7.3 主题注入
注入 `<link>` 或写 `:root[data-theme]` 覆盖令牌（§5.1）。与现有 `render.ts`(DOMPurify、github-dark) 协调：主题改外壳与令牌，不破坏笔记 HTML 净化。

## 8. 安全与信任模型（简化后）
不开放任意前端代码，信任面只剩两类：

| 扩展类型 | 风险 | 处置 |
|---|---|---|
| 主题 CSS（声明式） | 低（少量外联探测） | 默认放行，可选净化 |
| 后端 wasmi（Rust） | 低（沙箱 + 能力白名单 + 密钥不下放） | 安装时展示申请的 capabilities，确认后放行 |
| widget 声明（前端） | 低（只走我方组件，无任意 JS） | 默认放行 |

远程安装额外注意：下载 zip 校验大小/校验和（签名后置）、限制来源。AI 写笔记默认 UI 确认。

## 9. 新增 API（阶段 2 定稿；commands/ui/ai 到阶段 3）
```
GET    /api/plugins                       已装列表：manifest 摘要 + contributes + capabilities
                                          + enabled + error（坏插件以 error 字段出现，不隐身）
POST   /api/plugins/install               裸 zip body（同资源上传惯例）；201 / 400 bad_manifest
                                          / 409 version_conflict（?force=1 覆盖）；含 backend → 落地 disabled
DELETE /api/plugins/{id}                  卸载；活动数据源引用中 → 409 in_use
POST   /api/plugins/{id}/enable           {enabled:bool}；enable=能力授权；wasm 实例化失败 422；同受 in_use 守护
GET/PUT /api/plugins/{id}/settings        GET 秘密不回显（secret_set 标记）；PUT 缺键=不变、空串=清除
GET    /api/plugins/{id}/assets/{path}    静态资产（主题 css 等）；仅 enabled；防路径逃逸；no-cache
POST   /api/plugins/{id}/commands/{cmd}   ✅ 已落地（0.2 提前实现；0.3 响应扩 pending_writes）
POST   /api/plugins/{id}/ui/{view}        ✅ 已落地（0.3；{state?} → {ui, pending_writes}）
PUT    /api/plugins/{id}/auto-approve     ✅ 已落地（0.3；notes:write 免确认开关，宿主托管）
GET/PUT /api/ai/config                    ✅ 已落地（0.3；宿主级 AI 配置。原设想的
                                          POST /api/ai/complete 取消——AI 只经插件 host_call 走，
                                          前端无直连补全需求）
```
写确认（0.3 定稿）：**提案回传**——notes.upsert/create 默认不落盘，提案随 command/ui 响应顶层
`pending_writes` 带回前端，diff 确认后走普通 PUT/POST /api/notes*（钩子照常）；免确认=宿主托管开关，
开启时直写但跳过 before-save 钩子（防重入）。细节见 spec §6.5/§7/§9.5/§12.2。
钩子（无新端点）：`on-before-save` 注入 `update_note`/`create_note`。
存储 provider（无新端点）：`PUT /api/config` 走 `source_type="plugin"` + `plugin_id`/`plugin_storage`/`plugin_config`。
只读模式：插件管理写操作被 `guard_read_only` 一并拦截（GET 列表/资产仍可用 → 只读下已装主题继续生效）。
feature off：路由不存在，但 SPA fallback 会对 `/api/plugins` 回 200 的 index.html——前端探测必须查 content-type。

## 10. 代码落点
- 新 crate `plugin-sdk/`（ABI 宏 + 重导出 `jasper-core` 类型），path 依赖，沿用"无 workspace"。
- 新模块 `server/src/plugins/`：`PluginHost`（wasmi 引擎、manifest、实例、能力代理、zip 安装器），**feature-gate `plugins`**；运行时**不进 `core`**。
- `AppState` 加 `plugins: Option<Arc<PluginHost>>`（feature 关 = `None`，零开销）。
- 钩子注入：`api.rs::update_note`/`create_note`；新增 plugins/market/ai 路由。
- 前端：`web/src/lib/` 加贡献宿主 + 内置 widget（chat/list/form/...）+ 主题注入；`App.svelte` 预留插槽；**先做 §5.1 令牌化重构**。
- 默认构建（不带 `--features plugins`）行为完全不变。

## 11. 分阶段路线
1. **阶段 1 — 主题**：CSS 令牌化重构（补全令牌 + `data-theme` + 文档化）+ 声明式主题加载。零运行时，先做。✅ 已落地（2026-06-30）。
2. **阶段 2 — 后端骨架 + 存储扩展点**（进行中，2026-07-02 扩容）：`plugin-sdk` + wasmi 加载器 + zip 安装(本地) + `on-before-save` 钩子 + 示例格式化插件；加上 `[[contributes.storage]]` + `host:http` + `PluginStorage` 适配 + WebDAV-as-plugin 参考实现；前端插件管理面板 + 向导动态数据源表单（SchemaForm）。跑通 = 后端路线 + 存储路线都成立。
3. **阶段 3 — 受控能力 + widget 宿主**：`host:ai`/`notes:*` + 内置 `chat`/`form` widget + server-driven UI + commands/ui 端点 → **AI 插件端到端**。UI 去静态化。✅ 已落地（2026-07-02，spec 升 0.3）：notes:read/write（写=提案回传+宿主托管免确认）、host:ai（genai 库，anthropic/openai 兼容 + 自定义 base_url）、六 widget 渲染器 + 右侧 dock 侧栏、ui/auto-approve/ai-config 端点。官方 ai-chat 插件（jasper-plugins 仓库）与 SDK 0.3 crates.io 发版为后续。
4. **阶段 4 — 编辑钩子 + 扩词汇表**：输入时检测 + 更多 widget。✅ 已落地（2026-07-04，apiVersion 不变仍 0.4——落地既有契约）：`contributes.editor` → `editor.transform`（纯文本 in/out、无 notes/ai 上下文，端点 `POST /api/plugins/{id}/editor/transform`）+ SDK `editor` 槽 + 前端**单个 CodeMirror 6 实例的源码与实时预览两模式**接 `input` 相位（debounce → 保守替换缓冲，仅真实用户输入触发、天然防环、陈旧则丢弃）；widget 词汇补齐 `checkbox`/`select`/`divider`/`heading`（UiWidget 渲染器）。`before-save` 相位 + `command` 复用后置（现有服务端 `on-before-save` 钩子已覆盖保存前改写）。细节见 spec §12.4。（远程 URI / 市场此前已在「插件生态 + 市场」阶段落地）

## 12. 决策记录
**已定**：wasmi / JSON ABI / Rust-only 后端 / 无任意 JS（走 widget 词汇表）/ AI 写笔记默认确认可关 / zip 打包**仅本地加载**（市场后置）/ `host:fetch` 暂不做 / feature-gate / **令牌两层（基础调色板 ← 语义层，已落地）**。

**widget 初始集合（已定）**：`chat`、`list`/`tree`、`form`、`markdown`、`button`/`toolbar`、`checkbox`、`select`(单/多选)。后续按需扩。阶段 4（2026-07-04）已把 `checkbox`/`select` 落进 UiWidget 渲染器，并补 `divider`/`heading` 两个布局原语。

**已落地**：A-1 颜色 / CSS 令牌化 + `data-theme`；A-2 图标系统（icons.css / Icon / Button / ui，**全站去 emoji**，图标即 CSS mask）；A-3 多主题 + ThemePicker + 两套示例主题（Nord / Solarized，换色 + 换图标）（2026-06-30）。

**0.2 追加已定**（2026-07-02）：存储 provider 扩展点（含 host:http、SMB 非目标、WebDAV-as-plugin、限额两档、安装默认 disabled、in_use 守护、管理面板=顶栏、provider 走 GET /api/plugins）——细节见文首「新增决策」与 spec §12.1。

**0.3 追加已定**（2026-07-02）：写确认=提案回传、免确认宿主托管、ai.complete 走 genai（宿主级 provider/base_url/key/model）、notes/ai 仅 command/ui 上下文、sidebar 扩 command/view、widget 事件契约冻结——细节见 spec §12.2。

**0.4 阶段 4 追加已定**（2026-07-04，apiVersion 不变）：`contributes.editor`/`editor.transform` 编辑期钩子落地（纯文本 in/out、无 notes/ai 上下文、前端接 input 相位并保守替换缓冲——0.4 落地时仅源码模式，编辑器统一到 CodeMirror 6 后两模式（源码/实时预览）都接）；SDK 加 `editor` 槽；widget 词汇补齐 checkbox/select/divider/heading。均为「落地既有契约 + 前端渲染层扩充」，向后兼容不升 apiVersion——细节见 spec §12.4。

**仍待定（不阻塞，到对应阶段再定）**：
- [ ] 按钮“显示图标 / 文字 / 两者”是否在设置页给用户开关（store `ui.svelte.ts` 已就绪，差一个 UI）。
- [x] server-driven UI 声明树的 **schema 细节**——已定（0.3）：`{type, props, children}` 一层树，契约在 spec §9.2/§9.3。
```
