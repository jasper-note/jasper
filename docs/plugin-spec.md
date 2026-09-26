# jasper 插件规范 v0.4（草案）

> 状态：**草案，征求确认**。这是给「插件作者」和「宿主实现」照着做的**契约**；方案/路线见 [plugin-design.md](./plugin-design.md)。
> 规范用词遵循 RFC 2119：**MUST/必须**、**SHOULD/应当**、**MAY/可以**。
> 本版 `apiVersion = "0.4"`。破坏性改动升 MAJOR，向后兼容的新增升 MINOR。

**版本历史**

| 版本 | 日期 | 变化 |
|---|---|---|
| 0.1 | 2026-06-30（冻结） | 基础契约：主题 / 后端 wasm / 前端贡献 / 设置 |
| 0.2 | 2026-07-02 | 向后兼容新增：`[[contributes.storage]]` 存储 provider（§3.9）、`host:http` 能力（§7）、`storage.*` 方法族（§6.5）、§10 增 `required`/`placeholder`、存储类调用限额修正（§11）、SMB/裸 TCP 非目标声明（§11）、v0.2 决策（§12.1） |
| 0.3 | 2026-07-02 | 向后兼容新增：`notes:*`/`host:ai` 能力落地（§6.5/§7）、写确认=**提案回传**（§6.5/§7/§11）、`[[contributes.sidebar]]` 扩 `command`/`view`（§3.5）、widget 事件契约钉死（§9.2）、UiNode 交互约定（§9.3）、宿主端点 ui / ai-config / auto-approve（§9.5 新）、图标令牌 +3（§9.1）、§10 删除与宿主托管开关矛盾的示例、v0.3 决策（§12.2） |
| 0.4 | 2026-07-03 | 向后兼容新增：`[[contributes.locale]]` 语言包贡献（§3.10）——插件给应用**增加一门界面语言**（catalog JSON = message key → 译文，缺失回落 base；零代码即可，同主题信任档）；`system.locale` 免能力 host 方法（§6.5）——插件读**当前 UI 语言**以本地化自己运行时的产出（宿主持久化 UI 语言）；widget/命令 result 的文本字段可为 **locale map**（§9.2.1）——插件**返回多语言 UI**、前端按当前语言挑、v0.4 决策（§12.3） |
| 0.4（阶段 4） | 2026-07-04 | **落地既有契约**（apiVersion 不变）：`contributes.editor` → `editor.transform` 编辑期钩子（§3.7/§6.5/§8，端点 §9.5）——纯文本 in/out，前端接 `input` 相位、保守替换缓冲（0.4 落地时仅源码模式；编辑器统一到 CodeMirror 6 后源码/实时预览两模式都接）；widget 词汇扩充 `checkbox`/`select`/`divider`/`heading`（§9.2）；SDK `editor` 槽；v0.4 阶段 4 落地（§12.4） |

---

## 0. 范围与术语

- **宿主（host）**：jasper 本体（server + web）。
- **插件（plugin）**：一个 zip 包，向宿主贡献以下一种或多种：**主题**、**后端逻辑（wasm）**、**前端贡献**、**设置**。
- **后端模块**：插件里的 `*.wasm`，用 Rust 编译，跑在宿主的 **wasmi** 沙箱里。
- **能力（capability）**：插件访问宿主资源的授权项，**默认全部拒绝**。
- 时间戳一律 **Unix 毫秒**（i64）。字段命名一律 **snake_case**（与现有 HTTP API、jasper-core 一致）。

---

## 1. 插件类型

一个插件由 `manifest.toml` 声明它贡献什么，不是互斥的“类型”：

| 贡献 | 需要 wasm | 典型例子 |
|---|---|---|
| **主题** `[[contributes.theme]]` | 否（纯 CSS） | 换肤、换图标 |
| **语言包** `[[contributes.locale]]`（0.4） | 否（纯 JSON） | 法语/日语等界面翻译 |
| **后端钩子/命令** `[backend]` | 是 | 保存前格式化、AI 编排 |
| **前端贡献** `[[contributes.*]]` | 看情况 | 侧边栏面板、工具栏按钮 |
| **设置** `[settings.schema]` | 否 | 插件配置项 |
| **存储 provider** `[[contributes.storage]]`（0.2） | 是 | OneDrive/Dropbox 等云盘、WebDAV 变体 |

- **纯主题 / 纯语言包插件 MUST NOT 含 wasm**，宿主按“零代码”信任档加载（见 §11）。
- 含 `[backend]` 的插件 MUST 提供合法 wasm 且声明所需 capabilities。

---

## 2. 包格式与目录布局

- 一个插件是一个 **zip**（建议扩展名 `.jplug`，宿主 MUST 同时接受 `.zip`）。
- 解包后根目录布局：

```
manifest.toml          # 必须，根目录
plugin.wasm            # 含 [backend] 时必须；路径由 [backend].wasm 指定
assets/                # 可选：css / svg / 图片等
  themes/<id>.css
```

- 宿主安装时 MUST 把包解到 `<config>/plugins/<id>/`（`<id>` = manifest 的 `id`）。
- 路径字段（`wasm`、`css` 等）MUST 为相对包根的正斜杠路径，MUST NOT 逃出包根（禁止 `..`、绝对路径）。
- 当前仅支持**本地加载**（放目录 / 上传 zip）；远程 URI / 市场后置（见 design §4）。

---

## 3. manifest.toml 规范

宿主本版**仅接受 `manifest.toml`**（不接受 JSON）。未知字段 MUST 被忽略（向前兼容）。

### 3.1 顶层字段

| 字段 | 类型 | 必须 | 约束/说明 |
|---|---|---|---|
| `id` | string | ✅ | `^[a-z0-9][a-z0-9-]*$`，全局唯一，即安装目录名 |
| `name` | string | ✅ | 显示名 |
| `version` | string | ✅ | semver（如 `1.2.0`） |
| `apiVersion` | string | ✅ | 插件面向的插件 API 版本，`MAJOR.MINOR`（本版 `0.3`） |
| `description` | string | ⬜ | 一句话简介 |
| `author` | string | ⬜ | |
| `minHostVersion` | string | ⬜ | 要求的最低宿主版本（semver） |

### 3.2 `[backend]`（可选）

| 字段 | 类型 | 默认 | 说明 |
|---|---|---|---|
| `wasm` | string | — | wasm 路径；本表存在则**必须** |
| `capabilities` | array<string> | `[]` | 申请的能力（§7），未申请即拒绝 |
| `hooks` | array<string> | `[]` | 订阅的钩子（§8），值取自钩子名 |

### 3.3 `[[contributes.theme]]`（数组，可多个）

