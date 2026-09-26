---
name: jasper-plugin
description: Author a Jasper plugin (theme / before-save hook / storage provider / editor command) — scaffold, ABI/SDK usage, the wasm-toolchain gotchas, testing recipe, and packaging. Use when creating or debugging a plugin in this repo (plugins-examples/, plugin-sdk/, server/src/plugins/).
---

# Authoring a Jasper plugin

Jasper 插件 = 一个 zip 包（`.jplug`），含 `manifest.toml` + 可选 `plugin.wasm`。后端逻辑用 **Rust 编到 `wasm32-unknown-unknown`**，跑在 server 的 **wasmi 沙箱**里，经 JSON ABI 与宿主通信。本 skill 是「怎么做」和「坑在哪」；**契约**看 `docs/plugin-spec.md`（apiVersion 0.2），**架构决策**看 `docs/plugin-design.md`。

## 先决定贡献类型

一个插件由 manifest 声明它贡献什么（不互斥，可组合）。按需求选，然后**照抄对应的示例插件**当模板——这是最快的起步方式：

| 想做的事 | 贡献 | 需要 wasm | 示例模板（`plugins-examples/`） |
|---|---|---|---|
| 换肤/换图标 | `[[contributes.theme]]` | 否（纯 CSS） | 参见 spec 附录 A（本仓库无纯主题示例，见 `web/src/themes/`） |
| 加一门界面语言（法语等） | `[[contributes.locale]]`（0.4） | 否（纯 JSON） | spec 附录 C；夹具 `web/e2e/fixtures/locale.jplug`（生成器 `web/e2e/make-plugin-fixtures.py`） |
| 保存前改写正文 | `[backend] hooks=["before-save"]` | 是 | `trim-trailing/` |
| 新文件后端（云盘/协议） | `[[contributes.storage]]` + `host:http` | 是 | `webdav-storage/`（HTTP 系）；`s3-storage`（含 SigV4 签名，在 **jasper-plugins 仓库**） |
| 编辑器工具栏按钮/一次性动作 | `[[contributes.command]]` + `[[contributes.toolbar]]` | 是 | `ai-polish`（在 **jasper-plugins 仓库**；调 AI + 存密钥；provider 切 anthropic/openai 格式，是「多后端格式用一个 select 切换 + provider 感知纯函数」的范式） |
| 宿主测试夹具 | 手写 ABI | 是 | `testbed/`（不随发行分发） |

**纯主题 / 纯语言包插件 MUST NOT 含 wasm**，宿主按零代码信任档加载（安装即自动启用）。含 `[backend]` 的插件安装后**默认禁用**，用户在插件面板点启用=对 capabilities 的授权。

> **语言包插件（0.4）**：`[[contributes.locale]]{code,name,base?,messages}`，`messages` 指向扁平 catalog JSON（`"<msgKey>": "<译文>"`）。key 清单以宿主内置 **en** 目录（`web/src/lib/messages.ts` 的 `en`）为权威；只翻想覆盖的键，其余自动回落 `base`（默认 `en`）。`code` 别用 `en`/`zh`（会被内置顶掉）。装好后语言出现在顶栏 LangPicker。

## 脚手架（后端插件）

复制最接近的示例目录，改三处即可：`Cargo.toml` 的 `name`、`manifest.toml`、`src/lib.rs`。

**`Cargo.toml` 必备形状：**
```toml
[package]
name = "my-plugin"
edition = "2021"

[lib]
crate-type = ["cdylib"]          # 必须——产物才是 .wasm

[dependencies]
jasper-plugin-sdk = { path = "../../plugin-sdk" }
# 需要日期解析/格式化时（storage 插件常见）：务必关 chrono 默认 feature，见「坑」#2
# chrono = { version = "0.4", default-features = false, features = ["std"] }

[profile.release]
opt-level = "s"                  # 体积优先
lto = true
```

**`src/lib.rs` 用 `register!` 宏接入**（可组合任意子集，顺序无关）：
```rust
use jasper_plugin_sdk as sdk;

// before-save 钩子：fn(Note) -> Result<Note, String>
fn before_save(mut note: sdk::core::model::Note) -> Result<sdk::core::model::Note, String> { Ok(note) }

// 命令：fn(&str /*命令id*/, Value /*args*/) -> Result<Value, PluginError>
fn command(id: &str, args: sdk::serde_json::Value) -> Result<sdk::serde_json::Value, sdk::PluginError> { todo!() }

// server-driven UI（0.3）：fn(&str /*view*/, Value /*state*/) -> Result<Value /*UiNode 树*/, PluginError>
fn ui(view: &str, state: sdk::serde_json::Value) -> Result<sdk::serde_json::Value, sdk::PluginError> { todo!() }

// 编辑期文本变换（0.4，contributes.editor → editor.transform）：
//   fn(&str /*phase: before-save|input*/, String /*text*/) -> Result<String, PluginError>
fn editor(phase: &str, text: String) -> Result<String, sdk::PluginError> { Ok(text) }

sdk::register! { before_save: before_save, command: command, ui: ui, editor: editor }
// storage 插件：sdk::register! { storage: MyStorage }（MyStorage: impl sdk::storage::Storage）
```

