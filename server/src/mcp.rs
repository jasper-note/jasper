//! MCP server（Model Context Protocol，feature = "mcp"）。
//!
//! 把笔记库以 MCP 暴露给 Claude Code / Claude Desktop / Cursor 等客户端。传输用官方
//! Rust SDK（rmcp）的 **Streamable HTTP**，以 `nest_service` 挂在现有 axum router 的
//! [`crate::api::MCP_PATH`] 下——不另起进程、不另开端口，jasper 跑着就有 MCP。
//!
//! 本文件在**两种构建模式下都编译**：feature 关闭时只剩零成本桩（`router()` 返回空路由、
//! `TOOL_NAMES` 为空、`ENABLED` 为 false），默认构建不引入 rmcp/schemars 任何依赖。
//!
//! ## 为什么工具直接调 api.rs 的 handler
//!
//! 每个工具都是对现有 handler 的一层薄包装（手工构造 `State`/`Path`/`Json`/`Query`
//! 这些提取器——它们都是公开的 tuple struct，handler 本体就是普通 async fn）。
//! 这样可见范围过滤、before-save 钩子、事件广播、Joplin 字节格式全都是**同一份实现**，
//! HTTP 与 MCP 两条路径不会随时间漂移；代价只是 api.rs 里若干 `pub(crate)`。
//!
//! ## 门控为什么在工具层而不是中间件
//!
//! MCP 的 JSON-RPC 全压在一个路径上且**一律是 POST**（连 `tools/list` 都是），
//! 按 HTTP 方法拦截的 `guard_read_only` / `guard_auth` 对它一律失真。故两道守卫放行该路径
//! （见 [`crate::api::MCP_PATH`]），改由这里逐个工具门控：
//! - 读工具：把 `Access` 原样交给 handler，可见范围过滤照旧在 handler 里做；
//! - 写工具：先查全局只读（[`deny_read_only`]）、再要求 [`Access::Full`]（[`require_full`]）。

use crate::api::AppState;
use axum::Router;
use std::sync::Arc;

/// 本构建是否含 MCP server。前端设置页据此决定是否显示「MCP」段。
pub(crate) const ENABLED: bool = cfg!(feature = "mcp");

#[cfg(feature = "mcp")]
pub(crate) use imp::{router, TOOL_NAMES};

#[cfg(not(feature = "mcp"))]
pub(crate) fn router(_state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
}

/// feature 关闭时没有任何工具可列。
#[cfg(not(feature = "mcp"))]
pub(crate) const TOOL_NAMES: &[&str] = &[];