| 字段 | 类型 | 必须 | 说明 |
|---|---|---|---|
| `id` | string | ✅ | 主题 id（`data-theme` 的值），同插件内唯一 |
| `name` | string | ✅ | 显示名 |
| `base` | `"light"`\|`"dark"` | ✅ | 决定未覆盖令牌的回退基调 + 编辑器明暗 |
| `css` | string | ✅ | 主题 CSS 路径（相对包根） |

### 3.4 `[[contributes.command]]`

| 字段 | 类型 | 必须 | 说明 |
|---|---|---|---|
| `id` | string | ✅ | 命令 id（插件内唯一） |
| `title` | string | ✅ | 显示名 |
| `target` | `"backend"`\|`"builtin"` | ✅ | backend=调 wasm `command` 方法；builtin=宿主内置动作 |
| `icon` | string | ⬜ | 图标令牌名（§9.1），不带 `--icon-` 前缀 |

### 3.5 `[[contributes.sidebar]]`（0.3 扩展）

| 字段 | 类型 | 必须 | 说明 |
|---|---|---|---|
| `id` | string | ✅ | 面板 id（插件内唯一，`^[a-z0-9][a-z0-9-]*$`） |
| `title` | string | ✅ | |
| `icon` | string | ⬜ | 图标令牌名（§9.1），缺省宿主用 `plug` |
| `widget` | string | ✅ | 用哪个内置 widget（§9.2）；`view` 存在时仅作声明参考，实际树由 `ui` 方法给出 |
| `command` | string | ⬜ | **静态模式**交互目标：指向某 `contributes.command.id`（须 `target="backend"`）。`widget="chat"` 且无 `view` 时**必须** |
| `view` | string | ⬜ | **动态模式**：宿主经 `POST /api/plugins/{id}/ui/{view}`（§9.5）调 dispatch `ui`（§6.5）取声明树渲染 |

约束：`command` 或 `view` 任一存在 ⇒ 插件 MUST 含 `[backend]`；`command` MUST 引用已声明的 backend 命令。

```toml
# 静态模式：AI 聊天（消息由前端持有，发送经 command 调后端）
[[contributes.sidebar]]
id = "ai-chat"
title = "AI 对话"
icon = "chat"
widget = "chat"
command = "chat"

# 动态模式：面板树由 wasm 的 ui 方法按 view 返回
[[contributes.sidebar]]
id = "tools"
title = "工具箱"
widget = "markdown"
view = "main"
```

### 3.6 `[[contributes.toolbar]]`

| 字段 | 类型 | 必须 | 说明 |
|---|---|---|---|
| `command` | string | ✅ | 指向某 `contributes.command.id` |
| `location` | `"note-toolbar"`\|`"topbar"` | ✅ | 放置位置 |

### 3.7 `[[contributes.editor]]`（0.4 阶段 4 落地）

编辑期把编辑缓冲的文本交给 wasm 的 `editor.transform`（§6.5）改写。含此表的插件 MUST 含 `[backend]`；
同一 `on` 相位 MUST 至多声明一次。

| 字段 | 类型 | 必须 | 说明 |
|---|---|---|---|
| `on` | `"before-save"`\|`"input"` | ✅ | before-save=保存前；input=debounce 输入时（实时校验/联想） |
| `command` | string | ⬜ | 复用某命令；缺省调 `editor.transform`（§8）。**宿主本版仅实现 `editor.transform`**——`command` 复用后置，声明了也被忽略（向前兼容） |

- 宿主实现约定：经 `POST /api/plugins/{id}/editor/transform`（§9.5）调 `editor.transform`；**纯文本 in/out，不带 notes/ai 上下文**（`notes.*` 得 `unsupported`，防写入重入）。
- 前端接入约定（jasper）：单个 CodeMirror 6 实例的**源码与实时预览两模式**都接 `input` 相位——用户输入停顿（debounce）后依次调各声明插件、保守替换缓冲（等待期间用户又敲字则丢弃变换）。`before-save` 相位保留在契约里，前端接入后置（现有服务端 `hook.before_save`（§8）已覆盖保存前改写场景）。

### 3.8 `[settings.schema]`

每个键是一个设置项，值是字段定义（见 §10）。

### 3.9 `[[contributes.storage]]`（数组，可多个；0.2 新增）

插件贡献一种**存储 provider**（数据源后端，如云盘 API）。含此表的插件 MUST 含 `[backend]`（`storage.*` 方法由 wasm 实现，见 §6.5）。

| 字段 | 类型 | 必须 | 说明 |
|---|---|---|---|
| `id` | string | ✅ | provider id（插件内唯一），进 `source_key` 缓存隔离键 |
| `name` | string | ✅ | 显示名（设置向导里的数据源选项） |
| `icon` | string | ⬜ | 图标令牌名（§9.1），缺省宿主用 `plug` |
| `config_schema` | table | ✅ | **每数据源**配置表单，字段词汇同 §10（如 url/user/pass(secret)） |

```toml
[[contributes.storage]]
id = "webdav"
name = "WebDAV (插件)"

[contributes.storage.config_schema]
url  = { type = "string", label = "WebDAV URL", required = true, placeholder = "https://…" }
user = { type = "string", label = "用户名" }
pass = { type = "secret", label = "密码" }
```

- `config_schema` 描述「配置**一个数据源**要填什么」（per-SOURCE，同一插件可配出多个数据源），与 `[settings.schema]`（per-plugin KV）无关。
- 宿主把该数据源的 config 在**每次** `storage.*` 调用里传给插件（§6.5）；插件 MUST NOT 自行持久化凭据。
- **秘密回显不对称**（宿主实现约定）：数据源配置（含 `secret` 字段）随宿主 `GET /api/config` 回显以支持设置页预填——与内置 webdav 密码同姿势；而 `[settings.schema]` 的 secret 值**不回显**（§10）。

### 3.10 `[[contributes.locale]]`（数组，可多个；0.4 新增）

插件给应用**增加一门界面语言**（如法语、日语）。与主题贡献（§3.3）同构：**零代码即可**（无需 `[backend]`），
宿主按“零代码”信任档加载并自动启用。

| 字段 | 类型 | 必须 | 说明 |
|---|---|---|---|
| `code` | string | ✅ | 语言代码，BCP-47 子集（字母开头，其后字母/数字/`-`），如 `fr`、`ja`、`pt-BR`。同插件内唯一；作为语言切换器里存/选的值。**不应**与宿主内置代码（`en`/`zh`）相同——若相同，宿主 MUST 以内置为准、忽略该项（防劫持内置语言） |
| `name` | string | ✅ | 该语言的**自称显示名**（endonym，如 `Français`）；语言切换器直接展示，不再翻译 |
| `base` | `"en"`\|`"zh"` | ⬜ | 缺失键的回落语言（宿主内置之一），缺省 `en` |
| `messages` | string | ✅ | catalog JSON 路径（相对包根，规则同 §2） |