**加进 `plugins-examples/build-wasm.sh`** 的目录循环列表，否则不会被一键构建。

## SDK 速查（`jasper_plugin_sdk`）

- `sdk::core::model::{Note, Folder, …}` —— 与宿主**共享同一套类型**（serde），无需自定义 DTO。
- `sdk::host::log(level, msg)` —— 免能力；落宿主 stdout（带 `[plugin:id]` 前缀）。调试首选。
- `sdk::host::now_ms() -> Result<i64>` —— 免能力；**沙箱唯一的取时钟方式**（见坑 #3）。
- `sdk::host::system_locale() -> Result<String>` —— 免能力（0.4）；当前 UI 语言代码（`en`/`zh`/`fr`…）。用来本地化插件**自己运行时产出的文字**（chat 回复 / 动态 UI 文案）；与语言包正交（那是翻宿主界面）。未设回落 `en`。native-host 测试用 `set_locale()` 注入。
  - 另一条路（spec §9.2.1）：server-driven UI / 命令 `reply` 的**用户可见文本字段**可直接返回 locale map `{ en, zh, fr }`，前端按当前语言挑——**固定标签**用这个（省一次 host_call）；`system.locale` 留给**生成内容**（不必产每一门）。
- `sdk::host::settings_get(key)` / `settings_set(key, value)` —— 能力 `settings`；插件作用域 KV，secret 值前端不回显。
- `sdk::host::http_request(&HttpRequest) -> Result<HttpResponse>` —— 能力 `host:http`；宿主代理 HTTP(S)。**非 2xx 也返回 Ok（带 status）**，网络失败才 Err。二进制体 SDK 内部 base64。
- `sdk::storage::Storage` trait —— 8 方法镜像宿主 `StorageBackend`；`from_config(&Value)` 从数据源配置构造。
- `sdk::PluginError::{invalid, not_found, forbidden, internal, unsupported}` —— 错误码进 JSON 信封，宿主按码映射 HTTP 状态。

## ⚠️ 坑（wasm 工具链——这份 skill 的核心价值）

1. **`getrandom` 编不过 wasm32**：SDK 已注册 panic 版 custom backend（`plugin-sdk/src/lib.rs` 的 `rand_shim`），插件继承即可。**推论**：插件不要调 `core::serialize::new_id()`（会 panic 本次调用）——**id 由宿主生成**，插件不自造。

2. **`chrono` 默认 feature 拉进 wasm-bindgen → 沙箱无法实例化**。任何用 chrono 的插件 crate 必须 `default-features = false, features = ["std"]`（`webdav-storage` 与 jasper-plugins 的 `s3-storage` 都这么写）。同理别引入任何会带 `wasm-bindgen`/`js-sys` 的依赖。

3. **沙箱没有系统时钟**：`SystemTime::now()` 和 `chrono::Utc::now()` 在 `wasm32-unknown-unknown` 都会 panic。需要当前时间（SigV4 等签名、时间戳）**只能用 `sdk::host::now_ms()`**（它经 host_call 走 `time.now`）。

4. **验证 wasm 干净**：构建后 dump imports。`trim-trailing` 应 **零 import**；用到 host 能力的插件应**只**有 `joplin.host_call`。若看到 `__wbindgen_*` import，说明有 wasm-bindgen 依赖漏进来了（回去查坑 #2）。快速检查（Python 解析 import 段）见本仓库历史做法，或用 `wasm-tools print plugin.wasm | grep import`。

5. **`register!` 宏是累积器**：`before_save`/`storage`/`command`/`ui`（0.3）/`editor`（0.4）五槽任选、任意顺序、可组合。不用宏也行（`testbed/` 手写了 `plugin_alloc`/`plugin_free`/`plugin_dispatch` 三个 `#[no_mangle] extern "C"` 导出——参考它做特殊 ABI）。

6. **ABI 内存归属**（手写 ABI 时才需在意）：`plugin_dispatch` 的入参缓冲**归宿主**（宿主 alloc、读完响应后宿主 free），插件别释放它；`host_call` 的响应缓冲**归插件**（读完插件 free）。SDK 的 `rt::dispatch`/`call_host` 已处理好。

