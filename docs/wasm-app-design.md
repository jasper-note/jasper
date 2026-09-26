# jasper WASM 应用化设计方案

> 状态：**P0–P3 + P4(Chromium) 已落地**——无后端「可写本地应用」构建 `pnpm build:local`：笔记/笔记本/标签的
> 增删改移 + 资源图片，经 WASM 内核读写。持久后端**运行时二选一**：① IndexedDB（默认，刷新不丢；2026-07-05
> feat/wasm-write）；② **真实 Joplin 文件夹**（File System Access，仅 Chromium；顶栏「打开文件夹」，读写直落磁盘、
> Joplin 可同步；2026-07-05 feat/wasm-fsa）。Firefox/Safari 无目录选择器，降级（webkitdirectory 导入 / zip 导出）仍待做（见 §9）。
> 目标读者：项目作者本人。
> 相关：[joplin-data-format.md](./joplin-data-format.md)（写回格式依据）、`wasm/src/lib.rs`（WASM 内核：读+写）、
> `web/src/lib/api.ts`（三态 api + 可插拔本地后端 idb/fsa）、`web/src/lib/localStore.ts`（IndexedDB）、
> `web/src/lib/fsStore.ts`（FSA 真实文件夹）。

## 0. 一句话

`jasper-core`（parser + serialize + 内存索引 `Library` 的**全部**增删改）**无 IO、已编到 wasm**——所以 `Library` 在内存里能做的事，浏览器里都能做。现在的 WASM demo 只读，**不是因为能力缺失，而是缺一个写入落点（浏览器无磁盘）＋ 前端 `readOnly` 门闸把写入入口全封了**。本方案给浏览器构建补上 **IndexedDB 持久化**，让纯前端 demo 升级成「浏览器里能跑的真正离线笔记应用」。

## 1. 背景与核心洞察

- server 相对 core 只多做四件事：**IO**（`std::fs` / WebDAV `ureq`）、**持久化**（SQLite config/cache）、**插件**（wasmi 沙箱）、**AI/鉴权/SSE**。业务逻辑一行都不在 server——全在 `jasper-core`。
- `core::library::Library` 已经提供全套写原语（`server/src/api.rs` 的 handler 只是它们的薄壳）：
  - `upsert_note` / `upsert_folder` / `upsert_resource` / `upsert_tag` / `upsert_note_tag`
  - `remove_note` / `remove_note_tag` / `remove_resource`
  - `is_self_or_descendant`（移动防环）、`resource_usage` / `notes_referencing_resource`（孤儿/引用）
- `core::serialize` 提供全套 `.md` 生成（`new_note_md` / `update_note_md` / `move_note_md` / `new_folder_md` / `rename_folder_md` / `move_folder_md` / `new_tag_md` / `new_note_tag_md` / `new_resource_md` / `update_resource_md`）。**这些函数都把 `now: i64` 作参数**（不内部取时钟），天然适配浏览器注入时间。
- 写路径的标准套路（照抄 `api.rs`）：`new_id()` → `serialize::xxx_md(..., now)` 生成原始 `.md` → `library.upsert_xxx(&content)` 同步内存 → 服务端再落盘。浏览器版把「落盘」换成「写 IndexedDB」即可。

## 2. 目标与非目标

**目标**
- 浏览器纯前端构建（无 server）支持笔记/笔记本/标签的**增删改移**，**刷新后仍在**（IndexedDB 持久）。
- 资源图片：上传、显示、管理（blob 存 IndexedDB）。
- 与现有只读展示站（GitHub Pages）**共存**：两种构建各自独立，互不回退。

**非目标（保持 server-only）**
- WebDAV / 本地文件夹数据源（浏览器 CORS/FS 限制）——**File System Access API 单列为远期可选**（§9）。
- 插件系统（wasmi-in-wasm 不现实）、AI（genai/reqwest）、鉴权/访问控制、SSE。这些在浏览器构建里保持关闭（`events.ts`、`plugins.svelte.ts` 已按 backendless 分支处理，见 §6）。
- 与真实 Joplin 库的同步/冲突协议（本项目一贯不参与）。

## 3. 功能可行性分类

| 分类 | 功能 | 结论 |
|---|---|---|
| **A 已可用**（纯前端，零改动） | Markdown 渲染全套（高亮/表/KaTeX/任务清单/HTML 净化）、i18n、任务进度、两套编辑器（现被 `readOnly` 隐藏） | ✅ 开写入即点亮 |
| **B 本方案要做**（core 已有逻辑，IO-free） | 标签视图（读）、笔记编辑/新建/删除/移动、笔记本增改移（防环）、打/去标签、资源元数据/孤儿管理、资源二进制（blob） | ✅ WASM 薄封装 + IndexedDB |
| **C 保持 server-only** | WebDAV/本地文件夹数据源、SQLite、插件、AI、鉴权、SSE | ❌ 浏览器构建关闭 |