**catalog（`messages` 指向的 JSON）**
- 一个**扁平对象**：`{ "<msgKey>": "<译文>", … }`。key 取宿主的界面 message key 集合，value 为该语言译文。
- 宿主的 message key 集合是**宿主内部契约**（非本规范冻结）：作者以宿主内置 `en` 目录为**权威 key 清单**照译；宿主 SHOULD 提供其 `en` 目录供作者取用。
- **缺失/未知 key 宽容**：catalog 未覆盖的 key MUST 回落到 `base`（再回落宿主内置默认）；catalog 里宿主不认识的 key MUST 被忽略（向前兼容，宿主升级不删旧 key 时旧语言包继续可用）。
- value 为纯文本，`{name}` 形式的插值占位符 SHOULD 与宿主原文保持一致。

```toml
# manifest.toml —— 纯语言包（零代码，自动启用）
id = "lang-fr"
name = "Français"
version = "1.0.0"
apiVersion = "0.4"

[[contributes.locale]]
code = "fr"
name = "Français"
messages = "assets/locales/fr.json"
```
```json
// assets/locales/fr.json —— 只需覆盖想翻的 key，其余自动回落 base(en)
{ "common.save": "Enregistrer", "topbar.search": "Rechercher des notes…" }
```

- 宿主实现约定：catalog 经资产端点（§12.1-5 的 `GET /api/plugins/{id}/assets/{path}`）托管，前端 fetch 后注册进 i18n；启用/停用/卸载语言包后语言切换器随之增减，当前所选语言的来源插件消失时 MUST 回落到某内置语言。

---

## 4. 版本与兼容

- 宿主 MUST 暴露自身支持的 `apiVersion` 集合（如 `["0.1", "0.2", "0.3", "0.4"]`）。
- 加载时：若 `manifest.apiVersion` 的 MAJOR 不在宿主支持集，MUST 拒绝并提示。MINOR 更高可加载但宿主 MAY 警告。
- `minHostVersion` 大于当前宿主版本时 MUST 拒绝。
- 同 `id` 重复安装：以更高 `version` 覆盖；相等或更低 MUST 询问/拒绝。

---

## 5. 生命周期

```
发现(扫描 plugins 目录 / 上传)
  → 读 manifest → 校验(§3/§4) → 失败则标记错误并跳过
  → [若有 backend] 编译 wasm(wasmi) → 用“受能力门控的 import”实例化
  → 调 dispatch:"metadata" 握手(可选)
  → 注册贡献(主题/命令/面板/钩子/设置/存储)
  → enabled
安装后初始状态(0.2): 含 [backend] 的插件 MUST 落地为 disabled，enable = 用户对
  capabilities 的授权动作(宿主 UI 展示能力清单并确认)；纯零代码插件(如纯主题) MAY 自动 enable
启停: disable 后宿主 MUST NOT 再调其钩子/命令；enable 恢复；
  被**当前活动数据源**引用的存储插件 MUST NOT 被 disable/卸载(宿主拒绝, 409)
卸载: 释放实例，移除注册；删除 = 卸载 + 删目录
```

- 后端实例化策略：宿主 MUST 为**每次调用新建实例**（状态隔离，不跨调用共享可变状态）。单次调用 MUST 施加资源上限（§11）。
- 钩子/命令调用 MUST 在 server 的 `spawn_blocking` 中执行（不阻塞异步运行时）。

---

## 6. 后端 ABI（wasm，Rust-only）

编码一律 **UTF-8 JSON**。指针/长度均 `u32`，打包返回值 `u64 = (ptr << 32) | len`。

### 6.1 内存与分配（插件 MUST 导出）

- `memory`：线性内存。
- `plugin_alloc(size: u32) -> u32`：返回可写 `size` 字节的指针。
- `plugin_free(ptr: u32, size: u32)`：释放。

### 6.2 host → 插件：`plugin_dispatch`（插件 MUST 导出）

```
plugin_dispatch(ptr: u32, len: u32) -> u64
```
- 入参：`ptr/len` 指向插件内存里的**请求 JSON**：`{ "method": string, "params": <json> }`。
  请求缓冲区归**宿主**所有（宿主经 `plugin_alloc` 分配写入、调用结束后宿主 `plugin_free`）；插件 MUST NOT 释放它（0.2 澄清）。
- 返回：打包的 `(out_ptr, out_len)`，指向插件内存里的**响应 JSON**（§6.4）。宿主读完 MUST 调 `plugin_free` 释放。

### 6.3 插件 → host：`host_call`（宿主 MUST 提供为 import，模块名 `joplin`）

```
joplin.host_call(ptr: u32, len: u32) -> u64
```
- 插件把请求 JSON 写进自己的内存，传 `ptr/len`。
- 宿主读取 → 鉴权（§7）→ 执行 → 通过回调插件的 `plugin_alloc` 把**响应 JSON** 写入插件内存，返回打包 `(out_ptr, out_len)`。插件读完 MUST 调 `plugin_free`。
- 该 import 是插件访问宿主的**唯一**通道；具体能做什么由 capabilities 决定。

### 6.4 JSON 信封与错误模型

请求：
```json
{ "method": "notes.get", "params": { "id": "…" } }
```
响应（二选一）：
```json
{ "ok": true,  "result": { … } }
{ "ok": false, "error": { "message": "…", "code": "forbidden" } }
```
- `code` 取值（约定）：`forbidden`(能力不足) / `not_found` / `invalid` / `internal` / `unsupported`。
- 插件返回非法 JSON、越界指针、或超资源上限：宿主 MUST 视该调用失败（`internal`），不得使整库崩溃。

> **SDK**：`plugin-sdk`（Rust）封装上述 ABI，作者只写 `#[on_before_save] fn(note: Note) -> Result<Note>` 之类；共享 `jasper-core` 的 `Note/Folder` 类型，serde 直接序列化。

### 6.5 方法清单

**host → 插件（plugin_dispatch 的 method）**

| method | params | result | 触发 |
|---|---|---|---|
| `metadata` | — | `{ ok: true }` | 握手（可选） |
| `hook.before_save` | `{ note }` | `{ note }` | 订阅 `hooks=["before-save"]`，保存前 |
| `command` | `{ id, args? }` | `{ result? , ui? }` | 命令被触发 |
| `editor.transform`（0.4 落地） | `{ phase, text }` | `{ text }` | `contributes.editor`，`phase`∈`before-save\|input`；纯文本 in/out，**无 notes/ai 上下文**（`notes.*`→`unsupported`，同 hooks/storage） |
| `ui` | `{ view, state? }` | `UiNode`（§9.3） | 动态面板取声明树 |
| `storage.list_items` | `{ storage, config }` | `{ items: ItemStat[] }` | 数据源为该 provider 时（0.2，下同） |
| `storage.get_item` | `{ storage, config, name }` | `{ content }` | |
| `storage.put_item` | `{ storage, config, name, content }` | `{}` | |
| `storage.delete_item` | `{ storage, config, name }` | `{}` | |
| `storage.get_resource` | `{ storage, config, resource_id }` | `{ data_b64 }` | |
| `storage.put_resource` | `{ storage, config, resource_id, data_b64 }` | `{}` | |
| `storage.delete_resource` | `{ storage, config, resource_id }` | `{}` | |
| `storage.init_new` | `{ storage, config }` | `{}` | 「新建库」时 |