7. **Rust 测试里的非 ASCII**：`br#"...中文..."#` 字节串字面量编不过（must be ASCII）。测试要发含中文的 body 用 `r#"..."#.to_string().into_bytes()`。

8. **storage 插件必须返回真实 mtime**：`list_items` 的 `ItemStat.updated_time` 决定增量缓存；返回 0 合法但会导致每次全量拉取。WebDAV 从 `getlastmodified`、S3 从 `LastModified` 取。

9. **命令改写不回显编辑器**（before-save 也一样，除 note-toolbar command 外）：见 CLAUDE.md「before-save 改写不回显」——改写落 API 响应与磁盘，切走再切回笔记才见。而 `note-toolbar` command 的 `result.body` 会替换编辑缓冲（spec §9.4），这是例外。

## note-toolbar 命令约定（spec §9.4）

宿主以 `args = { note_id, title, body }`（当前编辑器内容）调用 `command`；若返回 `result` 含字符串字段 `body`，宿主用它替换编辑缓冲并走自动保存。命令按 Normal 档限额（网络等待经 host_call 豁免 CPU 墙钟）。前端按钮在编辑态工具栏出现（源码 / 实时预览均可）。

## 测试配方（四层，全在 CI 跑）

1. **native 单元**（`cargo test` 在插件目录）：把可测逻辑写成纯函数（请求组装、响应解析、config 规范化、签名算法）。**签名/协议类用已知答案测试**（jasper-plugins 的 `s3-storage` 用 AWS 官方 SigV4 向量）。host_call 在 native 下默认返回错误桩；要在 native 集成测试里真调 host，dev-dependencies 开 SDK 的 **`native-host`** feature（http→ureq 真网络、time.now→系统钟、settings 用 `sdk::native_host::set_setting` 注入——ai-polish 的 stub 全链路与 s3-storage 的 MinIO round-trip 都这么跑，无沙箱语义：没有能力门控/限额）。
2. **wasm 夹具**（server `--features plugins`）：需要 `plugin.wasm`，缺失时**自动跳过**（`if !plugin.wasm.exists() { eprintln!; return }`）。ABI 往返、限额、能力门控在这层。
3. **stub 服务**（command/HTTP 插件）：起一个极简 TCP server 返回固定响应，插件经 `host:http` 打过去——宿主侧模板是 `routes.rs::backend_command_end_to_end`（testbed 的 relay 命令：装→启用→存 secret→调命令→断言）；插件侧模板是 jasper-plugins 里 ai-polish 的 `native_e2e`（native-host + 本地 stub）。
4. **容器集成**（真实后端）：env-gated，未设环境变量则跳过。本仓库 `webdav-storage` 用 `JASPER_TEST_WEBDAV_URL`（hacdias/webdav，`docker compose -f docker-compose.dev.yml up -d`）；jasper-plugins 的 `s3-storage` 用 `JASPER_TEST_S3_URL`（MinIO，在那边的 compose/CI 里）。

宿主侧的 manifest 校验/路由测试写在 `server/src/plugins/{manifest,routes,storage}.rs` 的 `#[cfg(test)]`。

## 构建 · 打包 · 安装

```bash
# 需先 rustup target add wasm32-unknown-unknown
plugins-examples/build-wasm.sh                    # 全部示例编到 wasm + 拷 plugin.wasm 到 manifest 旁

# 打成可安装包（zip of manifest.toml + plugin.wasm）
cd plugins-examples && python3 -c "import zipfile; z=zipfile.ZipFile('my.jplug','w',zipfile.ZIP_DEFLATED); [z.write('my-plugin/'+f, f) for f in ('manifest.toml','plugin.wasm')]; z.close()"

# 跑起来试装（插件面板 → 安装 → 选 .jplug → 启用授权）
cd server && cargo run --features plugins           # http://127.0.0.1:27583/
```

`plugin.wasm` 与 `*.jplug` 都已 gitignore（构建产物）。默认构建（不带 `--features plugins`）完全不含插件宿主、零新依赖、行为不变——改动务必保证这一点（`cargo test` 默认 feature 必须绿）。

## 能力与安全底线

默认全拒，manifest `[backend].capabilities` 逐项申请，启用时向用户展示确认。`host:http` 会在授权弹窗显示联网警告。无裸 socket——SMB/NFS 这类裸 TCP 是**非目标**（OS 挂载 + 内置 local 数据源覆盖）。密钥用 `settings` 的 `secret` 类型存宿主，插件经 `settings_get` 读，前端永不回显。