#[cfg(feature = "mcp")]
mod imp {
    use super::*;
    use crate::auth::Access;
    use axum::extract::{Path, Query, State};
    use axum::Extension;
    use axum::Json as AxumJson;
    use rmcp::handler::server::wrapper::{Json, Parameters};
    use rmcp::model::{ProtocolVersion, ServerCapabilities, ServerConfig};
    use rmcp::service::{RequestContext, RoleServer};
    use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService,
    };
    use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler};
    use schemars::JsonSchema;
    use serde::Deserialize;
    use serde_json::Value;
    use std::sync::atomic::Ordering;

    /// 暴露的工具名（设置页展示用，与下面 `#[tool]` 一一对应；改这里记得同步 impl）。
    pub(crate) const TOOL_NAMES: &[&str] = &[
        "search_notes",
        "get_note",
        "list_notes",
        "list_folders",
        "list_tags",
        "notes_by_tag",
        "create_note",
        "update_note",
        "create_folder",
        "add_note_tag",
        "remove_note_tag",
        "delete_note",
        "delete_folder",
    ];

    /// 搜索默认返回条数上限。笔记库动辄上百篇，全量倒进模型上下文既慢又不会被读完；
    /// 需要更多由调用方显式传 `limit`。
    const DEFAULT_SEARCH_LIMIT: usize = 30;

    // ---------- 工具入参 ----------

    #[derive(Deserialize, JsonSchema)]
    pub struct SearchArgs {
        /// 搜索词，匹配笔记标题与正文（不区分大小写）。
        pub query: String,
        /// 最多返回多少条，默认 30。
        pub limit: Option<usize>,
    }

    #[derive(Deserialize, JsonSchema)]
    pub struct NoteIdArgs {
        /// 笔记 id（32 位十六进制）。
        pub id: String,
    }

    #[derive(Deserialize, JsonSchema)]
    pub struct FolderIdArgs {
        /// 笔记本 id（32 位十六进制）。
        pub id: String,
    }

    #[derive(Deserialize, JsonSchema)]
    pub struct ListNotesArgs {
        /// 笔记本 id；留空或省略 = 未归属任何笔记本的笔记。
        pub folder_id: Option<String>,
    }

    #[derive(Deserialize, JsonSchema)]
    pub struct TagIdArgs {
        /// 标签 id（32 位十六进制，可由 list_tags 取得）。
        pub tag_id: String,
    }

    #[derive(Deserialize, JsonSchema)]
    pub struct CreateNoteArgs {
        /// 目标笔记本 id；留空 = 建在「未分类」。
        pub parent_id: Option<String>,
        /// 标题。
        pub title: String,
        /// 正文（Markdown）。
        pub body: Option<String>,
        /// 建成待办事项而非普通笔记。
        pub is_todo: Option<bool>,
    }

    #[derive(Deserialize, JsonSchema)]
    pub struct UpdateNoteArgs {
        /// 要更新的笔记 id。
        pub id: String,
        /// 新标题。
        pub title: String,
        /// 新正文（Markdown）。**整篇覆盖**，不是追加——先用 get_note 取回原文再改。
        pub body: String,
    }

    #[derive(Deserialize, JsonSchema)]
    pub struct CreateFolderArgs {
        /// 父笔记本 id；留空 = 建在顶层。
        pub parent_id: Option<String>,
        /// 笔记本名称。
        pub title: String,
    }

    #[derive(Deserialize, JsonSchema)]
    pub struct AddTagArgs {
        /// 笔记 id。
        pub note_id: String,
        /// 标签名。已存在则复用（trim + 不区分大小写），否则新建。
        pub title: String,
    }

    #[derive(Deserialize, JsonSchema)]
    pub struct RemoveTagArgs {
        /// 笔记 id。
        pub note_id: String,
        /// 要去掉的标签 id。
        pub tag_id: String,
    }

    // ---------- 门控与错误 ----------

    /// 取本次请求的访问级别。guard_auth 在外层 layer 里把它塞进了请求扩展，
    /// rmcp 又把 `http::request::Parts` 原样带进 `RequestContext`。
    /// 取不到说明中间件链被改动过——按最保守的匿名处理，不默认放行。
    fn access_of(ctx: &RequestContext<RoleServer>) -> Access {
        ctx.extensions
            .get::<axum::http::request::Parts>()
            .and_then(|p| p.extensions.get::<Access>().copied())
            .unwrap_or(Access::Anonymous)
    }

    /// 全局只读模式下拒绝写工具（对应 HTTP 侧 guard_read_only 的 403）。
    fn deny_read_only(state: &AppState) -> Result<(), ErrorData> {
        if state.read_only.load(Ordering::Relaxed) {
            return Err(ErrorData::invalid_request(
                "服务处于只读模式，写操作被拒绝（可在设置页关闭只读）",
                None,
            ));
        }
        Ok(())
    }

    /// 写工具要求已授权（对应 HTTP 侧 guard_auth 的 401）。
    /// 走到这里说明没配 MCP API key（配了的话 [`guard_mcp_access`] 已在更外层把未授权请求挡掉），
    /// 即实例设了访问密码而客户端没带有效凭证。
    fn require_full(access: Access) -> Result<(), ErrorData> {
        if access != Access::Full {
            return Err(ErrorData::invalid_request(
                "未授权：请在设置页 MCP 段生成 API key，并在客户端配置 `Authorization: Bearer <key>` 请求头",
                None,
            ));
        }
        Ok(())
    }

    /// handler 返回的 StatusCode 翻成人话——模型看得懂才好自我纠正。
    fn status_err(code: axum::http::StatusCode, what: &str) -> ErrorData {
        use axum::http::StatusCode as S;
        let msg = match code {
            S::NOT_FOUND => format!("{what}不存在"),
            S::BAD_REQUEST => format!("{what}的参数不合法"),
            S::SERVICE_UNAVAILABLE => "尚未配置数据源".to_string(),
            other => format!("{what}失败（{other}）"),
        };
        ErrorData::invalid_request(msg, None)
    }

    fn json_of<T: serde::Serialize>(v: T) -> Result<Json<Value>, ErrorData> {
        serde_json::to_value(v)
            .map(Json)
            .map_err(|e| ErrorData::internal_error(format!("序列化失败: {e}"), None))
    }

    // ---------- server ----------

    /// 每个 MCP 会话一个实例（由 `StreamableHttpService` 的工厂闭包构造）。
    /// 工具表由 `#[tool_router]` 生成为关联函数 `Self::tool_router()`，`#[tool_handler]`
    /// 直接调它，故这里不必把 ToolRouter 存成字段。
    #[derive(Clone)]
    pub struct JasperMcp {
        state: Arc<AppState>,
    }

    #[tool_router]
    impl JasperMcp {
        pub fn new(state: Arc<AppState>) -> Self {
            Self { state }
        }

        // ----- 读 -----

        #[tool(
            description = "全文搜索笔记，匹配标题与正文，按更新时间倒序返回摘要。想读正文再调 get_note。",
            annotations(title = "搜索笔记", read_only_hint = true)
        )]
        async fn search_notes(
            &self,
            Parameters(args): Parameters<SearchArgs>,
            ctx: RequestContext<RoleServer>,
        ) -> Result<Json<Value>, ErrorData> {
            let AxumJson(mut notes) = crate::api::search(
                State(self.state.clone()),
                Extension(access_of(&ctx)),
                Query(crate::api::SearchQuery { q: Some(args.query) }),
            )
            .await;
            notes.truncate(args.limit.unwrap_or(DEFAULT_SEARCH_LIMIT));
            json_of(notes)
        }

        #[tool(
            description = "按 id 读一篇笔记的完整内容（含正文 Markdown）。",
            annotations(title = "读取笔记", read_only_hint = true)
        )]
        async fn get_note(
            &self,
            Parameters(args): Parameters<NoteIdArgs>,
            ctx: RequestContext<RoleServer>,
        ) -> Result<Json<Value>, ErrorData> {
            let AxumJson(note) = crate::api::note_detail(
                State(self.state.clone()),
                Extension(access_of(&ctx)),
                Path(args.id),
            )
            .await
            .map_err(|c| status_err(c, "笔记"))?;
            json_of(note)
        }

        #[tool(
            description = "列出某个笔记本下的笔记摘要（不含正文），按更新时间倒序。",
            annotations(title = "列出笔记", read_only_hint = true)
        )]
        async fn list_notes(
            &self,
            Parameters(args): Parameters<ListNotesArgs>,
            ctx: RequestContext<RoleServer>,
        ) -> Result<Json<Value>, ErrorData> {
            let AxumJson(notes) = crate::api::notes_list(
                State(self.state.clone()),
                Extension(access_of(&ctx)),
                Query(crate::api::NotesQuery { folder: args.folder_id }),
            )
            .await;
            json_of(notes)
        }

        #[tool(
            description = "列出整棵笔记本树（含每个笔记本的直属笔记数）。id 为空串的节点是「未分类」。",
            annotations(title = "列出笔记本", read_only_hint = true)
        )]
        async fn list_folders(
            &self,
            ctx: RequestContext<RoleServer>,
        ) -> Result<Json<Value>, ErrorData> {
            let AxumJson(tree) =
                crate::api::folders(State(self.state.clone()), Extension(access_of(&ctx))).await;
            json_of(tree)
        }

        #[tool(
            description = "列出全部标签及各自的笔记篇数。",
            annotations(title = "列出标签", read_only_hint = true)
        )]
        async fn list_tags(
            &self,
            ctx: RequestContext<RoleServer>,
        ) -> Result<Json<Value>, ErrorData> {
            let AxumJson(tags) =
                crate::api::tags_list(State(self.state.clone()), Extension(access_of(&ctx))).await;
            json_of(tags)
        }

        #[tool(
            description = "列出打了某个标签的笔记摘要。标签 id 由 list_tags 取得。",
            annotations(title = "按标签列笔记", read_only_hint = true)
        )]
        async fn notes_by_tag(
            &self,
            Parameters(args): Parameters<TagIdArgs>,
            ctx: RequestContext<RoleServer>,
        ) -> Result<Json<Value>, ErrorData> {
            let AxumJson(notes) = crate::api::tag_notes(
                State(self.state.clone()),
                Extension(access_of(&ctx)),
                Path(args.tag_id),
            )
            .await;
            json_of(notes)
        }

        // ----- 写 -----

        #[tool(
            description = "新建一篇笔记，返回含 id 的完整笔记。",
            annotations(title = "新建笔记")
        )]
        async fn create_note(
            &self,
            Parameters(args): Parameters<CreateNoteArgs>,
            ctx: RequestContext<RoleServer>,
        ) -> Result<Json<Value>, ErrorData> {
            deny_read_only(&self.state)?;
            require_full(access_of(&ctx))?;
            let AxumJson(note) = crate::api::create_note(
                State(self.state.clone()),
                AxumJson(crate::api::CreateNoteReq {
                    parent_id: args.parent_id.unwrap_or_default(),
                    title: args.title,
                    body: args.body.unwrap_or_default(),
                    is_todo: args.is_todo.unwrap_or(false),
                }),
            )
            .await
            .map_err(|c| status_err(c, "新建笔记"))?;
            json_of(note)
        }

        #[tool(
            description = "整篇覆盖一条笔记的标题与正文（不是追加）。改之前先 get_note 取回原文。",
            annotations(title = "更新笔记")
        )]
        async fn update_note(
            &self,
            Parameters(args): Parameters<UpdateNoteArgs>,
            ctx: RequestContext<RoleServer>,
        ) -> Result<Json<Value>, ErrorData> {
            deny_read_only(&self.state)?;
            require_full(access_of(&ctx))?;
            let AxumJson(note) = crate::api::update_note(
                State(self.state.clone()),
                Path(args.id),
                AxumJson(crate::api::UpdateNoteReq { title: args.title, body: args.body }),
            )
            .await
            .map_err(|c| status_err(c, "笔记"))?;
            json_of(note)
        }

        #[tool(
            description = "新建一个笔记本，返回含 id 的引用。",
            annotations(title = "新建笔记本")
        )]
        async fn create_folder(
            &self,
            Parameters(args): Parameters<CreateFolderArgs>,
            ctx: RequestContext<RoleServer>,
        ) -> Result<Json<Value>, ErrorData> {
            deny_read_only(&self.state)?;
            require_full(access_of(&ctx))?;
            let AxumJson(folder) = crate::api::create_folder(
                State(self.state.clone()),
                AxumJson(crate::api::CreateFolderReq {
                    parent_id: args.parent_id.unwrap_or_default(),
                    title: args.title,
                }),
            )
            .await
            .map_err(|c| status_err(c, "新建笔记本"))?;
            json_of(folder)
        }

        #[tool(
            description = "给笔记打标签。标签名已存在则复用（不区分大小写），否则新建。返回该笔记当前的全部标签。",
            annotations(title = "打标签")
        )]
        async fn add_note_tag(
            &self,
            Parameters(args): Parameters<AddTagArgs>,
            ctx: RequestContext<RoleServer>,
        ) -> Result<Json<Value>, ErrorData> {
            deny_read_only(&self.state)?;
            require_full(access_of(&ctx))?;
            let AxumJson(tags) = crate::api::add_note_tag(
                State(self.state.clone()),
                Path(args.note_id),
                AxumJson(crate::api::AddTagReq { title: args.title }),
            )
            .await
            .map_err(|c| status_err(c, "打标签"))?;
            json_of(tags)
        }

        #[tool(
            description = "从笔记上去掉某个标签（标签本身保留）。返回该笔记剩余的标签。",
            annotations(title = "去标签")
        )]
        async fn remove_note_tag(
            &self,
            Parameters(args): Parameters<RemoveTagArgs>,
            ctx: RequestContext<RoleServer>,
        ) -> Result<Json<Value>, ErrorData> {
            deny_read_only(&self.state)?;
            require_full(access_of(&ctx))?;
            let AxumJson(tags) = crate::api::remove_note_tag(
                State(self.state.clone()),
                Path((args.note_id, args.tag_id)),
            )
            .await
            .map_err(|c| status_err(c, "去标签"))?;
            json_of(tags)
        }

        // ----- 删（不可撤销） -----

        #[tool(
            description = "永久删除一篇笔记。不可撤销，没有回收站。",
            annotations(title = "删除笔记", destructive_hint = true)
        )]
        async fn delete_note(
            &self,
            Parameters(args): Parameters<NoteIdArgs>,
            ctx: RequestContext<RoleServer>,
        ) -> Result<Json<Value>, ErrorData> {
            deny_read_only(&self.state)?;
            require_full(access_of(&ctx))?;
            let code = crate::api::delete_note(State(self.state.clone()), Path(args.id)).await;
            if code != axum::http::StatusCode::NO_CONTENT {
                return Err(status_err(code, "删除笔记"));
            }
            json_of(serde_json::json!({ "deleted": true }))
        }

        #[tool(
            description = "永久删除一个笔记本，**连同它的子笔记本与其中所有笔记一起删除**。\
                           不可撤销。删之前建议先用 list_folders / list_notes 确认会删掉哪些内容。",
            annotations(title = "删除笔记本（级联）", destructive_hint = true)
        )]
        async fn delete_folder(
            &self,
            Parameters(args): Parameters<FolderIdArgs>,
            ctx: RequestContext<RoleServer>,
        ) -> Result<Json<Value>, ErrorData> {
            deny_read_only(&self.state)?;
            require_full(access_of(&ctx))?;
            let code = crate::api::delete_folder(State(self.state.clone()), Path(args.id)).await;
            if code != axum::http::StatusCode::NO_CONTENT {
                return Err(status_err(code, "删除笔记本"));
            }
            json_of(serde_json::json!({ "deleted": true }))
        }
    }

    #[tool_handler]
    impl ServerHandler for JasperMcp {
        fn get_info(&self) -> ServerConfig {
            // ServerConfig 与内层 Implementation 都是 #[non_exhaustive]，只能先取默认值再逐字段改。
            let mut info = ServerConfig::default();
            info.protocol_version = ProtocolVersion::LATEST;
            info.capabilities = ServerCapabilities::builder().enable_tools().build();
            info.server_info.name = "jasper".into();
            info.server_info.version = env!("CARGO_PKG_VERSION").into();
            info.instructions = Some(
                "Jasper 是一个 Joplin 兼容的笔记库。笔记与笔记本的 id 都是 32 位十六进制字符串；\
                 正文是 Markdown。笔记本 id 为空串代表「未分类」。\
                 先用 search_notes / list_folders 定位，再用 get_note 读正文；\
                 update_note 是整篇覆盖，改之前务必先读回原文。\
                 删除类工具不可撤销，执行前请向用户确认。"
                    .into(),
            );
            info
        }
    }

    /// MCP 端点的准入：运行时开关 + 独立 API key。只挂在 MCP 子 router 上，不影响其它路由。
    /// router 只在启动时构建一次，故两者都必须在**请求时**查，不能在构建时分支。
    ///
    /// API key 是给 MCP **单独上的一把锁**，与浏览器会话解耦：
    /// - 设了 key → `/mcp` 只认这把 key，带错/不带一律 401，**会话 token 也不行**。
    ///   这样「配了 key」才等于「只有拿钥匙的进得来」——否则未设访问密码时设了 key 等于没设。
    ///   校验通过即把 [`Access::Full`] 覆盖进请求扩展（guard_auth 先跑、给的是 Anonymous），
    ///   工具层照常从扩展里取，无需感知 key 的存在。
    /// - 没设 key → 维持原行为：guard_auth 算出的 Access 照旧（会话 token / 未设密码时恒 Full）。
    ///
    /// 万一 key 丢了也锁不死自己：设置页走的是普通的 `/api/mcp/config`，用浏览器身份即可重置。
    async fn guard_mcp_access(
        axum::extract::State(state): axum::extract::State<Arc<AppState>>,
        mut req: axum::extract::Request,
        next: axum::middleware::Next,
    ) -> axum::response::Response {
        use axum::response::IntoResponse as _;
        let (enabled, key) = {
            let cfg = state.config.lock().unwrap();
            (cfg.mcp_enabled(), cfg.mcp_api_key())
        };
        if !enabled {
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                AxumJson(serde_json::json!({
                    "error": "mcp_disabled",
                    "message": "MCP server 已在设置页关闭"
                })),
            )
                .into_response();
        }
        if !key.is_empty() {
            let ok = crate::api::bearer_token(req.headers())
                .map(|t| crate::auth::secret_eq(&t, &key))
                .unwrap_or(false);
            if !ok {
                tracing::warn!("mcp request rejected: missing or wrong api key");
                return (
                    axum::http::StatusCode::UNAUTHORIZED,
                    AxumJson(serde_json::json!({
                        "error": "mcp_unauthorized",
                        "message": "MCP API key 不正确：请在客户端配置 `Authorization: Bearer <key>` 请求头（可在设置页 MCP 段复制完整命令）"
                    })),
                )
                    .into_response();
            }
            req.extensions_mut().insert(Access::Full);
        }
        next.run(req).await
    }

    /// 把 MCP 端点挂进 axum router。service 自带 state（闭包捕获），故不吃 axum 的 State。
    pub(crate) fn router(state: Arc<AppState>) -> Router<Arc<AppState>> {
        let svc_state = state.clone();
        let service = StreamableHttpService::new(
            move || Ok(JasperMcp::new(svc_state.clone())),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
        );
        Router::new()
            .nest_service(crate::api::MCP_PATH, service)
            .layer(axum::middleware::from_fn_with_state(state, guard_mcp_access))
    }
}