**`storage.*` 约定（0.2）**
- `storage` = 贡献的 provider id；`config` = 该数据源的配置对象（与 `config_schema` 对齐），每次调用传入——插件应视自己为**无状态**（每次调用新实例，§5），按需重建客户端。
- `ItemStat = { name, updated_time }`：`name` 形如 `<32hex>.md`；`updated_time` 为条目 mtime（Unix 毫秒）。插件 MUST 尽力返回真实 mtime（WebDAV 类取 `getlastmodified`）；返回 `0` 合法，但宿主将失去增量缓存（每次全量拉取）。
- 二进制经 **base64**（`data_b64`）。
- 语义对齐宿主 `StorageBackend`：`delete_*` 幂等（目标不存在视为成功）；`init_new` 建根目录 + `.resource/` + 写默认 `info.json`，仅「新建库」时被调用。

**插件 → host（host_call 的 method，括号为所需能力）**

| method | params | result | 能力 |
|---|---|---|---|
| `log` | `{ level, message }` | `{}` | 无 |
| `time.now`（0.2） | — | `{ unix_ms }` | 无（沙箱无时钟；签名协议如 S3 SigV4 需要） |
| `system.locale`（0.4） | — | `{ locale }` | 无（当前 UI 语言代码，插件据此本地化自己运行时产出的文字） |
| `notes.get` | `{ id }` | `{ note }` | `notes:read` |
| `notes.search` | `{ query, limit? }` | `{ notes: NoteRef[] }` | `notes:read` |
| `notes.list_folders` | — | `{ folders: FolderRef[] }` | `notes:read` |
| `notes.upsert` | `{ id, title?, body? }` | `{ note, pending }` | `notes:write` |
| `notes.create` | `{ parent_id, title?, body? }` | `{ note, pending }` | `notes:write` |
| `ai.complete` | `{ messages, options? }` | `{ content }` | `host:ai` |
| `settings.get` | `{ key }` | `{ value }` | `settings` |
| `settings.set` | `{ key, value }` | `{}` | `settings` |
| `http.request`（0.2） | `{ method, url, headers?, body_b64?, timeout_ms? }` | `{ status, headers, body_b64 }` | `host:http` |

**`http.request` 约定（0.2）**
- 宿主代理执行（插件不碰 socket）；仅 `http:`/`https:`。
- 限额（宿主 MUST 施加，可配置）：响应体 ≤ 128 MiB、重定向 ≤ 5、`timeout_ms` 默认 30 000、上限 120 000。
- **非 2xx 状态码照常以 `ok:true` 返回**（`status` 带回，错误语义留给插件——WebDAV 需要 404 判断）；连接失败/超时等网络错误才是 `{ok:false, code:"internal"}`。

**`system.locale` 约定（0.4）**
- 返回**当前 UI 语言代码**（如 `en`/`zh`/`fr`）——即用户在应用里选的界面语言。供插件本地化**自己运行时产出的文字**（chat 回复、`ui` 树里的文案、命令结果里的提示等）。与语言包贡献（§3.10）**正交**：语言包翻的是宿主界面，`system.locale` 让插件翻自己的产出。
- **无需能力**，任何分发上下文都可调（读的是宿主配置，不涉及笔记库/写入）。
- 宿主实现约定：宿主**持久化**当前 UI 语言（前端在切换/启动时同步到宿主，jasper 存 config.db 并经 `GET/PUT /api/locale` 存取）；未设置时 MUST 回落一个有效内置代码（jasper 回落 `en`），保证插件总能拿到非空代码。

**`notes.*` / `ai.complete` 约定（0.3）**
- **仅在 `command` 与 `ui` 分发上下文可用**：`hook.before_save` / `storage.*` 分发内调用一律返回 `{ok:false, code:"unsupported"}`（防写入重入与索引期死锁；后续版本可能放开只读子集）。
- **写确认 = 提案回传**：宿主按插件托管一个「写入免确认」开关（§7，默认关）。
  - 开关**关**（默认）：`notes.upsert`/`notes.create` MUST NOT 落盘；返回 `{note: 应用改动后的完整 note, pending: true}`，并把提案累积进本次触发调用的 HTTP 响应顶层 `pending_writes`（§9.5），由宿主 UI 确认后经普通笔记 API 落盘（before-save 钩子照常）。
  - 开关**开**：宿主立即写入（**跳过 before-save 钩子**，防重入——两条路径的字节差异记录在案，§11）并返回 `{note, pending: false}`。
  - 本次 wasm 调用最终失败（超限/trap/业务错误上抛）时，已累积的提案 MUST 一并丢弃。
- pending 的 `create` 提案 `note.id` 为空串（id 在用户批准落盘时由宿主生成）；插件无法在 pending 模式下对新建笔记链式写入（记录在案）。
- 只读模式：`notes.upsert`/`notes.create` → `{ok:false, code:"forbidden"}`。
- `notes.get`：不存在 → `not_found`。`notes.search`：`limit` 默认 20、上限 100。`notes.list_folders`：按 `(title, id)` 排序。
- `notes.create`：`parent_id` MUST 是已存在的笔记本，否则 `invalid`（比宿主 HTTP API 更严——插件不应静默产出孤儿笔记）。
- `ai.complete`：`options = { model?, temperature?, max_tokens? }`（宿主钳制：temperature 0..=2，max_tokens 1..=32768；`model` 缺省用宿主配置）。一次性返回 `{content}`（流式仍后置，§12 决策 4 不变）。宿主 AI 未配置 → `{ok:false, code:"internal"}`（message 指明去设置页配置；不用 `unsupported`——那会被 SDK 误读为宿主太旧）。网络等待计入 IO 豁免（§11）。

**数据形状**
```
Note      = { id, title, body, parent_id, markup_language, created_time,
              updated_time, is_todo, todo_completed, is_conflict, source_url, order }
NoteRef   = { id, title, parent_id }
FolderRef = { id, title, parent_id }
Message   = { role: "system"|"user"|"assistant", content: string }
```
- `Note` = jasper-core `Note` 的 serde 形状，与 `hook.before_save` 的 note 完全一致；插件反序列化 MUST 忽略未知键（向前兼容）。
- `markup_language`：1=Markdown，2=HTML。
- `notes.upsert` 仅改传入的字段；其余元数据宿主**逐字保留**（复用 `serialize::update_note_md`）。

---

## 7. 能力（capabilities）

默认全拒。manifest `[backend].capabilities` 申请，安装时向用户展示并征得同意。