## 4. 架构：五个关键改动

### 4.1 WASM API 面扩展（`Demo` → 可写 `App`）

`wasm/src/lib.rs` 的 `Demo` 结构方法从 `&self` 扩为 `&mut self`，新增写方法。每个写方法**返回一个 JSON**，同时携带：① 供 UI 用的结果（note detail / folder ref 等）；② 供前端持久化的**变更增量**（改了/删了哪些原始条目）。签名里**显式接收 `now: f64`**（JS 侧传 `Date.now()`），内部转 `i64`——**绝不调用 `serialize::now_ms()`**（见 §4.2）。

拟新增方法（镜像 `api.rs` handler）：

```
// 写：返回 { detail?, persist: { upserts:[{id,type_,raw}], deletes:[id] } }
create_note(parent_id, title, body, is_todo, now) -> String
update_note(id, title, body, now)                 -> String
move_note(id, new_parent, now)                     -> String
delete_note(id)                                    -> String
create_folder(parent_id, title, now)              -> String
rename_folder(id, title, now)                      -> String
move_folder(id, new_parent, now)                  -> String   // 内部 is_self_or_descendant 防环
add_note_tag(note_id, title, now)                 -> String   // trim + 不区分大小写复用/新建
remove_note_tag(note_id, tag_id)                  -> String
upsert_resource_meta(id, title, mime, ext, size, now) -> String  // 二进制走 JS/IndexedDB，元数据走这里
// 读（补齐）：
tags()          -> String   // tags_sorted + tag_note_count
notes_by_tag(id)-> String
note_tags(id)   -> String
resources()     -> String   // resource_usage 求 used_by
```

> **变更增量 vs 全量快照**：MVP 阶段用**全量快照**最省心——每次写完让 WASM 暴露 `snapshot() -> Vec<raw>`，前端 debounce 后整体写回 IndexedDB（个人库数百条、字符串小，成本可忽略）。后续若嫌 O(n) 再切增量 `persist` 字段。**先快照，够用再优化**。

### 4.2 时刻/ID 注入（`now_ms` 落坑，务必记住）

- `serialize::now_ms()` 用 `std::time::SystemTime::now()`，在 **`wasm32-unknown-unknown` 上会 panic**（`time not implemented on this platform`）。现有只读 demo 从不触发；一旦写入就会踩。
- **对策**：所有写方法把 `now` 当参数从 JS 传入（`Date.now()`）。serialize 的 `*_md` 函数本就接收 `now`，天然契合。**规约：WASM 层任何路径都不得调用 `serialize::now_ms()`。**
- `new_id()` 用 `getrandom`，`wasm/Cargo.toml` 已开 `features=["js"]` 走浏览器 crypto，**可用**。
- `serialize::format_iso` 用 `chrono::from_timestamp_millis`（不取时钟），**可用**。

### 4.3 持久化 schema（IndexedDB）与迁移

浏览器侧新增一个极薄的存储层（前端 TS，非 WASM）：`web/src/lib/localStore.ts`。

- **DB**：`jasper-local`，`meta.schema_version` 记版本。
- **object stores**：
  - `items`（key=`id`）：`{ id, type_, raw }`——每条即一份原始 `.md` 文本（笔记/笔记本/标签/note_tag/资源元数据，全平铺，对齐 Joplin「一条目一文件」）。
  - `resources`（key=`id`）：`{ id, mime, blob }`——资源二进制（`Blob`）。
  - `meta`（key=固定串）：`{ schema_version, seeded }`。
- **启动**：读 `items` 全部 `raw` → `Demo::from_contents(raws)` 建库；`resources` 懒加载成 blob URL（§4.4）。
- **写**：WASM 写方法返回后，前端把变更（快照或增量）落 `items`；资源上传把 `Blob` 落 `resources`、元数据 `.md` 落 `items`。
- **首次运行播种（seed）**：空库时用内置演示库 `demo::items()` 播种（应用不空、可直接玩），并置 `meta.seeded=true`；用户可自行删空。是否播种做成开关，见 §8。
- **迁移**：`schema_version` 单调递增；升级时按版本跑迁移函数。原始 `.md` 是 Joplin 格式、向前稳定，迁移风险低。

### 4.4 资源 blob 路径（唯一非自明的 UI 改动）

