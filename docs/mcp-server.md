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

设置页的「添加到 Claude Code」一栏已经把命令拼好（含当前会话 token），复制粘贴即可。手写的话：

```bash
# 未设访问密码
claude mcp add --transport http jasper http://127.0.0.1:27583/mcp

# 设了访问密码：带上会话 token（登录后从设置页复制）
claude mcp add --transport http jasper http://127.0.0.1:27583/mcp \
  --header "Authorization: Bearer <token>"
```

其它客户端按各自的 Streamable HTTP 配置方式填 `http://<host>:<port>/mcp` 即可。

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

## 鉴权与只读：为什么门控在工具层

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

服务端不知道客户端是从哪个地址访问它的（`127.0.0.1`？局域网 IP？反代域名？），也不该知道
浏览器里的会话 token，所以它下发的是**模板**，留 `{origin}` / `{header}` 两个占位符由前端按
运行时实际情况填（`settingsSchema.ts::fillPlaceholders`）。展示型字段不回传给服务端
（`buildRequestBody` 会剔除）。

## 测试

- `server/src/api.rs`：`settings_schema_exposes_mcp_section`（描述符下发的是模板、不含 token、
  开关落库）、`mcp_endpoint_bypasses_method_guards_and_honors_switch`（只读+匿名下守卫不拦、
  关掉开关后 503）。均 `#[cfg(feature = "mcp")]`。
- `web/src/lib/settingsSchema.test.ts`：占位符填充、无 token 时不留空 Authorization 头、
  展示型字段不进请求体。
- 全链路手测（curl 走完整 Streamable HTTP 握手）覆盖：initialize → tools/list（13 个，
  annotations 正确）→ search/get/create/update/tag 往返 → 级联 delete_folder → 磁盘 Joplin
  格式核对 → 只读拒写放读 → 匿名拒写、带 token 放行。