| 能力 | 解锁的 host 方法 | 说明 |
|---|---|---|
| `notes:read` | `notes.get` / `notes.search` / `notes.list_folders` | 只读 |
| `notes:write` | `notes.upsert` / `notes.create` | **默认每次写入以提案回传**（§6.5），宿主 UI 确认后落盘；「免确认」开关**由宿主托管**（插件面板按插件开启，宿主端点见 §9.5），MUST NOT 放进插件自身 `settings.schema`（否则插件可经 `settings` 能力自改绕过确认） |
| `host:ai` | `ai.complete` | 密钥/端点在宿主，插件不可见 |
| `settings` | `settings.get` / `settings.set` | 插件作用域 KV |
| `host:http`（0.2） | `http.request` | 宿主**代理**的 HTTP(S) 出口（限额见 §6.5/§11）。安装/启用确认 MUST 显著提示「该插件可访问网络」。无域名白名单（存储端点来自用户配置）。 |

- `log`、`time.now`、`system.locale` 不需能力（毫秒粒度时间 / UI 语言代码的泄露面可忽略）。
- 0.1 的「`host:fetch` 本版不提供」由 `host:http` 取代：仍**不是**任意网络出口——只有宿主代理、限额受控的 HTTP(S)，永无裸 socket（§11 非目标）。
- 调用未授权方法 MUST 返回 `{ok:false, error:{code:"forbidden"}}`。

---

## 8. 钩子与扩展点

| 钩子名（hooks 值） | dispatch method | 语义 |
|---|---|---|
| `before-save` | `hook.before_save` | 笔记保存前改写；返回的 `note` 写回。多个插件按加载顺序串联。 |

其它扩展点不靠 `hooks`，靠 `contributes.*` 声明触发：
- **命令**：`contributes.command` + 触发（工具栏/面板）→ `command` 方法。
- **编辑器**（0.4 阶段 4 落地）：`contributes.editor` → `editor.transform` 方法（`before-save` 作用于 body，`input` 作用于 debounce 后文本）。任一插件的 `editor.transform` 返回错误时，宿主 MUST 跳过它、沿用上一步文本（**不丢用户输入**）；前端在源码与实时预览两模式都接 `input` 相位并保守替换缓冲（§3.7）。

后置（本版不实现，预留方法名）：`render_fence`（自定义代码块渲染）、`import`/`export`。

钩子约束：
- `hook.before_save` 返回错误时，宿主 MUST 放弃该插件的改写、用原 note 继续保存（**不因插件失败而丢用户数据**），并提示。
- **改写以落盘为准，不承诺回显**（0.2 澄清）：钩子发生在保存链路的服务端，宿主编辑器 MAY 不把改写结果回填进当前编辑缓冲（jasper 即如此——自动保存频繁，回填会跳光标）。用户重新打开笔记后可见改写；插件作者不应假设改写立即出现在编辑器里。

---

## 9. 前端贡献

### 9.1 主题：CSS 令牌契约（已实现，本节为权威清单）

主题 = 一份 CSS，覆盖以下令牌；选择器 MUST 为 `:root[data-theme='<主题id>'] { … }`。未覆盖的令牌回退到 `base` 基调默认值。
本清单为 v0.1 **冻结契约、只增不删**：新增令牌升 MINOR；删除/改名/改语义算破坏性，升 MAJOR。

**颜色语义令牌（可覆盖）**
```
--bg --bg-bar --bg-side
--text --text-dim
--border --hover
--accent --accent-soft
--code-bg
--danger --danger-soft --danger-soft-weak
--success
--on-accent          /* 强色按钮上的文字色 */
--overlay            /* 弹层遮罩 */
--shadow-modal       /* 弹层阴影（完整 box-shadow 值） */
--code-block-bg --code-block-text
```

**图标令牌（可覆盖，值为 `url("data:image/svg+xml,…")`）**
```
--icon-close --icon-plus --icon-settings --icon-image --icon-folder
--icon-file --icon-alert --icon-edit --icon-trash --icon-eye
--icon-code --icon-rich --icon-attach --icon-clean --icon-cloud
--icon-globe --icon-sun --icon-moon --icon-contrast --icon-palette
--icon-chat --icon-send --icon-bot                        /* 0.3 新增 */
```
- 图标经 CSS `mask` + `currentColor` 渲染：**单色、跟随文字色**；SVG 颜色无意义，只取 alpha。
- 主题 SHOULD NOT 依赖未在此列出的内部 class 选择器（不稳定，不保证兼容）。
- `--palette-*`（基础调色板）是实现细节，主题 MAY 覆盖但不在稳定契约内。

### 9.2 widget 词汇表（0.3 钉死事件契约）

插件**只声明用哪个 widget + 给数据**，不写 HTML/JS/CSS。本版词汇表与交互契约：

| widget | props | 交互契约 |
|---|---|---|
| `chat` | `placeholder?`, `command?`（动态模式；静态模式用 sidebar 的 `command`） | 消息列表由**前端持有**（静态 sidebar chat 会话可由宿主持久化 + 多会话，见下）。发送 → 调 command，`args = { messages: Message[]（含刚输入的 user 消息）, input: string, note_id: string\|null, selection?: {text} }`；result 含字符串 **`reply`** → 以 assistant 消息追加（markdown 渲染）；动态模式下 result 含 `ui` → 替换整棵树 |
| `button` | `label`, `icon?`, `command`, `args?` | 点击 → command，`args = props.args ?? {}` 并入 `note_id` |
| `form` | `fields`（§10 词汇）, `values?`, `submit_label?`, `command` | 前端按 §10 校验通过后提交 → command，`args = { values }` 并入 `note_id` |
| `list` | `items: {id,title,subtitle?,icon?}[]`, `command?` | 点条目 → command，`args = { id }` 并入 `note_id`；无 `command` 则纯展示 |
| `tree` | `nodes: {id,title,children?}[]`, `command?` | 点节点 → command，`args = { id }` 并入 `note_id`；无 `command` 则纯展示 |
| `markdown` | `source: string` | 无交互；DOMPurify 净化 + `:/id` 资源改写后渲染 |
| `checkbox`（0.4） | `label`, `checked?`, `command?`, `args?` | 勾选切换 → command，`args = { ...props.args, checked: bool }` 并入 `note_id` |
| `select`（0.4） | `label?`, `options: (string \| {value,label})[]`, `value?`, `command?` | 变更选项 → command，`args = { value }` 并入 `note_id` |
| `divider`（0.4） | — | 无交互；一条分隔线（布局原语） |
| `heading`（0.4） | `text`, `level?`（1–3，缺省 2） | 无交互；一段区块标题（布局原语） |