- `:/id` → 显示 URL 的改写**一点集约**在 `api.resourceUrl(id)`（`render.ts:71`（阅读视图）、`editor/livePreview.ts` + `editor/widgets.ts`（实时预览）、`ResourcePanel.svelte`）。
- 浏览器构建里 `resourceUrl(id)` 改为**返回 blob URL**。因 `resourceUrl` 目前是**同步**签名，方案：启动/上传时把 `resources` store 的 `Blob` 用 `URL.createObjectURL` 建成 `Map<id, objectURL>`，`resourceUrl` 同步查表；卸载/删除时 `revokeObjectURL`。
- 上传 `uploadResource(file, filename)`：前端算 id（`new_id`）、存 `Blob` 进 `resources`、调 WASM `upsert_resource_meta` 生成元数据 `.md`、更新 blob URL 表，返回与 server 版同形的 `{ id, markdown, ... }`。

### 4.5 `readOnly` 门闸分离（区分「只读展示」与「可写本地应用」）

现状：`App.svelte:114` `const readOnly = $derived(IS_DEMO || serverReadOnly || locked)`——`IS_DEMO` 一刀切只读。

引入**两个正交的构建标志**（`api.ts`）：
- `IS_WASM`：无 server、走 jasper-core wasm 的**无后端构建**（展示站与本地应用**都**属此）。`events.ts`/`plugins.svelte.ts` 的现有 `IS_DEMO` 分支改挂 `IS_WASM`（它们要的是「没有后端」，与可写性无关）。
- `WASM_WRITABLE`：该无后端构建是否**可写＋持久**（本地应用为真，展示站为假）。

于是：
```
readOnly = serverReadOnly || locked || (IS_WASM && !WASM_WRITABLE)
IS_DEMO（只读展示横幅）≡ IS_WASM && !WASM_WRITABLE
```
构建入口：
- `VITE_DEMO=1` → 只读展示站（现状不变，Pages 继续部署这套）。
- `VITE_LOCAL=1` → 可写本地应用（`IS_WASM && WASM_WRITABLE`），新加 `build:local` 脚本。

`web/src/lib/api.ts` 的组装从「二选一」扩为「三态」：
```
export const api = WASM_WRITABLE ? localApi : IS_WASM ? demoApi : httpApi
```
`localApi` 必须覆盖**全部可写方法面**（updateNote/createNote/deleteNote/moveNote/createFolder/renameFolder/moveFolder/noteTags/addNoteTag/removeNoteTag/uploadResource/resources/renameResource/deleteResource/resourceUrl/tags/notesByTag），否则未实现的方法会落到 `httpApi` 去 `fetch('/api/...')` 失败。demoApi 维持只读子集（现状）。

## 5. 分阶段路线图

- **P0 · 标签视图（读）**——`demo::items()` 补标签/关联条目，WASM 加 `tags/notes_by_tag/note_tags` 只读方法，demoApi 接上（现在返回 `[]`）。低风险、独立可交付，展示站也受益。
- **P1 · 可写内核（内存态）+ 门闸分离**——WASM 写方法 + `Date.now()` 注入 + `IS_WASM`/`WASM_WRITABLE` 拆分 + `localApi`。此时写入生效但刷新即丢（无持久）。先把「写入链路 + UI 点亮」跑通。
- **P2 · IndexedDB 持久**——`localStore.ts` + 启动恢复 + debounce 快照写回 + 首次播种。至此＝真正的离线笔记应用。
- **P3 · 资源图片**——`resources` store + blob URL 表 + `resourceUrl` 改写 + 上传链路。
- **P4 ·（可选/远期）** File System Access API 读写真实文件夹（§9）、导出/导入（`.jex`/zip 备份）。

## 6. 复用与既有分支

- `events.ts:17` 已 `if (IS_DEMO || ...) return false`——SSE 在无后端构建天然关闭；仅把判据由 `IS_DEMO` 改 `IS_WASM`。
- `plugins.svelte.ts:106` 已 `if (IS_DEMO) { ... }`——插件在无后端构建关闭；同样改挂 `IS_WASM`。
- `App.svelte:41` `if (!IS_DEMO) api.putLocale(...)`——本地应用无 server locale 端点，判据改 `IS_WASM`（不 PUT）。
- 写入 UI 全部已受 `readOnly` 驱动并层层下传（NoteView/NoteList/FolderTree/NoteTags/ResourcePanel），门闸一改即自动放开，**无需逐组件动**。

## 7. 测试方针

- **core**：写原语与 serialize 已有单测覆盖（`cd core && cargo test`），本方案不新增 core 逻辑，主要是复用。
- **WASM 层**：保持薄；逻辑正确性靠 core 单测背书。可加 1~2 个 `wasm-bindgen-test` 冒烟（建库→写→读回）。
- **前端**：
  - `localStore.test.ts`（Vitest + `fake-indexeddb`）：条目/资源往返、启动恢复、播种、schema 迁移。
  - `localApi` 适配层单测：mock wasm 模块，断言写方法把变更正确交给 `localStore`、`resourceUrl` 返 blob URL。
  - 门闸：`readOnly` 三态推导单测。
