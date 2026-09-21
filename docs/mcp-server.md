# Jasper 作为 MCP server

把笔记库以 [Model Context Protocol](https://modelcontextprotocol.io) 暴露出去，让 Claude Code、
Claude Desktop、Cursor 等客户端直接搜索、读写你的 Joplin 笔记。

实现在 `server/src/mcp.rs`（feature = `mcp`），传输用官方 Rust SDK [rmcp](https://docs.rs/rmcp)
的 **Streamable HTTP**，以 `nest_service` 挂在现有 axum router 的 `/mcp` 下——不另起进程、
不另开端口，jasper 跑着就有 MCP。

## 构建与开启

```bash
cd server && cargo build --release --features mcp        # 或 embed,plugins,mcp
```

默认构建**不含** MCP，也不引入 rmcp/schemars 任何依赖（对齐 `plugins`/`embed` 的做法）。
Docker 镜像已带 `mcp`。

编进去之后，MCP 默认**开启**（能编出来本身已是一次显式选择），可在 **设置 › MCP** 里随时关掉——
关掉后端点立即返回 `503 {"error":"mcp_disabled"}`。开关存在 `config.db`（`PUT /api/mcp/config`）。

## 接上客户端

在 **设置 › MCP** 里点「生成 API Key」，然后复制「添加到 Claude Code」那一栏——命令已经把
端点和密钥都拼好了：

```bash
claude mcp add --transport http jasper http://127.0.0.1:27583/mcp \
  --header "Authorization: Bearer jasper_mcp_<64hex>"
```

其它客户端按各自的 Streamable HTTP 配置方式填 `http://<host>:<port>/mcp` 即可，密钥同样走
`Authorization: Bearer` 头。

## API key

MCP 有**自己的一把长效密钥**，与浏览器登录状态完全解耦。这是必要的：会话 token 存在内存
（`auth.rs` 的 `sessions: RwLock<HashSet<String>>`），服务重启即失效、浏览器点登出会吊销、
改密码会全清——浏览器里重登只是输个密码，但 MCP 客户端的请求头是写死在配置文件里的，
每次失效都得手动更新，不可用。

| | 会话 token | MCP API key |
|---|---|---|
| 存储 | 内存 `HashSet` | `config.db`（明文） |
| 服务重启 | 失效 | **存活** |
| 浏览器登出 / 改密码 | 失效 | **存活** |
| 用途 | 浏览器前端 | 仅 `/mcp` 端点 |

- **形状**：`jasper_mcp_` + 256 bit 随机 hex（`auth::gen_mcp_key`）。前缀让它在配置文件、
  进程列表、日志里一眼可辨，也方便将来做密钥扫描。
- **明文存**：与 `webdav_pass`、AI `api_key` 同一口径。用户必须能在设置页看到并复制它，
  哈希存就再也拿不回来了。读它的 `/api/settings/schema` 已是机密读（匿名 401）。
- **比较用常数时间**（`auth::secret_eq`），不按字节短路。
- **一旦设置，`/mcp` 只认它**——不带、带错、甚至拿一枚**有效的浏览器会话 token**，都是 401。
  否则「配了 key」不等于「只有拿钥匙的进得来」：实例没设访问密码时，设了 key 等于没设。
- **没设 key** → 回落既有行为（会话 token，或未设访问密码时恒 `Full`）。
- **锁不死自己**：管理走的是普通的 `PUT /api/mcp/config`（受只读 + 鉴权守卫约束，
  匿名 401、只读 403），用浏览器身份就能重新生成或清除。
- 日志只记「生成/清除了」，**绝不记 key 本身**。

设置页提供 生成 / 重新生成（旧 key 立即失效）/ 清除 三个动作。

## 工具

13 个，全部是对既有 HTTP handler 的薄包装（见下「为什么不另写一层」）。

| 工具 | 作用 | 标注 |
|---|---|---|
| `search_notes` | 全文搜索标题与正文，默认返回前 30 条摘要 | 只读 |
| `get_note` | 按 id 读一篇笔记的完整内容（含正文） | 只读 |
| `list_notes` | 列某笔记本下的笔记摘要 | 只读 |
| `list_folders` | 整棵笔记本树（含篇数） | 只读 |
| `list_tags` | 全部标签及篇数 | 只读 |
| `notes_by_tag` | 打了某标签的笔记 | 只读 |
| `create_note` | 新建笔记 | 写 |
| `update_note` | **整篇覆盖**标题与正文 | 写 |
| `create_folder` | 新建笔记本 | 写 |
| `add_note_tag` | 打标签（已有则复用，不区分大小写） | 写 |
| `remove_note_tag` | 去标签（标签本身保留） | 写 |
| `delete_note` | 永久删除一篇笔记 | 破坏性 |
| `delete_folder` | 永久删除笔记本，**级联删子笔记本与其中所有笔记** | 破坏性 |

破坏性工具带 `destructiveHint`，客户端据此会弹确认；`get_info()` 的 instructions 里也写明了
「删除类工具不可撤销，执行前请向用户确认」。

资源（图片/附件）的上传下载没有做成工具——二进制走 MCP 不划算，需要时用 `/api/resources`。

## 鉴权与只读：三道闸

```
MCP 客户端  ──Authorization: Bearer <key>──▶
  guard_auth（最外层，api.rs）
    定 Access::Full / Anonymous，塞进请求扩展；对 /mcp 只定不拦
  guard_mcp_access（仅挂在 MCP 子 router，mcp.rs）
    ① 开关关了 → 503
    ② 配了 API key 且不匹配 → 401（会话 token 也不行）
       匹配 → 用 Access::Full 覆盖请求扩展
  工具层
    写/删：deny_read_only() → require_full()
    读：  把 Access 交给原 handler，继承它的 Scope 过滤
```

只读模式与身份正交：`deny_read_only` 先于 `require_full`，所以只读时哪怕带着正确的 key
也写不了，但搜索/读取照常。

### 为什么门控在工具层

MCP 的 JSON-RPC **全压在 `/mcp` 一个路径上，且一律是 POST**（连 `tools/list` 都是）。
而 jasper 原有的两道守卫都是**按 HTTP 方法**拦截的：

- `guard_read_only`：只读模式下拦一切 POST/PUT/DELETE/PATCH
- `guard_auth`：匿名时拦一切写方法

直接挂进去的话，只读模式会让 MCP **整个不可用**（连工具都列不出），设了访问密码时匿名也一样。
这显然不对：只读应该只挡写工具，不该挡搜索。

所以两道守卫都**放行 `/mcp`**（`api.rs::is_mcp_path`），门控下沉到每个工具：

- **读工具**：把 `Access` 原样传给 handler，可见范围（`Scope`：无密码阅读开关 + 笔记本黑白名单）
  过滤照旧在 handler 里做，与 HTTP 完全一致。
- **写工具**：先 `deny_read_only`（对应 HTTP 的 403），再 `require_full`（对应 HTTP 的 401）。

守卫仍然会照常算出 `Access` 塞进请求扩展，rmcp 把 `http::request::Parts` 原样带进
`RequestContext`，工具从那里取。取不到时按**匿名**处理（保守，不默认放行）。

> 注意 `PUT /api/mcp/config`（开关本身）是普通的 `/api/*` 写端点，**不**在豁免之列——
> 只读态不可改、匿名不可改，与其它设置一致。

## 为什么不另写一层业务逻辑

每个工具都是对 `api.rs` 里现有 handler 的薄包装：axum 的提取器（`State`/`Path`/`Json`/`Query`/
`Extension`）都是可手工构造的 tuple struct，handler 本体就是普通 async fn，直接调即可。

这样可见范围过滤、before-save 插件钩子、SSE 事件广播、Joplin 字节格式全都是**同一份实现**，
HTTP 与 MCP 两条路径不会随时间漂移。代价只是 api.rs 里若干 `pub(crate)`。

替代方案（在 MCP 层重写一遍读写逻辑）会立刻引入两套语义，以后每改一处都要记得改两遍。

## 设置页

MCP 段走既有的 server-driven 设置描述符（`GET /api/settings/schema`），仅在
`--features mcp` 构建里出现。为此给字段词汇加了两个展示型类型：

- `copy` —— 只读 + 一键复制（端点地址、`claude mcp add` 命令）
- `note` —— 纯展示文本（工具清单）

服务端不知道客户端是从哪个地址访问它的（`127.0.0.1`？局域网 IP？反代域名？），所以端点地址
下发的是**模板**，留 `{origin}` 占位符由前端填（`settingsSchema.ts::fillPlaceholders`）。

认证头那段则分两种：配了 MCP API key 时由服务端直接拼死（key 本就存在服务端，这个描述符又是
机密读）；没配才留 `{header}` 占位符让前端填浏览器会话 token——后者服务端不知道也不该知道。

另外给 `show_if` 补了 `truthy: false` 语义（「没值时才显示」），用来在未生成 key 时显示
「生成 API Key」、已生成时换成「重新生成 / 清除」。动作 `on_success: "reload-section"` 让
前端重拉描述符并重挂该分区，把服务端新生成的 key 回显出来。展示型字段不回传给服务端
（`buildRequestBody` 会剔除）。

## 测试

- `server/src/auth.rs`：`mcp_key_shape_and_constant_time_compare`（前缀/长度/每次新随机、
  `secret_eq` 对截短与等长不同串都判否）。
- `server/src/api.rs`（均 `#[cfg(feature = "mcp")]`）：
  - `settings_schema_exposes_mcp_section` —— 描述符下发的是模板、不含 token、开关落库
  - `mcp_api_key_locks_the_endpoint_independently_of_sessions` —— 生成后描述符回显并拼好命令；
    不带/带错/**拿有效会话 token** 都 401；`revoke_all()`（等价于重启）后 key 照常可用；
    重新生成使旧 key 立即失效；清除后回落既有行为；`regenerate+clear` 同时传 → 400
  - `mcp_config_endpoint_is_guarded_like_any_other_setting` —— 管理端点匿名 401、只读 403
  - `mcp_endpoint_bypasses_method_guards_and_honors_switch` —— 只读+匿名下守卫不拦、关掉后 503
- `web/src/lib/settingsSchema.test.ts`：占位符填充、无 token 时不留空 Authorization 头、
  `truthy:false` 显隐、展示型字段不进请求体。
- 全链路手测（curl 走完整 Streamable HTTP 握手）覆盖：initialize → tools/list（13 个，
  annotations 正确）→ search/get/create/update/tag 往返 → 级联 delete_folder → 磁盘 Joplin
  格式核对 → 只读拒写放读 → 匿名拒写、带 token 放行 → 生成 key 后 401/401/200 三态 →
  **重启服务后同一把 key 依然可用**。