统一约定：
- 侧边栏内一切命令调用 MUST 并入 `note_id`（当前选中笔记 id，可为 `null`）。
- **选区上下文（向后兼容新增）**：当用户在**笔记内容区**（阅读视图 / 编辑器）里选中了文字时，命令 args 并入 `selection = { text: string }`（当前选区文字）；无选区则不带该键。插件据此实现「改写/翻译选中段落」等——**可选、宿主实现说明**：jasper 全局监听 `selectionchange` 记住笔记内容区里的选区、发送时并入（切换笔记时清空）；老宿主不带此键，插件 MUST 容忍缺失。
- **会话持久化（宿主实现说明，非契约）**：静态 sidebar chat 的消息历史 MAY 由宿主持久化并支持**多会话**（jasper 按 `pluginId/panelId` 存 localStorage，chat 面板带会话切换/新建/删除/重命名）。这对插件透明——契约仍是每次发送把完整 `messages` 传给命令；插件保持无状态。
- 命令 result 中：`ui`（UiNode）→ 替换面板树（动态模式）；`reply`（string 或 §9.2.1 的 locale map）→ 追加进 chat；其余键宿主忽略（向前兼容）。
- HTTP 响应顶层 `pending_writes`（§9.5）交给宿主确认弹窗，与 widget 无关。
- 词汇表扩充是**前端渲染契约**（server-driven UI 树里未知 `type` 前端安全忽略，§9.3），故新增 widget 向前兼容、不升 MAJOR。0.4 阶段 4 补齐设计文档 §7.2 早列入的 `checkbox`/`select` + 布局原语 `divider`/`heading`；其可本地化文本字段（`checkbox.label`/`select.label`/`select` option 的 `label`/`heading.text`）遵循 §9.2.1。未来仍按需扩（升 MINOR）。

#### 9.2.1 可本地化文本字段（0.4）

以上 widget 里**面向用户的文本字段**——`markdown.source`、`button.label`、`form.submit_label`、`chat.placeholder`、`list.items[].title`/`subtitle`、`tree.nodes[].title`、`checkbox.label`（0.4）、`select.label` 与其 option 的 `label`（0.4）、`heading.text`（0.4）、命令 result 的 `reply`——**MAY 是纯字符串，或一张 locale map** `{ [locale]: string }`（键为语言代码，同 §3.10 的 `code`）。这样插件可**返回多语言 UI**，由宿主/前端按当前 UI 语言挑合适的一门显示。

```json
{ "type": "button", "props": { "label": { "en": "Save", "zh": "保存", "fr": "Enregistrer" }, "command": "save" } }
```

- 宿主解析规则：纯字符串原样用；locale map 按**当前 UI 语言**取值，缺失回落链 **当前语言 → `en` → `zh` → 任意首个非空 → 空串**。切换语言即时重解析（无需插件重算）。
- 非文本字段（`command`/`icon`/`id`/`args`/`fields` 等）**不**本地化，仍是原类型。
- 与 `system.locale`（§6.5）**互补**：固定标签用「返回多语言、前端挑」最省事；运行时生成的文字（如 AI 回复正文）用 `system.locale` 只产一门。二者可在同一插件并用。
- 键用语言代码（`en`/`zh`/`fr`…）；用户装了对应语言包时（§3.10）该语言也会被选中。宿主对未知键宽容忽略。

### 9.3 server-driven UI 声明树

动态面板（`dispatch:"ui"`）返回一棵 **UiNode**：
```
UiNode = {
  "type": string,                 // widget 名（§9.2）
  "props": { … },                 // 该 widget 的 props
  "children": UiNode[]            // 可选
}
```
- 节点内的交互通过 `props.command`（指向 `contributes.command`，须 `target="backend"`）回调后端，args 形状按 §9.2 各 widget 契约。
- 宿主 MUST 只渲染已知 widget；未知 `type` MUST 安全忽略——**连同其 `children` 一起跳过**（不报错、不注入）。
- `children` 本版仅作纵向堆叠布局，无其它布局语义。

### 9.4 命令 / 侧边栏 / 工具栏 / 设置贡献

- **侧边栏**：`contributes.sidebar` 在左栏加入口，点开渲染指定 `widget`（静态）或 `ui` 方法返回的树（动态，`view` 字段，§3.5）。jasper 的实现：入口按钮在左栏底部，面板在阅读区右侧第 4 栏 dock（约 320px，可关闭）——此为宿主实现说明，非契约。
- **工具栏**：`contributes.toolbar` 把某命令放到 `note-toolbar` 或 `topbar`。
- **命令**：`target="backend"` 调后端 `command`；`target="builtin"` 触发宿主内置动作（白名单，TBD）。
- 显示图标/文字由全局按钮显示模式（已实现 `ui.svelte.ts` 的 `both|icon|text`）统一控制。
- **`note-toolbar` 后端命令的宿主约定**（0.2 落地）：宿主以 `args = { note_id, title, body }`（当前编辑器内容）调用 `command`；
  若返回的 `result` 含字符串字段 `body`，宿主用它**替换编辑器缓冲**（进入正常的自动保存链路）。
  其余 result 形状由宿主忽略（向前兼容）。宿主经 `POST /api/plugins/{id}/commands/{cmd}`（body `{ args }`）触发；
  只读模式下该端点被写方法守卫拦截。命令按 Normal 档限额执行（AI 类命令的网络等待经 host_call 豁免 CPU 墙钟）。

### 9.5 宿主端点（0.3 新增/扩展；宿主实现约定）

```
POST /api/plugins/{id}/commands/{cmd}   响应扩为 { result, pending_writes }（0.3 向后兼容新增键；
                                        pending_writes 恒在，无提案时为空数组）
POST /api/plugins/{id}/ui/{view}        body { state? } → { ui: UiNode, pending_writes }；
                                        view 未在该插件 contributes.sidebar 声明 / 插件禁用 → 404
POST /api/plugins/{id}/editor/transform body { phase, text } → { text }（0.4 阶段 4）；
                                        phase 非 before-save|input → 400；该相位未在 contributes.editor
                                        声明 / 插件禁用 → 404；纯文本 in/out，无 pending_writes
PUT  /api/plugins/{id}/auto-approve     body { enabled } → 刷新后的插件信息；写「免确认」开关（§7）
GET/PUT /api/ai/config                  宿主级 AI 配置 { provider, base_url, api_key, model }
                                        （provider ∈ anthropic|openai；密钥存宿主，插件永不可见）
```

- `pending_writes` 元素形状：`{ action: "update"|"create", plugin_id, note: {id,parent_id,title,body,…}, original: {title,body}|null }`（`create` 的 `original` 为 null、`note.id` 为空串）。
- 只读模式：以上 POST/PUT 全部被写守卫拦截（403）——**只读下动态侧栏、chat、编辑期变换均不可用**（记录为已知取舍；只读下编辑器本就隐藏，`editor/transform` 不会被触发）。
- `GET /api/ai/config` 回显 `api_key`（与数据源密码同姿势，本机信任模型）；插件面的 `settings.schema` secret 不回显规则（§10）不适用于此。