- **e2e（可选）**：对 `build:local` 产物跑一条 Playwright（新建→编辑→刷新→仍在），复用现有 e2e 骨架但**不起 Rust 后端**（纯静态托管）。

## 8. 未决问题 / 待定

- **首次播种**：默认用演示库播种，还是空库 + 一键「载入示例」？（倾向：默认播种，横幅提示「本地数据存在你的浏览器里」。）
- **持久策略**：P2 用全量快照（简单）先落地，是否值得后续做增量？（个人规模下快照够用。）
- **展示站是否也切到可写本地应用**：两者可并存不同 URL；或让 Pages 同时出「只读展示」与「本地应用」两个入口。
- **导出/备份**：浏览器数据易被清。是否 P3/P4 就提供「导出为 zip/`.jex`」以防丢数据。
- **多标签页并发**：同一 IndexedDB 被多标签页写。是否需要 `BroadcastChannel` 或简单「后写覆盖 + 载入时提示」。（个人单标签使用低风险，先不做。）

## 9. P4：File System Access API（真实文件夹）—— Chromium 已落地

`showDirectoryPicker()`（Chromium 系）让浏览器构建**直接读写真实 Joplin 同步文件夹**，把「无后端构建」从「浏览器内私有库」升级成「无 server 的完整本地客户端」。

**落点（`web/src/lib/fsStore.ts` + `api.ts` 可插拔后端）**：
- **数据源 = 后端二选一，运行时切换**：`LocalBackend` 抽象（`loadRaws`/`persist`/资源读写）；`idbBackend`（IndexedDB，默认）与 `fsaBackend(handle)`（真实文件夹）。切换即重建 wasm 实例 + 资源 blob URL 映射。
- **读**：`readAllItems` 遍历根目录、过滤 `^[0-9a-fA-F]{32}\.md$`（对齐 `server::storage::is_item_filename`）→ raws → `Demo.fromRaws`；`.resource/<id>` 二进制懒读成 blob URL。
- **写 = per-file 增量**：`persist` diff 快照（JS 侧 `idOfRaw` 解析 id，`prevSnapshot` 比对）→ 只 `writeItem` 改动的 `<id>.md`、`deleteItemFile` 已移除的；不整目录重写。写回同 `serialize.rs` 格式，Joplin 下次同步拾取。`createWritable().close()` 提交（原子性够用）。
- **句柄持久**：`FileSystemDirectoryHandle` 可 structuredClone → 存 IndexedDB（`jasper-fs`）；回访 `queryPermission`，权限仍在则自动重连，过期则 UI 提示「重新连接」（`requestPermission` 需用户手势）。
- **门闸/tree-shake**：控制挂 `export const localFolder = WASM_WRITABLE ? {...} : undefined`——native 构建 `WASM_WRITABLE` 折叠常量 false → `localFolder=undefined`、fsStore/backend/wasm 一并 tree-shake（已验证原生 dist 无 wasm 资产）。
- **UI**：`App.svelte` 顶栏（仅 `localFolder` 存在时）——Chromium 显示「打开 Joplin 文件夹」→ 打开后 chip 显示文件夹名 + 「关闭文件夹」回浏览器存储；Firefox（`!supported()`）显示不支持提示。

**测试**：`fsStore.test.ts`（内存版 fake `FileSystemDirectoryHandle` 覆盖读/写/删/资源/文件名过滤）；端到端由 Playwright 注入 fake `showDirectoryPicker`（类实现 → structuredClone 不抛）驱动真实应用：打开文件夹 → 读现有 → 新建笔记 → 断言真实文件夹 per-file 写回新 `<id>.md`。

**约束/未决**：仅 Chromium；权限重启需重授（已处理为「重新连接」）；打开中 Joplin 外部改文件不自动刷新（无 mtime 轮询，MVP 略）；`.resource` 二进制启动全量预载成 blob URL（大库可后续改懒加载）。

## 9.1 Firefox / Safari 降级（待做）

无 `showDirectoryPicker`（Mozilla 将本地磁盘选择器列为 harmful；OPFS 是源私有沙箱，Joplin 看不到 → 无用）。降级路径：`<input webkitdirectory>` 一次性**导入**真实文件夹 → IndexedDB 本地库；改动经 **zip 导出**下载、手动放回 Joplin 目录。非实时，但可 round-trip。列为后续。