---

## 10. 设置 schema

`[settings.schema]` 每项：
```toml
[settings.schema]
api_key = { type = "secret", label = "API Key" }
model   = { type = "string", default = "claude-opus-4-8" }
provider = { type = "select", options = ["claude", "openai"], default = "claude" }
```

> 「写入免确认」这类**授权语义**的开关 MUST NOT 声明在此（0.3 起明确）：它由宿主托管（§7/§9.5），
> 插件经 `settings` 能力可读写自己的 schema 值，放这里等于插件可自行绕过写确认。

| `type` | 渲染 | 值类型 |
|---|---|---|
| `string` | 单行输入 | string |
| `multiline` | 多行 | string |
| `secret` | 密码框（值在宿主存储；后端经 `settings.get` 读，前端**不回显**） | string |
| `bool` | 开关 | bool |
| `number` | 数字 | number |
| `select` | 下拉（需 `options`） | string |

公共可选键：`label`、`default`、`description`、`required`（0.2，bool）、`placeholder`（0.2，string）。值存于宿主、**按插件 id 隔离**。

> 本字段词汇同时用于 `[[contributes.storage]].config_schema`（§3.9）；注意两处 secret 的回显规则不同（§3.9 末条）。

---

## 11. 安全与资源限制

**信任分档**
- 纯主题（CSS）/ 纯语言包（JSON catalog）/ widget 声明：低风险，默认放行（语言包只作文本注入进 i18n，不含可执行代码）。
- 后端 wasm：沙箱 + 能力白名单 + 密钥不下放；安装时 MUST 展示申请的 capabilities 并征得同意。
- **不支持任意前端 JS / iframe**（本版）。语言包 catalog 是纯数据；其文本经宿主既有的文案渲染路径展示，宿主 SHOULD 对含标记的文案照原文本处理（不因语言包而放宽净化）。

**沙箱与上限（后端每次调用）**
- 默认零 WASI 能力（无 fs/socket/clock，除非未来显式授权）。
- 宿主 MUST 施加：**fuel 上限**、**线性内存上限**、**墙钟超时**，均可配置。超限 MUST 中止该调用并记错。
- 墙钟（0.2 修正）：MUST 只计插件**自身执行**时间——`host_call` 期间的宿主 IO（如 `http.request` 网络等待）不计入。
- 调用分两档（0.2）：**Normal**（默认 64 MiB / fuel 1e9 / CPU 2s）；**Storage**（`storage.*`：默认 256 MiB / fuel 5e9 / CPU 10s——base64 大资源需要更大内存与解码预算）。
- base64-in-JSON 使单个资源的实际上限 ≈ 存储档内存限额 / 2.7 ≈ 90 MiB（v1 接受的代价，记录在案）。
- 宿主 MAY 在插件反复超限/报错后自动停用它。

**网络与裸 socket（非目标，0.2）**
- 插件的网络出口**只有** `host:http`（宿主代理）；**永不**向 wasm 暴露裸 TCP/UDP socket。
- 因此 SMB/NFS 这类裸 TCP 协议不是插件目标：在操作系统层挂载后用内置 `local` 数据源即可。

**写入与外发**
- `notes:write` 默认**提案回传**（§6.5/§7）：插件调用不落盘，宿主 UI 确认后经普通笔记 API 落盘（before-save 钩子照常）；宿主托管的「免确认」开启时直写但**跳过钩子**（防重入）——同一插件写入在两条路径下可能产生不同字节，记录在案。
- 插件写入与编辑器并发保存同一笔记 = 最后写赢（与两个并发 API 保存等价，0.3 不引入锁）。
- `ai.complete` 的密钥/端点由宿主托管，插件不可见；宿主 SHOULD 对外发目标做 allowlist。

**主题 CSS**
- SHOULD NOT 含远程 `@import` / 远程 `url()`（防外联探测）；宿主 MAY 在加载时净化/拒绝。

---

## 12. v0.1 决策（已冻结，2026-06-30）

1. **包扩展名**：`.jplug`（宿主同时接受 `.zip`）。
2. **后端实例**：每次调用新建实例（状态隔离）。
3. **资源上限默认**：内存 64 MiB、墙钟 2s、fuel ~1e9 指令，均可配置。
4. **`ai.complete`**：本版一次性返回；流式（增量）后置——以后作为可选能力新增，不改现有签名。
5. **manifest**：仅 `manifest.toml`，不接受 JSON。
6. **主题契约**：§9.1 令牌清单为权威、**只增不删**（删/改 = 破坏性，升 MAJOR）。

`apiVersion 0.1` 据此冻结；后续实现（`plugin-sdk` + 宿主加载器）照本规范。

### 12.1 v0.2 决策（2026-07-02）

1. **存储 provider 扩展点** `[[contributes.storage]]`（§3.9 / §6.5）：云盘等文件后端插件化，不再全部写进核心；首个参考实现 = WebDAV-as-plugin（内置 webdav 保留作对照）。
2. **`host:http`**（§7）：宿主代理 HTTP(S)，取代 0.1 的「host:fetch 不提供」；仍无裸 socket。
3. **SMB/裸 TCP 非目标**（§11）：OS 挂载 + `local` 数据源覆盖。
4. **安装后默认 disabled**（§5）：含 `[backend]` 的插件启用即能力授权；纯零代码插件自动启用。
5. **资产端点**：宿主提供 `GET /api/plugins/{id}/assets/{path}`（主题 css 等；仅 enabled 插件、防路径逃逸）。
6. **秘密回显不对称**（§3.9）：数据源配置回显、插件设置不回显。
7. **in_use 守护**（§5）：活动数据源引用的插件禁止停用/卸载。
8. **限额两档 + CPU-only 墙钟**（§11）。

### 12.2 v0.3 决策（2026-07-02）

1. **写确认 = 提案回传**（§6.5）：`notes.upsert/create` 默认不落盘、返回 `{note, pending:true}`，提案随触发调用的 HTTP 响应顶层 `pending_writes` 交前端确认，批准走普通笔记 API（钩子照常）。零新增服务端状态、不阻塞 wasm。
2. **免确认开关宿主托管**（§7/§9.5）：按插件存宿主配置，`PUT /api/plugins/{id}/auto-approve` 写入；直写路径跳过 before-save 钩子（防重入）。§10 的旧示例 `auto_approve_write` 因此删除。
3. **`ai.complete` 由宿主以 genai 库代理**：宿主级配置 `{provider: anthropic|openai, base_url?, api_key, model}`（`GET/PUT /api/ai/config`）；openai provider 走 chat-completions 协议 + 自定义 base_url（覆盖 Ollama/DeepSeek/中转）。一次性返回不变（v0.1 决策 4）。
4. **notes/ai 仅 command/ui 上下文**（§6.5）：hooks/storage 分发内 `unsupported`。
5. **sidebar 扩 `command`/`view`**（§3.5）：静态 widget 交互走 command、动态树走 ui 方法；入口左栏底部、面板右侧 dock（实现说明）。
6. **widget 事件契约冻结**（§9.2）：chat 消息前端持有、`result.reply` 追加、`result.ui` 换树、一切命令并入 `note_id`。
7. **pending create 无 id**（§6.5）：id 批准时生成，插件不可链式写。
8. **只读模式**：commands/ui/auto-approve/ai-config 的写方法全拦（§9.5）。

### 12.3 v0.4 决策（2026-07-03）

1. **语言包扩展点** `[[contributes.locale]]`（§3.10）：应用界面语言可由插件运行时新增（法语/日语等），不再把每门语言都编进主体——与主题贡献同构（零代码、纯数据、自动启用、经资产端点托管）。
2. **catalog = 扁平 key→译文**：宿主 message key 集合是宿主内部契约（不进本规范冻结清单）；缺失键回落 `base`（默认 `en`）再回落内置，未知键忽略。宿主升级只增不删旧 key 时旧语言包继续可用。
3. **内置语言不可被劫持**：语言 `code` 与内置 `en`/`zh` 相同者以内置为准、忽略插件项。
4. **纯数据信任档**：语言包不含可执行代码，与主题 CSS 同属低风险默认放行；不因语言包放宽文案净化（§11）。
5. **`system.locale`（免能力 host 方法）**：与语言包正交——语言包翻宿主界面，`system.locale` 让**任意插件**读当前 UI 语言以本地化自己运行时的产出（chat 回复 / 动态 UI 文案等）。宿主**持久化** UI 语言（jasper 存 config.db，`GET/PUT /api/locale`；前端切换/启动时同步；未设回落 `en`）。这是「插件自己运行时的多语言」的落点，不属于语言包特性。
6. **返回多语言 UI**（§9.2.1）：widget/命令 result 的用户可见文本字段可为 locale map `{ [locale]: string }`，宿主/前端按当前语言挑（回落 当前→en→zh→首个非空→''）。与 `system.locale` 互补——固定标签用「返回多语言」（省一次 host_call），生成内容用 `system.locale`（不必产每一门）。纯前端解析，服务端把 `ui`/`reply` 原样透传，无端点/无格式变更。

### 12.4 v0.4 阶段 4 落地（2026-07-04；apiVersion 不变，仍 0.4）

阶段 4 = 落地两处**早已在规范里、此前未实现**的契约，均向后兼容、不升 apiVersion（`editor.transform`/`contributes.editor` 自 0.1/0.2 起就在 §3.7/§6.5/§8；widget 词汇扩充是前端渲染契约，未知 type 前端安全忽略）。

1. **编辑期钩子** `contributes.editor` → `editor.transform`（§3.7/§6.5/§8）：编辑期把编辑缓冲文本交给 wasm 改写。宿主端点 `POST /api/plugins/{id}/editor/transform`（§9.5），纯文本 in/out、无 notes/ai 上下文（走无上下文 `dispatch`，防写入重入）；任一插件失败即跳过（不丢用户输入）。SDK 加 `editor` 槽（`register! { editor: fn(&str,String)->Result<String> }`）。
2. **前端两个模式都接 `input` 相位**（实现说明）：单个 CodeMirror 6 实例的源码与实时预览同一缓冲，debounce 输入停顿后依次调声明插件、保守替换缓冲——仅在**真实用户输入**（打字/删除/粘贴）后触发（排除程序化 dispatch/外部同步，天然防环），等待期间用户又敲字则丢弃变换（不覆盖新输入）；`before-save` 相位与 `command` 复用后置（现有服务端 `hook.before_save` 已覆盖保存前改写）。
3. **widget 词汇扩充**（§9.2）：补齐设计文档 §7.2 早列入的 `checkbox`/`select`（独立交互控件）+ 布局原语 `divider`/`heading`。新增 widget 只在前端渲染层，服务端 `ui`/`reply` 仍原样透传；其可本地化文本字段遵循 §9.2.1。

```
my-theme.jplug
├── manifest.toml
└── assets/themes/oceanic.css
```
```toml
# manifest.toml
id = "oceanic"
name = "Oceanic"
version = "1.0.0"
apiVersion = "0.1"

[[contributes.theme]]
id = "oceanic"
name = "Oceanic Dark"
base = "dark"
css = "assets/themes/oceanic.css"
```
```css
/* assets/themes/oceanic.css */
:root[data-theme='oceanic'] {
  --bg: #0f1b2d;
  --text: #cdd9e5;
  --accent: #4fb6c2;
  --border: #1e3047;
  /* 顺手换个图标 */
  --icon-settings: url("data:image/svg+xml,%3Csvg…%3E");
}
```

## 附录 B：最小后端插件（保存前格式化）

```toml
# manifest.toml
id = "trim-trailing"
name = "去行尾空白"
version = "0.1.0"
apiVersion = "0.1"

[backend]
wasm = "plugin.wasm"
capabilities = []          # 纯计算，不碰笔记库/AI
hooks = ["before-save"]
```
```rust
// 用 plugin-sdk（伪代码）：作者只写业务，ABI/JSON 由 SDK 处理
use joplin_plugin_sdk::*;
use jasper_core::model::Note;

#[on_before_save]
fn before_save(mut note: Note) -> Result<Note, String> {
    note.body = note.body.lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(note)
}
```

## 附录 C：最小语言包插件（0.4）

```
lang-fr.jplug
├── manifest.toml
└── assets/locales/fr.json
```
```toml
# manifest.toml —— 零代码，安装后自动启用
id = "lang-fr"
name = "Français"
version = "1.0.0"
apiVersion = "0.4"

[[contributes.locale]]
code = "fr"
name = "Français"
messages = "assets/locales/fr.json"
```
```json
// assets/locales/fr.json —— 只列想翻的 key，其余自动回落 base(en)
{
  "common.save": "Enregistrer",
  "common.cancel": "Annuler",
  "topbar.search": "Rechercher des notes…"
}
```

## 附录 D：最小编辑期插件（0.4，输入时变换）

```toml
# manifest.toml —— 输入停顿后把编辑缓冲交给 editor.transform 改写
id = "fix-quotes"
name = "智能引号"
version = "0.1.0"
apiVersion = "0.4"

[backend]
wasm = "plugin.wasm"
capabilities = []          # 纯文本计算，不碰笔记库/网络

[[contributes.editor]]
on = "input"
```
```rust
// 用 plugin-sdk：作者只写 fn(phase, text) -> Result<String>，ABI/JSON 由 register! 处理
use jasper_plugin_sdk as sdk;
use sdk::PluginError;

fn transform(_phase: &str, text: String) -> Result<String, PluginError> {
    Ok(text.replace("\"", "”").replace("--", "—"))
}

sdk::register! { editor: transform }
```
