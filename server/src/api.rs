//! HTTP API 层（axum）。
//!
//! 配置：
//!   GET    /api/status           是否已配置 + 计数 + 数据源类型
//!   GET    /api/config           当前配置
//!   PUT    /api/config           设置/切换数据源（连接并重建索引）
//! 读：
//!   GET    /api/folders          笔记本树
//!   POST   /api/folders          新建笔记本 { parent_id?, title }
//!   PUT    /api/folders/{id}     重命名笔记本 { title }
//!   PUT    /api/folders/{id}/move 移动笔记本（改 parent_id；防环）
//!   DELETE /api/folders/{id}     删除笔记本（级联：子笔记本 + 其中全部笔记）
//!   PUT    /api/mcp/config       MCP server 开关 { enabled }（仅 --features mcp 构建）
//!   /mcp                          MCP server 端点（Streamable HTTP；见 mcp.rs 与 MCP_PATH）
//!   GET    /api/notes?folder=ID  笔记列表
//!   GET    /api/notes/{id}       笔记详情
//!   GET    /api/tags             标签列表（含篇数）
//!   GET    /api/tags/{id}/notes  某标签下的笔记
//!   GET    /api/notes/{id}/tags  某笔记的标签
//! 写：
//!   POST   /api/notes/{id}/tags        给笔记打标签 { title }（标签不存在则新建，兼容 Joplin）
//!   DELETE /api/notes/{id}/tags/{tid}  从笔记去掉某标签（删 note_tag 关联，保留标签本身）
//!   GET    /api/resources        资源清单（含引用计数）
//!   GET    /api/resources/{id}   资源二进制
//!   GET    /api/search?q=...     全文搜索
//!   GET    /api/events           SSE 变更流（事件 `change`：{kind, op, id}；前端按需刷新）
//! 写：
//!   POST   /api/notes            新建笔记
//!   PUT    /api/notes/{id}       更新笔记
//!   PUT    /api/notes/{id}/move  移动笔记到另一笔记本（改 parent_id）
//!   DELETE /api/notes/{id}       删除笔记
//!   POST   /api/resources        上传资源（二进制为体），写 .resource/<id> + <id>.md
//!   PUT    /api/resources/{id}   重命名资源
//!   DELETE /api/resources/{id}   删除资源（二进制 + 元数据）

use crate::auth::{Access, AuthState, Scope};
use crate::config::{self, AuthConfig, ConfigStore, SourceConfig};
use crate::library::Library;
use crate::model::{MarkupLanguage, Note};
use crate::serialize;
use crate::storage::StorageBackend;
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{header, HeaderMap, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

pub struct AppState {
    /// Arc 化以便把引用递进插件 dispatch（wasmi Store 拥有 ctx、spawn_blocking 要 'static）。
    pub library: Arc<RwLock<Library>>,
    pub storage: RwLock<Option<Arc<dyn StorageBackend>>>,
    /// Arc 化以便与 PluginHost 共享同一配置库连接。
    pub config: Arc<Mutex<ConfigStore>>,
    pub cache: crate::cache::CacheStore,
    /// 只读模式：为真时中间件拒绝一切写方法（/api/config 除外）。运行时可切换。
    pub read_only: AtomicBool,
    /// 访问鉴权（access control）：访问密码 / 无密码阅读 / 黑白名单 / 内存会话 token。
    pub auth: AuthState,
    /// 插件宿主（--features plugins；关闭或初始化失败时为 None）。
    pub plugins: Option<Arc<crate::plugins::PluginHost>>,
    /// 变更事件总线（SSE /api/events）：一切写路径在此广播，前端按需刷新。
    pub events: crate::events::EventBus,
}

fn storage_of(state: &Arc<AppState>) -> Option<Arc<dyn StorageBackend>> {
    state.storage.read().unwrap().clone()
}

pub fn router(state: Arc<AppState>) -> Router {
    use tower_http::cors::CorsLayer;
    use tower_http::trace::TraceLayer;
    Router::new()
        .route("/api/status", get(status))
        .route("/api/config", get(get_config).put(apply_config))
        // 服务器驱动的设置描述符（分区目录 + 字段 schema + 当前值 + 动作）：前端通用渲染器据此渲染设置面板
        .route("/api/settings/schema", get(settings_schema))
        // UI 语言持久化（前端切换/启动时写）：插件经免能力 host_call `system.locale` 读同一值
        .route("/api/locale", get(get_locale).put(put_locale))
        // MCP server 运行时开关（仅 `--features mcp` 构建有意义，否则 404）
        .route("/api/mcp/config", put(put_mcp_config))
        // 访问鉴权（access control）：登录取 token / 登出 / 读写访问控制设置
        .route("/api/auth/login", post(auth_login))
        .route("/api/auth/logout", post(auth_logout))
        .route("/api/auth/settings", get(get_auth_settings).put(put_auth_settings))
        .route("/api/folders", get(folders).post(create_folder))
        .route("/api/folders/{id}", put(rename_folder).delete(delete_folder))
        .route("/api/folders/{id}/move", put(move_folder))
        .route("/api/notes", get(notes_list).post(create_note))
        .route("/api/notes/{id}", get(note_detail).put(update_note).delete(delete_note))
        .route("/api/notes/{id}/move", put(move_note))
        .route("/api/tags", get(tags_list))
        .route("/api/tags/{id}/notes", get(tag_notes))
        .route("/api/notes/{id}/tags", get(note_tags_list).post(add_note_tag))
        .route("/api/notes/{id}/tags/{tag_id}", delete(remove_note_tag))
        .route("/api/resources", get(list_resources).post(upload_resource))
        .route("/api/resources/{id}", get(resource).put(rename_resource).delete(delete_resource))
        .route("/api/search", get(search))
        .route("/api/events", get(events_sse))
        // 插件管理路由（feature off = 空路由）。挂在只读/鉴权守卫之内 → 只读/未授权时插件写操作同样被拦。
        .merge(crate::plugins::api_router())
        // MCP server（feature off = 空路由）。同样挂在守卫之内 —— 守卫会照常算出 Access 塞进扩展，
        // 只是对这个路径不按 HTTP 方法拦截，门控下沉到各工具（见 MCP_PATH 与 mcp.rs）。
        .merge(crate::mcp::router(state.clone()))
        // 只读守卫：只读模式下拦截一切写方法（/api/config 与 /api/auth/* 除外）。放在最内层，
        // 保证它能拿到 State 且早于任何 handler 运行。
        .layer(axum::middleware::from_fn_with_state(state.clone(), guard_read_only))
        // 鉴权守卫：定 Access（塞进请求扩展供 handler 读）+ 拦未授权的写/机密读。
        // 加在只读守卫之后 → 它更外层、先运行：认证优先于只读（未授权写得 401 而非 403）。
        .layer(axum::middleware::from_fn_with_state(state.clone(), guard_auth))
        // 资源上传可能较大（图片/附件），放宽请求体上限到 100MB
        .layer(DefaultBodyLimit::max(100 * 1024 * 1024))
        .layer(CorsLayer::permissive())
        // 每个请求一条 method+path+status+耗时 的结构化日志（tower_http=debug 时可见，见 main.rs 默认过滤）
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// MCP 端点（Streamable HTTP）。JSON-RPC 全部压在这一个路径上，**连列工具都是 POST**，
/// 所以按 HTTP 方法拦截的两道守卫对它一律失真——只读时会连 `tools/list` 都 403，
/// 设了访问密码时匿名连工具清单都列不出。故两道守卫都放行这个路径，
/// 改由工具层逐个门控（写工具查只读与 [`Access`]，读工具按 [`Scope`] 过滤），见 `mcp.rs`。
pub(crate) const MCP_PATH: &str = "/mcp";

fn is_mcp_path(path: &str) -> bool {
    // StreamableHttpService 以 nest_service 挂载，其下还会有子路径
    path == MCP_PATH || path.starts_with("/mcp/")
}

/// 只读强制拦截（核心安全保证）：只读开启时，凡写方法（POST/PUT/DELETE/PATCH）一律 403，
/// 例外是 `/api/config`（否则无法在设置页把只读关回去）、`/api/auth/*`（否则只读态无法登录/改密码）
/// 与 `/mcp`（见 [`MCP_PATH`]，改由工具层门控）。
/// 读方法（GET/HEAD/OPTIONS）放行。按 HTTP 方法集中拦截 → 将来新增任何写路由自动被覆盖。
async fn guard_read_only(State(state): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let is_write = matches!(
        *req.method(),
        Method::POST | Method::PUT | Method::DELETE | Method::PATCH
    );
    let exempt = matches!(
        req.uri().path(),
        // `/api/locale` 是 UI 偏好而非库数据：只读态也允许 owner 持久化语言（鉴权守卫照常）。
        "/api/config" | "/api/locale" | "/api/auth/login" | "/api/auth/logout" | "/api/auth/settings"
    ) || is_mcp_path(req.uri().path());
    if is_write && !exempt && state.read_only.load(Ordering::Relaxed) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "read_only", "message": "服务处于只读模式，写操作被拒绝" })),
        )
            .into_response();
    }
    next.run(req).await
}

/// 从 `Authorization: Bearer <token>` 头取会话 token（MCP 用同一个头传它的 API key）。
pub(crate) fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let t = v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer "))?.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// 从 `GET /api/events?token=<会话 token>` 的查询串取会话 token。
///
/// `EventSource` 不能自定义请求头（浏览器 API 限制），SSE 订阅因此带不了 `Authorization`，
/// 会被一律判成 [`Access::Anonymous`]——受限匿名下每条事件都被粗化成 library reload，
/// 前端只能全量重载（编辑中的笔记被销毁重建）。仅给这一个只读端点开查询串取 token 的口子，
/// 其余端点照旧只认请求头，避免 token 落进日志与浏览器历史。
fn sse_query_token(req: &Request) -> Option<String> {
    if req.uri().path() != "/api/events" {
        return None;
    }
    // 会话 token 是 hex 串（见 auth.rs），无需百分号解码。
    let t = req.uri().query()?.split('&').find_map(|kv| kv.strip_prefix("token="))?;
    (!t.is_empty()).then(|| t.to_string())
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({ "error": "unauthorized", "message": "需要登录：请先输入访问密码" })),
    )
        .into_response()
}

/// 鉴权守卫（access control）：
/// 1. 据 Bearer token 定 [`Access`] 并塞进请求扩展，供各 handler 按可见范围过滤内容
///    （`GET /api/events` 额外认 `?token=`，见 [`sse_query_token`]）。
/// 2. 未授权（Anonymous）时：拦一切写方法（`/api/auth/login|logout` 除外——登录本身要能打），
///    并拦会泄露机密的读端点（`/api/config`、`/api/ai/config`、`GET /api/auth/settings`、
///    `GET /api/resources` 资源清单会暴露私有资源标题）。
/// 未设访问密码时 Access 恒 Full → 以下判断都不触发，行为与无鉴权时完全一致（向后兼容）。
async fn guard_auth(State(state): State<Arc<AppState>>, mut req: Request, next: Next) -> Response {
    let token = bearer_token(req.headers()).or_else(|| sse_query_token(&req));
    let access = state.auth.access_for(token.as_deref());
    req.extensions_mut().insert(access);

    if access != Access::Full {
        let path = req.uri().path();
        let is_write = matches!(
            *req.method(),
            Method::POST | Method::PUT | Method::DELETE | Method::PATCH
        );
        // 登录/登出不受写守卫拦（登录时还没 token）。注意 PUT /api/auth/settings 不在此列 →
        // 匿名改访问控制设置会被下面拦成 401（未设密码时 Access=Full 才放行首次设置）。
        // `/mcp` 同样放行到工具层：Access 已塞进扩展，读工具按 Scope 过滤、写工具自查 Full，
        // 否则匿名连 tools/list 都列不出（见 MCP_PATH 注释）。
        let write_exempt = matches!(path, "/api/auth/login" | "/api/auth/logout") || is_mcp_path(path);
        // 机密读端点（`/api/resources` 只匹配清单本身，`/api/resources/{id}` 二进制不在内）。
        // `/api/settings/schema` 回显 webdav 密码 / AI key / 私有笔记本名单 → 匿名不可读。
        let secret_read = matches!(
            path,
            "/api/config"
                | "/api/ai/config"
                | "/api/auth/settings"
                | "/api/resources"
                | "/api/settings/schema"
        );
        if is_write && !write_exempt {
            return unauthorized();
        }
        if !is_write && secret_read {
            return unauthorized();
        }
    }
    next.run(req).await
}

// ---------- DTO ----------

#[derive(Serialize)]
struct StatusResp {
    configured: bool,
    source_type: String,
    notes: usize,
    folders: usize,
    read_only: bool,
    /// 是否设了访问密码（受保护）。
    auth_enabled: bool,
    /// 本请求是否已登录（携带有效会话 token）。
    authenticated: bool,
    /// 允许无密码阅读总开关（前端据此提示匿名可见范围）。
    passwordless_read: bool,
    /// 服务端版本（市场 UI 拿它做 minHostVersion 兼容过滤）
    version: &'static str,
}

#[derive(Serialize)]
struct ConfigResult {
    ok: bool,
    error: Option<String>,
    notes: usize,
    folders: usize,
}

impl ConfigResult {
    fn err(e: impl std::fmt::Display) -> Self {
        Self { ok: false, error: Some(e.to_string()), notes: 0, folders: 0 }
    }
}

#[derive(Deserialize)]
struct ApplyConfigReq {
    source_type: String,
    #[serde(default)]
    local_path: String,
    #[serde(default)]
    webdav_url: String,
    #[serde(default)]
    webdav_user: String,
    #[serde(default)]
    webdav_pass: String,
    /// source_type=="plugin"（插件存储 provider，spec 0.2）
    #[serde(default)]
    plugin_id: String,
    #[serde(default)]
    plugin_storage: String,
    #[serde(default)]
    plugin_config: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    read_only: bool,
    #[serde(default)]
    create_new: bool,
}

// 以下 DTO 与读写 handler 标 pub(crate)：MCP 工具层（mcp.rs）直接复用这一份 handler，
// 而不是另写一套业务逻辑——可见范围过滤 / before-save 钩子 / 事件广播因此天然一致，
// 不会随时间漂移。输出 DTO 只需类型可见（MCP 侧只做 Serialize），入参 DTO 的字段也要可见（要构造）。
#[derive(Serialize)]
pub(crate) struct FolderNode {
    id: String,
    title: String,
    note_count: usize,
    children: Vec<FolderNode>,
}

#[derive(Serialize)]
pub(crate) struct NoteSummary {
    id: String,
    title: String,
    updated_time: i64,
    parent_id: String,
    is_todo: bool,
    todo_completed: bool,
    /// 正文内 markdown 任务清单的完成/总数（总数 0 = 无任务清单）。
    task_done: usize,
    task_total: usize,
}

#[derive(Serialize)]
pub(crate) struct NoteDetail {
    id: String,
    title: String,
    body: String,
    markup_language: i32, // 1=Markdown, 2=HTML
    parent_id: String,
    created_time: i64,
    updated_time: i64,
    source_url: String,
    is_todo: bool,
    todo_completed: bool,
}

#[derive(Deserialize)]
pub(crate) struct UpdateNoteReq {
    pub(crate) title: String,
    pub(crate) body: String,
}

#[derive(Deserialize)]
struct MoveNoteReq {
    parent_id: String,
}

#[derive(Deserialize)]
pub(crate) struct CreateNoteReq {
    pub(crate) parent_id: String,
    #[serde(default)]
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) body: String,
    /// 是否建为待办（is_todo: 1）。
    #[serde(default)]
    pub(crate) is_todo: bool,
}

#[derive(Deserialize)]
pub(crate) struct CreateFolderReq {
    #[serde(default)]
    pub(crate) parent_id: String, // 空 = 根
    #[serde(default)]
    pub(crate) title: String,
}

#[derive(Deserialize)]
struct MoveFolderReq {
    parent_id: String, // 空 = 移到根
}

#[derive(Deserialize)]
struct RenameFolderReq {
    title: String,
}

#[derive(Serialize)]
pub(crate) struct FolderRef {
    id: String,
    title: String,
    parent_id: String,
}

fn summarize(n: &Note) -> NoteSummary {
    let (task_done, task_total) = crate::library::count_tasks(&n.body);
    NoteSummary {
        id: n.id.clone(),
        title: n.title.clone(),
        updated_time: n.updated_time,
        parent_id: n.parent_id.clone(),
        is_todo: n.is_todo,
        todo_completed: n.todo_completed,
        task_done,
        task_total,
    }
}

fn detail_of(n: &Note) -> NoteDetail {
    NoteDetail {
        id: n.id.clone(),
        title: n.title.clone(),
        body: n.body.clone(),
        markup_language: match n.markup_language {
            MarkupLanguage::Markdown => 1,
            MarkupLanguage::Html => 2,
        },
        parent_id: n.parent_id.clone(),
        created_time: n.created_time,
        updated_time: n.updated_time,
        source_url: n.source_url.clone(),
        is_todo: n.is_todo,
        todo_completed: n.todo_completed,
    }
}

// ---------- 配置 handlers ----------

async fn status(
    State(state): State<Arc<AppState>>,
    Extension(access): Extension<Access>,
) -> Json<StatusResp> {
    let configured = state.storage.read().unwrap().is_some();
    let (notes, folders) = {
        let lib = state.library.read().unwrap();
        (lib.notes.len(), lib.folders.len())
    };
    let source_type = state
        .config
        .lock()
        .unwrap()
        .load()
        .map(|c| c.source_type)
        .unwrap_or_default();
    let read_only = state.read_only.load(Ordering::Relaxed);
    let auth_enabled = state.auth.enabled();
    // 已登录 = 设了密码且本请求为 Full（未设密码时 Full 不算「已登录」）。
    let authenticated = auth_enabled && matches!(access, Access::Full);
    let passwordless_read = state.auth.passwordless_read();
    Json(StatusResp {
        configured,
        source_type,
        notes,
        folders,
        read_only,
        auth_enabled,
        authenticated,
        passwordless_read,
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[derive(Deserialize)]
struct LocaleReq {
    locale: String,
}

/// GET /api/locale —— 当前持久化的 UI 语言（插件经 host_call `system.locale` 读同一值）。
async fn get_locale(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let locale = state.config.lock().unwrap().ui_locale();
    Json(serde_json::json!({ "locale": locale }))
}

/// PUT /api/locale —— 前端切换/启动时持久化 UI 语言（偏好，非库数据）。
/// 免只读守卫（owner 只读态也能设）；鉴权守卫照常（设了密码时匿名不可改系统语言）。
async fn put_locale(State(state): State<Arc<AppState>>, Json(req): Json<LocaleReq>) -> Response {
    match state.config.lock().unwrap().set_ui_locale(req.locale.trim()) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "internal", "message": e.to_string() })),
        )
            .into_response(),
    }
}

async fn get_config(State(state): State<Arc<AppState>>) -> Json<SourceConfig> {
    Json(state.config.lock().unwrap().load().unwrap_or_default())
}

async fn apply_config(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ApplyConfigReq>,
) -> Json<ConfigResult> {
    tracing::info!(source_type = %req.source_type, create_new = req.create_new, "applying data source config");
    let mut cfg = SourceConfig {
        source_type: req.source_type,
        local_path: req.local_path,
        webdav_url: req.webdav_url,
        webdav_user: req.webdav_user,
        webdav_pass: req.webdav_pass,
        plugin_id: req.plugin_id,
        plugin_storage: req.plugin_storage,
        plugin_config: serde_json::to_string(&req.plugin_config).unwrap_or_default(),
        read_only: req.read_only,
        ..Default::default()
    };
    // 插件数据源：按贡献的 config_schema 校验/规范化，并算出缓存隔离键（不含 secret）
    if cfg.source_type == "plugin" {
        if let Err(e) = crate::plugins::prepare_plugin_source(&mut cfg, state.plugins.as_ref()) {
            return Json(ConfigResult::err(e));
        }
    }
    let storage = match config::build_storage(&cfg, state.plugins.as_ref()) {
        Ok(s) => s,
        Err(e) => return Json(ConfigResult::err(e)),
    };

    // 连接 / 初始化 / 建索引放到 blocking 线程（含网络/磁盘 IO）
    let st = storage.clone();
    let create_new = req.create_new;
    let source = config::source_key(&cfg);
    let state2 = state.clone();
    let built = tokio::task::spawn_blocking(move || -> anyhow::Result<(Library, crate::library::BuildStats)> {
        if create_new {
            st.init_new()?;
        }
        let (lib, stats) = crate::indexer::build_cached(st.as_ref(), &state2.cache, &source)?;
        Ok((lib, stats))
    })
    .await;

    match built {
        Ok(Ok((lib, stats))) => {
            tracing::info!(
                source_type = %cfg.source_type, notes = stats.notes, folders = stats.folders,
                "data source switched",
            );
            *state.library.write().unwrap() = lib;
            *state.storage.write().unwrap() = Some(storage);
            if let Err(e) = state.config.lock().unwrap().save(&cfg) {
                return Json(ConfigResult::err(e));
            }
            // 运行时同步只读开关（设置页切换即时生效，无需重启）
            state.read_only.store(cfg.read_only, Ordering::Relaxed);
            // 整库被替换：通知所有客户端全量刷新（含发起切换的那个之外的标签页）
            state.events.library_reloaded();
            Json(ConfigResult { ok: true, error: None, notes: stats.notes, folders: stats.folders })
        }
        Ok(Err(e)) => {
            tracing::warn!("data source switch failed: {e}");
            Json(ConfigResult::err(e))
        }
        Err(_) => {
            tracing::warn!("data source switch task panicked");
            Json(ConfigResult::err("处理任务失败"))
        }
    }
}

// ---------- 访问鉴权 handlers ----------

#[derive(Deserialize)]
struct LoginReq {
    #[serde(default)]
    password: String,
}

#[derive(Serialize)]
struct LoginResp {
    token: String,
}

/// POST /api/auth/login —— 校验访问密码，成功则签发会话 token（前端存 localStorage、后续写请求带 Bearer）。
async fn auth_login(State(state): State<Arc<AppState>>, Json(req): Json<LoginReq>) -> Response {
    if !state.auth.enabled() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "auth_disabled", "message": "未设置访问密码，无需登录" })),
        )
            .into_response();
    }
    if !state.auth.verify(&req.password) {
        tracing::debug!("auth login failed: wrong password");
        return unauthorized();
    }
    let token = state.auth.issue_token();
    tracing::info!("auth login succeeded, session issued");
    (StatusCode::OK, Json(LoginResp { token })).into_response()
}

/// POST /api/auth/logout —— 吊销当前会话 token（幂等；无 token 也返回 204）。
async fn auth_logout(State(state): State<Arc<AppState>>, headers: HeaderMap) -> StatusCode {
    if let Some(t) = bearer_token(&headers) {
        state.auth.revoke(&t);
    }
    StatusCode::NO_CONTENT
}

#[derive(Serialize)]
struct AuthSettingsResp {
    /// 是否已设访问密码（密码本身/哈希永不回传）。
    password_set: bool,
    passwordless_read: bool,
    list_mode: String,
    folder_list: Vec<String>,
}

fn auth_settings_resp(cfg: AuthConfig) -> AuthSettingsResp {
    AuthSettingsResp {
        password_set: !cfg.password_hash.is_empty(),
        passwordless_read: cfg.passwordless_read,
        list_mode: cfg.list_mode,
        folder_list: cfg.folder_list,
    }
}

/// GET /api/auth/settings —— 访问控制设置（供设置页预填）。guard_auth 已限定为 Full
/// （未设密码时人人 Full → 首次配置可读）。密码只回 `password_set` 布尔，绝不回哈希。
async fn get_auth_settings(State(state): State<Arc<AppState>>) -> Json<AuthSettingsResp> {
    let cfg = state.config.lock().unwrap().auth_config();
    Json(auth_settings_resp(cfg))
}

#[derive(Deserialize)]
struct PutAuthSettingsReq {
    /// 设置/修改访问密码：非空即改；省略/空 = 保持不变（除非 clear_password）。
    #[serde(default)]
    password: Option<String>,
    /// 清除访问密码（关闭鉴权，回到全开放）。
    #[serde(default)]
    clear_password: bool,
    #[serde(default)]
    passwordless_read: bool,
    #[serde(default = "default_list_mode")]
    list_mode: String,
    #[serde(default)]
    folder_list: Vec<String>,
}

fn default_list_mode() -> String {
    "none".to_string()
}

/// PUT /api/auth/settings —— 保存访问控制设置并即时同步运行态。guard_auth 已限定为 Full
/// （未设密码时人人 Full → 首次可设密码）；改/清密码会吊销全部既有会话。
async fn put_auth_settings(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PutAuthSettingsReq>,
) -> Result<Json<AuthSettingsResp>, StatusCode> {
    let mut cfg = state.config.lock().unwrap().auth_config();
    let mut password_changed = false;
    if req.clear_password {
        cfg.password_hash.clear();
        cfg.password_salt.clear();
        password_changed = true;
    } else if let Some(pw) = req.password.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let salt = crate::auth::gen_salt();
        cfg.password_hash = crate::auth::hash_password(pw, &salt);
        cfg.password_salt = salt;
        password_changed = true;
    }
    cfg.passwordless_read = req.passwordless_read;
    cfg.list_mode = req.list_mode;
    cfg.folder_list = req.folder_list;

    state
        .config
        .lock()
        .unwrap()
        .save_auth_config(&cfg)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    // 同步运行态（改设置即时生效，无需重启）
    state.auth.reload(&cfg);
    if password_changed {
        state.auth.revoke_all(); // 改/清密码 → 所有既有会话失效
    }
    tracing::info!(
        password_set = !cfg.password_hash.is_empty(),
        passwordless_read = cfg.passwordless_read,
        list_mode = %cfg.list_mode,
        "auth settings updated",
    );
    Ok(Json(auth_settings_resp(cfg)))
}

// ---------- 设置描述符（server-driven settings，供前端通用渲染器）----------

/// GET /api/settings/schema —— 下发分区化的设置描述符：分区目录 + 字段 schema + 当前值 + 动作。
/// 前端据此用单一通用渲染器渲染侧边栏设置面板（数据源 / 访问控制 / AI / 外观 / 编辑器），
/// 无需在前端硬编码有哪些设置、字段、顺序、可用性。分区顺序、图标、i18n 键均由此处控制。
///
/// 字段 `label_key`/`placeholder_key`/`desc_key` 是 i18n 键，前端 `t()` 解析（未知回退原串，
/// 故 provider 名等字面量也可直接放）。`values` 为当前值（含 secret，故 guard_auth 已把本路径
/// 列为机密读 → 匿名 401）。`scope=client` 的分区（外观 / 编辑器）无 `values`/`actions`，值由前端
/// 存 localStorage。AI 段仅当插件宿主可用时出现（`/api/ai/config` 亦仅 plugins 构建存在）。
async fn settings_schema(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    use serde_json::json;
    let (cfg, auth) = {
        let store = state.config.lock().unwrap();
        (store.load().unwrap_or_default(), store.auth_config())
    };

    // 数据源当前值：把 source_type=="plugin" 还原成前端用的 provider 键 `plugin:<id>:<contrib>`，
    // 让 enum 能回显选中项；plugin_config 文本还原成对象供 provider 子表单预填。
    let source_type_key = if cfg.source_type == "plugin" {
        format!("plugin:{}:{}", cfg.plugin_id, cfg.plugin_storage)
    } else if cfg.source_type.is_empty() {
        "local".to_string()
    } else {
        cfg.source_type.clone()
    };
    let plugin_config_val: serde_json::Value =
        serde_json::from_str(&cfg.plugin_config).unwrap_or_else(|_| json!({}));

    let data_source = json!({
        "id": "data-source",
        "title_key": "settings.section.dataSource",
        "icon": "folder",
        "scope": "server",
        "search_keys": ["settings.local", "settings.webdav", "settings.readOnly", "settings.folderPath"],
        "fields": [
            { "key": "create_new", "type": "enum", "label_key": "settings.libMode", "default": "existing",
              "options": [
                { "value": "existing", "label_key": "settings.useExisting" },
                { "value": "new", "label_key": "settings.createNew" }
              ] },
            { "key": "source_type", "type": "enum", "label_key": "settings.sourceType", "default": "local",
              "options_source": "storage_providers",
              "options": [
                { "value": "local", "label_key": "settings.local" },
                { "value": "webdav", "label_key": "settings.webdav" }
              ] },
            { "key": "local_path", "type": "text", "label_key": "settings.folderPath",
              "placeholder_key": "settings.localPhExisting",
              "show_if": { "field": "source_type", "equals": "local" } },
            { "key": "webdav_url", "type": "text", "label_key": "settings.webdavUrl",
              "placeholder_key": "settings.webdavUrlPh",
              "show_if": { "field": "source_type", "equals": "webdav" } },
            { "key": "webdav_user", "type": "text", "label_key": "settings.username",
              "show_if": { "field": "source_type", "equals": "webdav" } },
            { "key": "webdav_pass", "type": "secret", "label_key": "settings.password",
              "show_if": { "field": "source_type", "equals": "webdav" } },
            // 选中插件 provider（source_type 非 local/webdav）时渲染该 provider 的 config_schema 子表单
            { "key": "plugin_config", "type": "provider-config",
              "show_if": { "field": "source_type", "not_in": ["local", "webdav"] } },
            { "key": "read_only", "type": "bool", "label_key": "settings.readOnly",
              "desc_key": "settings.readOnlyDesc" }
        ],
        "values": {
            "create_new": "existing",
            "source_type": source_type_key,
            "local_path": cfg.local_path,
            "webdav_url": cfg.webdav_url,
            "webdav_user": cfg.webdav_user,
            "webdav_pass": cfg.webdav_pass,
            "plugin_config": plugin_config_val,
            "read_only": cfg.read_only
        },
        "actions": [
            { "id": "connect", "label_key": "settings.connect", "variant": "primary",
              "request": { "method": "PUT", "url": "/api/config", "convention": "config-result" },
              "on_success": "reload" }
        ]
    });

    let access_control = json!({
        "id": "access-control",
        "title_key": "settings.section.accessControl",
        "icon": "lock",
        "scope": "server",
        "desc_key": "settings.auth.desc",
        "search_keys": ["settings.auth.password", "settings.auth.passwordlessRead", "settings.auth.listMode"],
        "fields": [
            // 密码只写不回显（回 password_set 布尔）；已设时渲染器用 form.secretSetPh 提示留空保持不变
            { "key": "password", "type": "secret", "label_key": "settings.auth.password",
              "writeonly": true, "set_flag": "password_set",
              "placeholder_key": "settings.auth.passwordSetPh" },
            { "key": "passwordless_read", "type": "bool", "label_key": "settings.auth.passwordlessRead",
              "desc_key": "settings.auth.passwordlessReadDesc" },
            { "key": "list_mode", "type": "enum", "label_key": "settings.auth.listMode", "default": "none",
              "desc_key": "settings.auth.listHint",
              "show_if": { "field": "passwordless_read", "truthy": true },
              "options": [
                { "value": "none", "label_key": "settings.auth.listModeNone" },
                { "value": "whitelist", "label_key": "settings.auth.listModeWhitelist" },
                { "value": "blacklist", "label_key": "settings.auth.listModeBlacklist" }
              ] },
            { "key": "folder_list", "type": "notebook-multiselect", "options_source": "folders",
              "empty_key": "settings.auth.noFolders",
              "show_if": { "field": "list_mode", "in": ["whitelist", "blacklist"] } }
        ],
        "values": {
            "password_set": !auth.password_hash.is_empty(),
            "passwordless_read": auth.passwordless_read,
            "list_mode": auth.list_mode,
            "folder_list": auth.folder_list
        },
        "actions": [
            { "id": "clear", "label_key": "settings.auth.clearPassword", "variant": "danger",
              "show_if": { "field": "password_set", "truthy": true },
              "request": { "method": "PUT", "url": "/api/auth/settings", "convention": "status",
                           "extra": { "clear_password": true } },
              "on_success": "relogin" },
            { "id": "save", "label_key": "settings.auth.save", "variant": "primary",
              "request": { "method": "PUT", "url": "/api/auth/settings", "convention": "status" },
              "on_success": "relogin" }
        ]
    });

    let appearance = json!({
        "id": "appearance",
        "title_key": "settings.section.appearance",
        "icon": "palette",
        "scope": "client",
        "fields": [
            { "key": "theme", "type": "theme", "label_key": "settings.appearance.theme" },
            { "key": "language", "type": "language", "label_key": "settings.appearance.language" }
        ]
    });

    let editor = json!({
        "id": "editor",
        "title_key": "settings.section.editor",
        "icon": "edit",
        "scope": "client",
        "fields": [
            { "key": "engine", "type": "enum", "label_key": "settings.editor.default", "default": "source",
              // 客户端本地偏好：值存 localStorage（与 NoteView 共享 jasper.editor 键）
              "client_store": "jasper.editor",
              "options": [
                { "value": "source", "label_key": "settings.editor.source" },
                { "value": "wysiwyg", "label_key": "settings.editor.wysiwyg" }
              ] }
        ]
    });

    let mut sections = vec![data_source, access_control];
    // AI 段仅在插件宿主可用时出现（宿主级 AI 配置、/api/ai/config 均只在 plugins 构建存在）
    if state.plugins.is_some() {
        let ai = state.config.lock().unwrap().ai_config();
        sections.push(json!({
            "id": "ai",
            "title_key": "settings.section.ai",
            "icon": "sparkles",
            "scope": "server",
            "desc_key": "settings.ai.desc",
            "search_keys": ["settings.ai.provider", "settings.ai.apiKey", "settings.ai.model"],
            "fields": [
                { "key": "provider", "type": "enum", "label_key": "settings.ai.provider", "default": "",
                  "options": [
                    { "value": "", "label_key": "settings.ai.providerNone" },
                    { "value": "anthropic", "label_key": "Anthropic" },
                    { "value": "openai", "label_key": "OpenAI API" }
                  ] },
                { "key": "base_url", "type": "text", "label_key": "settings.ai.baseUrl",
                  "placeholder_key": "settings.ai.baseUrlPh",
                  "show_if": { "field": "provider", "truthy": true } },
                // api_key 回显（不遮罩）：PUT /api/ai/config 是整体替换，遮罩+空串回传会静默清空密钥
                { "key": "api_key", "type": "secret", "label_key": "settings.ai.apiKey",
                  "show_if": { "field": "provider", "truthy": true } },
                { "key": "model", "type": "text", "label_key": "settings.ai.model",
                  "placeholder_key": "settings.ai.modelPh",
                  "show_if": { "field": "provider", "truthy": true } }
            ],
            "values": {
                "provider": ai.provider,
                "base_url": ai.base_url,
                "api_key": ai.api_key,
                "model": ai.model
            },
            "actions": [
                { "id": "save", "label_key": "settings.ai.save", "variant": "primary",
                  "request": { "method": "PUT", "url": "/api/ai/config", "convention": "status" },
                  "on_success": "saved" }
            ]
        }));
    }
    // MCP 段仅在本构建含 MCP server 时出现（需 `--features mcp`）。
    // endpoint / command 的值是**模板**：服务端不知道客户端是从哪个地址访问它的（127.0.0.1?
    // 局域网 IP? 反代域名?），也不该知道浏览器里的会话 token，故留 {origin} / {header}
    // 两个占位符由前端按运行时实际情况填（见 settingsSchema.ts::fillPlaceholders）。
    if crate::mcp::ENABLED {
        let (enabled, api_key) = {
            let store = state.config.lock().unwrap();
            (store.mcp_enabled(), store.mcp_api_key())
        };
        let shown_if_on = json!({ "field": "enabled", "truthy": true });
        // 命令里的认证头：配了 MCP key 就由服务端直接拼死（key 本来就存在服务端，
        // 这个描述符又是机密读），没配才留 {header} 占位符让前端填浏览器会话 token
        // ——服务端不知道也不该知道后者。
        let header_part = if api_key.is_empty() {
            "{header}".to_string()
        } else {
            format!(" --header \"Authorization: Bearer {api_key}\"")
        };
        sections.push(json!({
            "id": "mcp",
            "title_key": "settings.section.mcp",
            "icon": "plug",
            "scope": "server",
            "desc_key": "settings.mcp.desc",
            "search_keys": ["settings.mcp.enabled", "settings.mcp.endpoint", "settings.mcp.command"],
            "fields": [
                { "key": "enabled", "type": "bool", "label_key": "settings.mcp.enabled",
                  "desc_key": "settings.mcp.enabledDesc" },
                { "key": "endpoint", "type": "copy", "label_key": "settings.mcp.endpoint",
                  "show_if": shown_if_on },
                // key 本身：设了才显示（`api_key_set` 是 values 里的只读标记）
                { "key": "api_key", "type": "copy", "label_key": "settings.mcp.apiKey",
                  "desc_key": "settings.mcp.apiKeyDesc",
                  "show_if": { "field": "api_key_set", "truthy": true } },
                { "key": "command", "type": "copy", "label_key": "settings.mcp.command",
                  "desc_key": "settings.mcp.commandDesc", "show_if": shown_if_on },
                { "key": "tools", "type": "note", "label_key": "settings.mcp.tools",
                  "show_if": shown_if_on }
            ],
            "values": {
                "enabled": enabled,
                "endpoint": format!("{{origin}}{MCP_PATH}"),
                "api_key": api_key,
                "api_key_set": !api_key.is_empty(),
                "command": format!("claude mcp add --transport http jasper {{origin}}{MCP_PATH}{header_part}"),
                "tools": crate::mcp::TOOL_NAMES.join("、")
            },
            "actions": [
                { "id": "save", "label_key": "settings.mcp.save", "variant": "primary",
                  "request": { "method": "PUT", "url": "/api/mcp/config", "convention": "status" },
                  "on_success": "saved" },
                // 生成/轮换：只发动作，不带字段值（免得顺手把开关也改了）。
                // 未设 key 时叫「生成」、已设时叫「重新生成」——同一个请求，两种措辞。
                { "id": "generate", "label_key": "settings.mcp.generate",
                  "request": { "method": "PUT", "url": "/api/mcp/config", "convention": "status",
                               "extra": { "regenerate_key": true } },
                  "submit": false, "on_success": "reload-section",
                  "show_if": { "field": "api_key_set", "truthy": false } },
                { "id": "regenerate", "label_key": "settings.mcp.regenerate",
                  "request": { "method": "PUT", "url": "/api/mcp/config", "convention": "status",
                               "extra": { "regenerate_key": true } },
                  "submit": false, "on_success": "reload-section",
                  "show_if": { "field": "api_key_set", "truthy": true } },
                { "id": "clear_key", "label_key": "settings.mcp.clearKey", "variant": "danger",
                  "request": { "method": "PUT", "url": "/api/mcp/config", "convention": "status",
                               "extra": { "clear_key": true } },
                  "submit": false, "on_success": "reload-section",
                  "show_if": { "field": "api_key_set", "truthy": true } }
            ]
        }));
    }
    sections.push(appearance);
    sections.push(editor);

    Json(json!({ "sections": sections }))
}

#[derive(Deserialize)]
struct McpConfigReq {
    /// 缺省 = 不改（「生成新密钥」「清除密钥」这类只发动作的请求不带它）。
    #[serde(default)]
    enabled: Option<bool>,
    /// 生成一枚新的 API key（旧的立即失效）。
    #[serde(default)]
    regenerate_key: bool,
    /// 清除 API key —— MCP 端点回落到会话 token / 无密码的既有行为。
    #[serde(default)]
    clear_key: bool,
}

/// PUT /api/mcp/config —— MCP 运行时开关 + 独立 API key 的生成/清除。
/// 端点地址由服务绑定地址决定、工具集由构建决定，都不是配置。
/// 注意本路由**不**豁免只读/鉴权守卫（它是普通的 `/api/*` 写端点，不是 MCP 端点本身）——
/// 只读态下不可改、匿名不可改，与其它设置一致。这也是 key 丢了之后的找回途径：
/// 用浏览器身份重新生成即可，不会把自己锁在外面。
async fn put_mcp_config(
    State(state): State<Arc<AppState>>,
    Json(req): Json<McpConfigReq>,
) -> StatusCode {
    if !crate::mcp::ENABLED {
        return StatusCode::NOT_FOUND; // 本构建没有 MCP，没有可配置的东西
    }
    if req.regenerate_key && req.clear_key {
        return StatusCode::BAD_REQUEST; // 自相矛盾，不猜用户意图
    }
    let store = state.config.lock().unwrap();
    let mut work = || -> anyhow::Result<()> {
        if let Some(enabled) = req.enabled {
            store.set_mcp_enabled(enabled)?;
        }
        if req.regenerate_key {
            store.set_mcp_api_key(&crate::auth::gen_mcp_key())?;
        } else if req.clear_key {
            store.set_mcp_api_key("")?;
        }
        Ok(())
    };
    match work() {
        Ok(()) => {
            // 只记发生了什么，绝不记 key 本身
            tracing::info!(
                enabled = ?req.enabled,
                regenerated = req.regenerate_key,
                cleared = req.clear_key,
                "mcp config updated",
            );
            StatusCode::NO_CONTENT
        }
        Err(e) => {
            tracing::warn!("failed to persist mcp config: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

// ---------- 读 handlers ----------

fn build_folder_tree(lib: &Library, parent_id: &str) -> Vec<FolderNode> {
    lib.child_folder_ids_sorted(parent_id)
        .into_iter()
        .filter_map(|id| {
            lib.folders.get(&id).map(|f| FolderNode {
                id: f.id.clone(),
                title: f.title.clone(),
                note_count: lib.note_count(&f.id),
                children: build_folder_tree(lib, &f.id),
            })
        })
        .collect()
}

/// 匿名受限范围下的笔记本树：只含可见笔记本；某可见笔记本的父级不可见时（whitelist 的私有祖先），
/// 把它提到顶层作为根（不暴露私有祖先的存在/标题）。
fn build_visible_folder_tree(lib: &Library, scope: &Scope) -> Vec<FolderNode> {
    let visible: HashSet<String> =
        lib.folders.keys().filter(|id| scope.allows_folder(id)).cloned().collect();
    // 根 = 可见但父级不可见（含父级为 "" 根）的笔记本
    let mut roots: Vec<String> = visible
        .iter()
        .filter(|id| {
            let parent = lib.folders.get(*id).map(|f| f.parent_id.clone()).unwrap_or_default();
            !visible.contains(&parent)
        })
        .cloned()
        .collect();
    roots.sort_by(|a, b| {
        let ta = lib.folders.get(a).map(|f| f.title.as_str()).unwrap_or("");
        let tb = lib.folders.get(b).map(|f| f.title.as_str()).unwrap_or("");
        ta.cmp(tb)
    });
    roots.iter().map(|r| build_visible_subtree(lib, r, &visible)).collect()
}

fn build_visible_subtree(lib: &Library, id: &str, visible: &HashSet<String>) -> FolderNode {
    let children = lib
        .child_folder_ids_sorted(id)
        .into_iter()
        .filter(|c| visible.contains(c))
        .map(|c| build_visible_subtree(lib, &c, visible))
        .collect();
    FolderNode {
        id: id.to_string(),
        title: lib.folders.get(id).map(|f| f.title.clone()).unwrap_or_default(),
        note_count: lib.note_count(id),
        children,
    }
}

pub(crate) async fn folders(
    State(state): State<Arc<AppState>>,
    Extension(access): Extension<Access>,
) -> Json<Vec<FolderNode>> {
    let lib = state.library.read().unwrap();
    let scope = state.auth.scope(&lib, access);
    if scope.is_none() {
        return Json(Vec::new()); // 私有：匿名看不到任何笔记本
    }
    let mut nodes = match &scope {
        Scope::All => build_folder_tree(&lib, ""),
        _ => build_visible_folder_tree(&lib, &scope),
    };
    // 未挂在任何笔记本下的笔记（parent_id==""）用一个合成节点表示，放最前面；仅当该范围可见时给。
    // id 仍是 ""，前端已有的 selectFolder("")/api.notes("") 直接复用。标题留空，
    // 由前端按 id==="" 特判取本地化文案（服务端不做 i18n）。
    if scope.allows_folder("") {
        let root_count = lib.note_count("");
        if root_count > 0 {
            nodes.insert(0, FolderNode { id: String::new(), title: String::new(), note_count: root_count, children: Vec::new() });
        }
    }
    Json(nodes)
}

#[derive(Deserialize)]
pub(crate) struct NotesQuery {
    pub(crate) folder: Option<String>,
}

pub(crate) async fn notes_list(
    State(state): State<Arc<AppState>>,
    Extension(access): Extension<Access>,
    Query(q): Query<NotesQuery>,
) -> Json<Vec<NoteSummary>> {
    let lib = state.library.read().unwrap();
    let folder = q.folder.unwrap_or_default();
    // 该笔记本对当前访问级别不可见 → 空列表（匿名不可越权枚举）
    if !state.auth.scope(&lib, access).allows_folder(&folder) {
        return Json(Vec::new());
    }
    Json(lib.notes_in_folder_sorted(&folder).into_iter().map(summarize).collect())
}

pub(crate) async fn note_detail(
    State(state): State<Arc<AppState>>,
    Extension(access): Extension<Access>,
    Path(id): Path<String>,
) -> Result<Json<NoteDetail>, StatusCode> {
    let lib = state.library.read().unwrap();
    let n = lib.note(&id).ok_or(StatusCode::NOT_FOUND)?;
    // 笔记所在笔记本不可见 → 404（不区分「不存在」与「无权」，避免探测）
    if !state.auth.scope(&lib, access).allows_folder(&n.parent_id) {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(Json(detail_of(n)))
}

#[derive(Serialize)]
pub(crate) struct TagInfo {
    id: String,
    title: String,
    /// 打了该标签且仍存在的笔记数。
    note_count: usize,
}

/// GET /api/tags —— 全部标签（按标题排序，含篇数）。匿名受限时篇数只数可见笔记、零可见的标签隐藏。
pub(crate) async fn tags_list(
    State(state): State<Arc<AppState>>,
    Extension(access): Extension<Access>,
) -> Json<Vec<TagInfo>> {
    let lib = state.library.read().unwrap();
    let scope = state.auth.scope(&lib, access);
    if scope.is_none() {
        return Json(Vec::new());
    }
    let mut out = Vec::new();
    for t in lib.tags_sorted() {
        if matches!(scope, Scope::All) {
            // Full/全库可读：保持原样（含篇数 0 的标签也列出）
            out.push(TagInfo { id: t.id.clone(), title: t.title.clone(), note_count: lib.tag_note_count(&t.id) });
        } else {
            // 受限：只数可见笔记，零可见则隐藏该标签
            let count = lib
                .notes_with_tag(&t.id)
                .into_iter()
                .filter(|n| scope.allows_folder(&n.parent_id))
                .count();
            if count > 0 {
                out.push(TagInfo { id: t.id.clone(), title: t.title.clone(), note_count: count });
            }
        }
    }
    Json(out)
}

/// GET /api/tags/{id}/notes —— 打了某标签的笔记（按更新时间倒序），过滤到可见笔记本。
pub(crate) async fn tag_notes(
    State(state): State<Arc<AppState>>,
    Extension(access): Extension<Access>,
    Path(id): Path<String>,
) -> Json<Vec<NoteSummary>> {
    let lib = state.library.read().unwrap();
    let scope = state.auth.scope(&lib, access);
    if scope.is_none() {
        return Json(Vec::new());
    }
    Json(
        lib.notes_with_tag(&id)
            .into_iter()
            .filter(|n| scope.allows_folder(&n.parent_id))
            .map(summarize)
            .collect(),
    )
}

#[derive(Serialize)]
pub(crate) struct TagRef {
    id: String,
    title: String,
}

fn tag_ref(t: &crate::model::Tag) -> TagRef {
    TagRef { id: t.id.clone(), title: t.title.clone() }
}

#[derive(Deserialize)]
pub(crate) struct AddTagReq {
    pub(crate) title: String,
}

/// GET /api/notes/{id}/tags —— 某笔记的标签（按标题排序）。笔记所在笔记本不可见 → 404（同 note_detail）。
pub(crate) async fn note_tags_list(
    State(state): State<Arc<AppState>>,
    Extension(access): Extension<Access>,
    Path(id): Path<String>,
) -> Result<Json<Vec<TagRef>>, StatusCode> {
    let lib = state.library.read().unwrap();
    let note = lib.note(&id).ok_or(StatusCode::NOT_FOUND)?;
    if !state.auth.scope(&lib, access).allows_folder(&note.parent_id) {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(Json(lib.tags_of_note(&id).into_iter().map(tag_ref).collect()))
}

/// 打标签的执行计划（在读锁内算好，避免持锁做 IO / 借用冲突）。
enum TagPlan {
    /// 笔记已有该标签 → 幂等无操作。
    Noop,
    /// 需落盘：可选的新建标签条目 + 必建的 note_tag 关联条目（均为 (文件名, 内容)）。
    Create {
        new_tag: Option<(String, String)>,
        note_tag: (String, String),
    },
}

/// POST /api/notes/{id}/tags —— 给笔记打标签 { title }。
/// 语义对齐 Joplin `addNoteTagByTitle`：标题 trim + 不区分大小写复用已有标签，
/// 不存在则新建（type_=5），再建 note_tag 关联（type_=6）；笔记已有该标签则幂等。
pub(crate) async fn add_note_tag(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<AddTagReq>,
) -> Result<Json<Vec<TagRef>>, StatusCode> {
    // 提早确认有数据源（写路径）。
    storage_of(&state).ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let title = req.title.trim().to_string();
    if title.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let now = serialize::now_ms();

    let plan = {
        let lib = state.library.read().unwrap();
        if lib.note(&id).is_none() {
            return Err(StatusCode::NOT_FOUND);
        }
        match lib.tag_id_by_title(&title) {
            Some(tid) if lib.note_has_tag(&id, &tid) => TagPlan::Noop,
            Some(tid) => {
                let ntid = serialize::new_id();
                TagPlan::Create {
                    new_tag: None,
                    note_tag: (ntid.clone(), serialize::new_note_tag_md(&ntid, &id, &tid, now)),
                }
            }
            None => {
                let tid = serialize::new_id();
                let ntid = serialize::new_id();
                TagPlan::Create {
                    new_tag: Some((tid.clone(), serialize::new_tag_md(&tid, &title, now))),
                    note_tag: (ntid.clone(), serialize::new_note_tag_md(&ntid, &id, &tid, now)),
                }
            }
        }
    };

    if let TagPlan::Create { new_tag, note_tag } = plan {
        // 先落标签再落关联：读者先看到关联时，其引用的标签已存在。
        if let Some((tid, content)) = new_tag {
            write_item(&state, format!("{tid}.md"), content.clone()).await?;
            state.library.write().unwrap().upsert_tag(&content).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        let (ntid, content) = note_tag;
        write_item(&state, format!("{ntid}.md"), content.clone()).await?;
        state.library.write().unwrap().upsert_note_tag(&content).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        state.events.tags_changed(&id);
    }

    let lib = state.library.read().unwrap();
    Ok(Json(lib.tags_of_note(&id).into_iter().map(tag_ref).collect()))
}

/// DELETE /api/notes/{id}/tags/{tag_id} —— 从笔记去掉某标签（删 note_tag 关联，保留标签本身）。
/// 对齐 Joplin `removeNote`：删除该 (note,tag) 的全部关联；无关联则幂等返回当前标签。
pub(crate) async fn remove_note_tag(
    State(state): State<Arc<AppState>>,
    Path((id, tag_id)): Path<(String, String)>,
) -> Result<Json<Vec<TagRef>>, StatusCode> {
    storage_of(&state).ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let ntids = state.library.read().unwrap().note_tag_ids_for(&id, &tag_id);
    if !ntids.is_empty() {
        for ntid in &ntids {
            delete_item(&state, format!("{ntid}.md")).await?;
            state.library.write().unwrap().remove_note_tag(ntid);
        }
        state.events.tags_changed(&id);
    }
    let lib = state.library.read().unwrap();
    Ok(Json(lib.tags_of_note(&id).into_iter().map(tag_ref).collect()))
}

/// GET /api/resources/{id} —— 资源二进制。资源自身不知道所在笔记本，故按引用它的笔记的
/// parent_id 间接判定可见性（resource→note→folder），同 `note_detail` 一律 404 不区分不存在/无权。
/// 孤儿资源（无笔记引用）在受限范围下没有可归属的权限链路 → 默认拒绝。
async fn resource(
    State(state): State<Arc<AppState>>,
    Extension(access): Extension<Access>,
    Path(id): Path<String>,
) -> Response {
    let mime = {
        let lib = state.library.read().unwrap();
        let scope = state.auth.scope(&lib, access);
        if !matches!(scope, Scope::All)
            && !lib.notes_referencing_resource(&id).iter().any(|n| scope.allows_folder(&n.parent_id))
        {
            return StatusCode::NOT_FOUND.into_response();
        }
        lib.resource(&id)
            .map(|r| r.mime.clone())
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| "application/octet-stream".to_string())
    };
    let storage = match storage_of(&state) {
        Some(s) => s,
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    let bytes = tokio::task::spawn_blocking(move || storage.get_resource(&id)).await;
    match bytes {
        Ok(Ok(bytes)) => (
            [
                (header::CONTENT_TYPE, mime),
                (header::CACHE_CONTROL, "public, max-age=31536000".to_string()),
            ],
            bytes,
        )
            .into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

// ---------- 资源上传 ----------

#[derive(Deserialize)]
struct UploadQuery {
    #[serde(default)]
    filename: String,
}

#[derive(Serialize)]
struct UploadResp {
    id: String,
    title: String,
    mime: String,
    file_extension: String,
    size: usize,
    /// 可直接插入笔记正文的引用片段（图片用 `![]`，其它用 `[]`）。
    markdown: String,
}

/// 从文件名末尾取扩展名（小写，限字母数字、长度 ≤8）。
fn ext_from_filename(name: &str) -> Option<String> {
    let (_, e) = name.rsplit_once('.')?;
    let e = e.to_lowercase();
    (!e.is_empty() && e.len() <= 8 && e.chars().all(|c| c.is_ascii_alphanumeric())).then_some(e)
}

/// 常见 MIME → 扩展名兜底（文件名无扩展名时用）。
fn ext_from_mime(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "image/bmp" => "bmp",
        "image/avif" => "avif",
        "application/pdf" => "pdf",
        _ => return None,
    })
}

/// POST /api/resources?filename=<名> —— 请求体为原始二进制，Content-Type 为其 MIME。
/// 写入 `.resource/<id>` 二进制 + `<id>.md` 元数据条目，返回可插入正文的引用片段。
async fn upload_resource(
    State(state): State<Arc<AppState>>,
    Query(q): Query<UploadQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<UploadResp>, StatusCode> {
    let storage = storage_of(&state).ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    if body.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mime = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or(s).trim().to_string())
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| "application/octet-stream".to_string());

    let filename = q.filename.trim().to_string();
    let ext = ext_from_filename(&filename)
        .or_else(|| ext_from_mime(&mime).map(|s| s.to_string()))
        .unwrap_or_default();
    let id = serialize::new_id();
    let title = match (filename.is_empty(), ext.is_empty()) {
        (false, _) => filename.clone(),
        (true, false) => format!("{id}.{ext}"),
        (true, true) => id.clone(),
    };
    let size = body.len();
    let content = serialize::new_resource_md(&id, &title, &mime, &ext, size as i64, serialize::now_ms());

    // 先写二进制再写元数据（读者看到元数据时二进制已就绪）。IO 放 blocking 线程。
    let st = storage.clone();
    let id2 = id.clone();
    let content2 = content.clone();
    let bytes = body.to_vec();
    let written = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        st.put_resource(&id2, &bytes)?;
        st.put_item(&format!("{id2}.md"), &content2)?;
        Ok(())
    })
    .await;
    if !matches!(written, Ok(Ok(()))) {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    state
        .library
        .write()
        .unwrap()
        .upsert_resource(&content)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let markdown = if mime.starts_with("image/") {
        format!("![{title}](:/{id})")
    } else {
        format!("[{title}](:/{id})")
    };
    Ok(Json(UploadResp { id, title, mime, file_extension: ext, size, markdown }))
}

// ---------- 资源管理 ----------

#[derive(Serialize)]
struct ResourceInfo {
    id: String,
    title: String,
    mime: String,
    file_extension: String,
    size: i64,
    updated_time: i64,
    /// 引用该资源的笔记数（0 = 无人引用的孤儿）。
    used_by: usize,
}

fn resource_info(r: &crate::model::Resource, usage: &std::collections::HashMap<String, usize>) -> ResourceInfo {
    ResourceInfo {
        id: r.id.clone(),
        title: r.title.clone(),
        mime: r.mime.clone(),
        file_extension: r.file_extension.clone(),
        size: r.size,
        updated_time: r.updated_time,
        used_by: usage.get(&r.id).copied().unwrap_or(0),
    }
}

/// GET /api/resources —— 资源清单（含引用计数）。孤儿在前，其次按体积降序。
async fn list_resources(State(state): State<Arc<AppState>>) -> Json<Vec<ResourceInfo>> {
    let lib = state.library.read().unwrap();
    let usage = lib.resource_usage();
    let mut out: Vec<ResourceInfo> = lib.resources.values().map(|r| resource_info(r, &usage)).collect();
    out.sort_by(|a, b| a.used_by.cmp(&b.used_by).then(b.size.cmp(&a.size)));
    Json(out)
}

#[derive(Deserialize)]
struct RenameResourceReq {
    title: String,
}

/// PUT /api/resources/{id} —— 重命名资源（改标题元数据，刷新更新时间）。
async fn rename_resource(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<RenameResourceReq>,
) -> Result<Json<ResourceInfo>, StatusCode> {
    let storage = storage_of(&state).ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let name = format!("{id}.md");

    // 取原元数据 → 改标题 → 写回（IO 走 blocking）
    let st = storage.clone();
    let name2 = name.clone();
    let original = tokio::task::spawn_blocking(move || st.get_item(&name2))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let content = serialize::update_resource_md(&original, req.title.trim(), serialize::now_ms())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    write_item(&state, name, content.clone()).await?;

    let mut lib = state.library.write().unwrap();
    lib.upsert_resource(&content).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let usage = lib.resource_usage();
    let r = lib.resource(&id).ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(resource_info(r, &usage)))
}

/// DELETE /api/resources/{id} —— 删除资源（二进制 + 元数据条目）。
/// 不检查引用：调用方（前端）应先就引用情况向用户确认。
async fn delete_resource(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> StatusCode {
    let storage = match storage_of(&state) {
        Some(s) => s,
        None => return StatusCode::SERVICE_UNAVAILABLE,
    };
    let id2 = id.clone();
    let res = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        storage.delete_item(&format!("{id2}.md"))?; // 先删元数据（决定是否出现在清单）
        storage.delete_resource(&id2)?; // 再删二进制（幂等）
        Ok(())
    })
    .await;
    match res {
        Ok(Ok(())) => {
            state.library.write().unwrap().remove_resource(&id);
            StatusCode::NO_CONTENT
        }
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[derive(Deserialize)]
pub(crate) struct SearchQuery {
    pub(crate) q: Option<String>,
}

pub(crate) async fn search(
    State(state): State<Arc<AppState>>,
    Extension(access): Extension<Access>,
    Query(sq): Query<SearchQuery>,
) -> Json<Vec<NoteSummary>> {
    let lib = state.library.read().unwrap();
    let q = sq.q.unwrap_or_default();
    let scope = state.auth.scope(&lib, access);
    if scope.is_none() {
        return Json(Vec::new());
    }
    // 命中过滤到可见笔记本（Scope::All 时 allows_folder 恒真，无额外开销）
    Json(
        lib.search(&q)
            .into_iter()
            .filter(|n| scope.allows_folder(&n.parent_id))
            .map(summarize)
            .collect(),
    )
}

/// GET /api/events —— SSE 变更流（事件名 `change`，data 为 ChangeEvent JSON）。
/// 只带 (kind, op, id)，内容由前端按需再拉；接收端落后（lagged，慢消费者被丢事件）
/// 折算成一条 library reload，前端全量刷新兜底。GET 方法天然通过只读守卫。
///
/// 隐私：匿名且可见范围受限（非全库）时，把每条事件**折算为 library reload**——
/// 丢掉具体笔记/笔记本 id，避免向匿名者泄露私有条目的 id 与变更时机（前端收到 reload 只做
/// 一次按范围过滤的全量刷新）。Full / 无密码全库阅读则照常带 id。
async fn events_sse(
    State(state): State<Arc<AppState>>,
    Extension(access): Extension<Access>,
) -> axum::response::sse::Sse<impl tokio_stream::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>>
{
    use axum::response::sse::{Event, KeepAlive, Sse};
    use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
    use tokio_stream::wrappers::BroadcastStream;
    use tokio_stream::StreamExt as _;

    let coarsen = {
        let lib = state.library.read().unwrap();
        matches!(access, Access::Anonymous) && !matches!(state.auth.scope(&lib, access), Scope::All)
    };
    let stream = BroadcastStream::new(state.events.subscribe()).filter_map(move |item| {
        let ev = match item {
            Ok(ev) if !coarsen => ev,
            // 受限匿名：不透传具体 id，一律折算 reload
            Ok(_) => crate::events::ChangeEvent::reload(),
            Err(BroadcastStreamRecvError::Lagged(_)) => crate::events::ChangeEvent::reload(),
        };
        // 事件是纯 &'static str + String 结构，序列化不会失败；防御性丢弃而非 panic
        Event::default().event("change").json_data(&ev).ok().map(Ok)
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

// ---------- 写 handlers ----------

async fn write_item(state: &Arc<AppState>, name: String, content: String) -> Result<(), StatusCode> {
    let storage = storage_of(state).ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    tokio::task::spawn_blocking(move || storage.put_item(&name, &content))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn delete_item(state: &Arc<AppState>, name: String) -> Result<(), StatusCode> {
    let storage = storage_of(state).ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    tokio::task::spawn_blocking(move || storage.delete_item(&name))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// 笔记保存原语（阻塞）：写存储 + 刷内存索引 + 广播变更事件，返回最新 Note 快照。
/// api handlers（经 [`persist_note`]）与插件 notes.upsert/create 直写路径（已在阻塞线程）共用；
/// **不含 before-save 钩子**——由调用方决定（插件直写路径明确跳过，spec 0.3 §6.5 防重入）。
/// 事件在这个单一咽喉上发，普通 API 写入 / 插件免确认直写 / 外部 curl 写入天然全覆盖。
pub(crate) fn persist_note_blocking(
    library: &RwLock<Library>,
    storage: &dyn StorageBackend,
    events: &crate::events::EventBus,
    id: &str,
    content: &str,
) -> anyhow::Result<Note> {
    tracing::debug!(note_id = id, bytes = content.len(), "persisting note");
    if let Err(e) = storage.put_item(&format!("{id}.md"), content) {
        tracing::warn!(note_id = id, "failed to write note to storage: {e}");
        return Err(e);
    }
    let mut lib = library.write().unwrap();
    lib.upsert_note(content)?;
    let note = lib.note(id).cloned().ok_or_else(|| anyhow::anyhow!("写入后索引缺失: {id}"))?;
    events.note_upserted(id);
    Ok(note)
}

async fn persist_note(state: &Arc<AppState>, id: String, content: String) -> Result<Note, StatusCode> {
    let storage = storage_of(state).ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let library = state.library.clone();
    let events = state.events.clone();
    tokio::task::spawn_blocking(move || persist_note_blocking(&library, storage.as_ref(), &events, &id, &content))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub(crate) async fn update_note(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateNoteReq>,
) -> Result<Json<NoteDetail>, StatusCode> {
    let (original, hook_note) = {
        let lib = state.library.read().unwrap();
        let original = lib.note_raw(&id).map(|s| s.to_string()).ok_or(StatusCode::NOT_FOUND)?;
        // 给 before-save 钩子的笔记形状：现有元数据 + 新标题/正文
        let mut n = lib.note(&id).ok_or(StatusCode::NOT_FOUND)?.clone();
        n.title = req.title.clone();
        n.body = req.body.clone();
        (original, n)
    };
    // 插件 before-save 钩子（无插件时直通；插件失败回退原值，不丢数据）
    let (title, body) = crate::plugins::before_save(&state.plugins, hook_note).await;
    let content = serialize::update_note_md(&original, &title, &body, serialize::now_ms())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let n = persist_note(&state, id, content).await?;
    Ok(Json(detail_of(&n)))
}

/// PUT /api/notes/{id}/move —— 把笔记移动到另一个笔记本（改 parent_id）。
/// 目标须为已存在的笔记本；已在目标则直接返回，不写回。
async fn move_note(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<MoveNoteReq>,
) -> Result<Json<NoteDetail>, StatusCode> {
    let new_parent = req.parent_id.trim().to_string();
    let original = {
        let lib = state.library.read().unwrap();
        // 目标必须是已存在的笔记本，避免把笔记移到不存在的父级而“消失”
        if !lib.folders.contains_key(&new_parent) {
            return Err(StatusCode::BAD_REQUEST);
        }
        let note = lib.note(&id).ok_or(StatusCode::NOT_FOUND)?;
        if note.parent_id == new_parent {
            return Ok(Json(detail_of(note))); // 无变化，免写回
        }
        lib.note_raw(&id).map(|s| s.to_string()).ok_or(StatusCode::NOT_FOUND)?
    };
    let content = serialize::move_note_md(&original, &new_parent, serialize::now_ms())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let n = persist_note(&state, id, content).await?;
    Ok(Json(detail_of(&n)))
}

pub(crate) async fn create_note(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateNoteReq>,
) -> Result<Json<NoteDetail>, StatusCode> {
    let id = serialize::new_id();
    let now = serialize::now_ms();
    // 插件 before-save 钩子：以「将要创建的笔记」形状调用（spec §8 也覆盖新建）
    let hook_note = Note {
        id: id.clone(),
        parent_id: req.parent_id.clone(),
        title: req.title.clone(),
        body: req.body.clone(),
        created_time: now,
        updated_time: now,
        markup_language: MarkupLanguage::Markdown,
        is_todo: req.is_todo,
        todo_completed: false,
        is_conflict: false,
        source_url: String::new(),
        order: 0,
    };
    let (title, body) = crate::plugins::before_save(&state.plugins, hook_note).await;
    let content = serialize::new_note_md(&id, &req.parent_id, &title, &body, req.is_todo, now);

    let n = persist_note(&state, id, content).await?;
    Ok(Json(detail_of(&n)))
}

/// POST /api/folders —— 新建笔记本。parent_id 空=根，非空须为已存在笔记本。
pub(crate) async fn create_folder(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateFolderReq>,
) -> Result<Json<FolderRef>, StatusCode> {
    let parent = req.parent_id.trim().to_string();
    let title = req.title.trim().to_string();
    if title.is_empty() {
        return Err(StatusCode::BAD_REQUEST); // 名称由前端保证非空（带本地化默认名）
    }
    if !parent.is_empty() {
        let lib = state.library.read().unwrap();
        if !lib.folders.contains_key(&parent) {
            return Err(StatusCode::BAD_REQUEST);
        }
    }
    let id = serialize::new_id();
    let content = serialize::new_folder_md(&id, &parent, &title, serialize::now_ms());
    write_item(&state, format!("{id}.md"), content.clone()).await?;
    {
        let mut lib = state.library.write().unwrap();
        lib.upsert_folder(&content).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    state.events.folder_changed(&id);
    Ok(Json(FolderRef { id, title, parent_id: parent }))
}

/// PUT /api/folders/{id} —— 重命名笔记本（只改标题）。笔记本原始 .md 未缓存，故从存储现取。
async fn rename_folder(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<RenameFolderReq>,
) -> Result<Json<FolderRef>, StatusCode> {
    let storage = storage_of(&state).ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let title = req.title.trim().to_string();
    if title.is_empty() {
        return Err(StatusCode::BAD_REQUEST); // 名称由前端保证非空
    }
    {
        let lib = state.library.read().unwrap();
        if !lib.folders.contains_key(&id) {
            return Err(StatusCode::NOT_FOUND);
        }
    }
    let name = format!("{id}.md");
    let st = storage.clone();
    let name2 = name.clone();
    let original = tokio::task::spawn_blocking(move || st.get_item(&name2))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let content = serialize::rename_folder_md(&original, &title, serialize::now_ms())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    write_item(&state, name, content.clone()).await?;
    let resp = {
        let mut lib = state.library.write().unwrap();
        lib.upsert_folder(&content).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let f = lib.folders.get(&id).ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
        FolderRef { id: id.clone(), title: f.title.clone(), parent_id: f.parent_id.clone() }
    };
    state.events.folder_changed(&id);
    Ok(Json(resp))
}

/// PUT /api/folders/{id}/move —— 移动笔记本到新父级。parent_id 空=移到根。
/// 防环：不能移进自身或自身的后代。笔记本原始 .md 未缓存，故从存储现取。
async fn move_folder(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<MoveFolderReq>,
) -> Result<Json<FolderRef>, StatusCode> {
    let storage = storage_of(&state).ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let new_parent = req.parent_id.trim().to_string();
    {
        let lib = state.library.read().unwrap();
        if !lib.folders.contains_key(&id) {
            return Err(StatusCode::NOT_FOUND);
        }
        if !new_parent.is_empty() && !lib.folders.contains_key(&new_parent) {
            return Err(StatusCode::BAD_REQUEST);
        }
        if lib.is_self_or_descendant(&id, &new_parent) {
            return Err(StatusCode::BAD_REQUEST); // 移进自身/后代 → 成环
        }
        if let Some(f) = lib.folders.get(&id) {
            if f.parent_id == new_parent {
                return Ok(Json(FolderRef { id: id.clone(), title: f.title.clone(), parent_id: new_parent }));
            }
        }
    }
    let name = format!("{id}.md");
    let st = storage.clone();
    let name2 = name.clone();
    let original = tokio::task::spawn_blocking(move || st.get_item(&name2))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let content = serialize::move_folder_md(&original, &new_parent, serialize::now_ms())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    write_item(&state, name, content.clone()).await?;
    let resp = {
        let mut lib = state.library.write().unwrap();
        lib.upsert_folder(&content).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let f = lib.folders.get(&id).ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
        FolderRef { id: id.clone(), title: f.title.clone(), parent_id: f.parent_id.clone() }
    };
    state.events.folder_changed(&id);
    Ok(Json(resp))
}

/// 一次级联删除最多逐条广播多少事件；超过就折算成一条 library reload。
/// 逐条事件能让前端精准关闭「正打开的笔记被删」，但整树删除动辄上百条，
/// 既挤爆广播容量（对端 Lagged 后反正也要全量刷）又无谓刷屏。
const CASCADE_EVENT_LIMIT: usize = 64;

/// DELETE /api/folders/{id} —— 删除笔记本，**级联**删除其全部后代笔记本与其中的笔记。
/// 「未分类」(空 id) 不是真笔记本，命中 NOT_FOUND。先删笔记、再删笔记本条目，
/// 中途某条删失败即止：已删成功的那部分照样同步进内存索引（不让索引里留下盘上已没有的条目），
/// 整体报 500 让前端重拉。资源与 note_tag 关联条目不动——与 [`delete_note`] 同一口径
/// （note_tag 悬挂到已删笔记时不会进标签索引，孤儿资源在资源管理里统一清理）。
pub(crate) async fn delete_folder(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> StatusCode {
    let storage = match storage_of(&state) {
        Some(s) => s,
        None => return StatusCode::SERVICE_UNAVAILABLE,
    };
    let (folder_ids, note_ids) = {
        let lib = state.library.read().unwrap();
        if !lib.folders.contains_key(&id) {
            return StatusCode::NOT_FOUND;
        }
        // 存储是扁平的 `<id>.md`，笔记本没有真实目录，删除顺序无关紧要
        (lib.subtree_folder_ids(&id), lib.subtree_note_ids(&id))
    };
    tracing::debug!(folder_id = %id, folders = folder_ids.len(), notes = note_ids.len(), "deleting folder subtree");

    let (fids, nids) = (folder_ids.clone(), note_ids.clone());
    let res = tokio::task::spawn_blocking(move || {
        let mut done_notes: Vec<String> = Vec::new();
        let mut done_folders: Vec<String> = Vec::new();
        let mut ok = true;
        for nid in &nids {
            if let Err(e) = storage.delete_item(&format!("{nid}.md")) {
                tracing::warn!(note_id = %nid, "级联删除笔记失败: {e}");
                ok = false;
                break;
            }
            done_notes.push(nid.clone());
        }
        if ok {
            for fid in &fids {
                if let Err(e) = storage.delete_item(&format!("{fid}.md")) {
                    tracing::warn!(folder_id = %fid, "级联删除笔记本失败: {e}");
                    ok = false;
                    break;
                }
                done_folders.push(fid.clone());
            }
        }
        (ok, done_folders, done_notes)
    })
    .await;
    let (ok, done_folders, done_notes) = match res {
        Ok(x) => x,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR,
    };

    state.library.write().unwrap().remove_folders_and_notes(&done_folders, &done_notes);
    if done_folders.len() + done_notes.len() > CASCADE_EVENT_LIMIT {
        state.events.library_reloaded();
    } else {
        for nid in &done_notes {
            state.events.note_deleted(nid);
        }
        for fid in &done_folders {
            state.events.folder_deleted(fid);
        }
    }
    if ok {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

pub(crate) async fn delete_note(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> StatusCode {
    let storage = match storage_of(&state) {
        Some(s) => s,
        None => return StatusCode::SERVICE_UNAVAILABLE,
    };
    let name = format!("{id}.md");
    let res = tokio::task::spawn_blocking(move || storage.delete_item(&name)).await;
    match res {
        Ok(Ok(())) => {
            state.library.write().unwrap().remove_note(&id);
            state.events.note_deleted(&id);
            StatusCode::NO_CONTENT
        }
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt; // 提供 Router::oneshot

    // 无存储的最小 AppState（守卫在 handler 之前运行，故无需真实数据源）。
    fn state_with_read_only(read_only: bool) -> Arc<AppState> {
        state_full(read_only, AuthConfig::default())
    }

    // 带指定访问控制配置的最小 AppState。
    fn state_with_auth(cfg: AuthConfig) -> Arc<AppState> {
        state_full(false, cfg)
    }

    fn state_full(read_only: bool, auth_cfg: AuthConfig) -> Arc<AppState> {
        Arc::new(AppState {
            library: Arc::new(RwLock::new(Library::default())),
            storage: RwLock::new(None),
            config: Arc::new(Mutex::new(ConfigStore::in_memory().unwrap())),
            cache: crate::cache::CacheStore::in_memory().unwrap(),
            read_only: AtomicBool::new(read_only),
            auth: AuthState::from_config(&auth_cfg),
            plugins: None,
            events: crate::events::EventBus::new(),
        })
    }

    // 构造一份 AuthConfig（password 非空则加盐哈希）。
    fn auth_config(password: Option<&str>, passwordless: bool, mode: &str, list: &[&str]) -> AuthConfig {
        let (password_hash, password_salt) = match password {
            Some(p) => {
                let s = crate::auth::gen_salt();
                let h = crate::auth::hash_password(p, &s);
                (h, s)
            }
            None => (String::new(), String::new()),
        };
        AuthConfig {
            password_hash,
            password_salt,
            passwordless_read: passwordless,
            list_mode: mode.to_string(),
            folder_list: list.iter().map(|s| s.to_string()).collect(),
        }
    }

    async fn status_of(state: Arc<AppState>, method: &str, uri: &str, body: &'static str) -> StatusCode {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        router(state).oneshot(req).await.unwrap().status()
    }

    // 发一个请求（可带 Bearer token），返回完整 Response。
    async fn send(
        state: Arc<AppState>,
        method: &str,
        uri: &str,
        body: &str,
        token: Option<&str>,
    ) -> Response {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(t) = token {
            builder = builder.header("authorization", format!("Bearer {t}"));
        }
        router(state).oneshot(builder.body(Body::from(body.to_owned())).unwrap()).await.unwrap()
    }

    async fn body_json(resp: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    // 演示库：a(root) > b，独立 d(root)；三处各一篇笔记（id 1/2/3）。
    fn sample_contents() -> (Vec<String>, [String; 3], [String; 3]) {
        let folders = ["a", "b", "d"].map(|x| x.repeat(32));
        let [a, b, d] = folders.clone();
        let notes = ["1", "2", "3"].map(|x| x.repeat(32));
        let [n_a, n_b, n_d] = notes.clone();
        let contents = vec![
            format!("A\n\nid: {a}\nparent_id: \ntype_: 2"),
            format!("B\n\nid: {b}\nparent_id: {a}\ntype_: 2"),
            format!("D\n\nid: {d}\nparent_id: \ntype_: 2"),
            serialize::new_note_md(&n_a, &a, "note a", "body a", false, 1),
            serialize::new_note_md(&n_b, &b, "note b", "body b", false, 2),
            serialize::new_note_md(&n_d, &d, "note d", "body d", false, 3),
        ];
        (contents, folders, notes)
    }

    fn state_with_lib(cfg: AuthConfig, contents: Vec<String>) -> Arc<AppState> {
        let state = state_with_auth(cfg);
        let (lib, _) = Library::from_contents(contents);
        *state.library.write().unwrap() = lib;
        state
    }

    #[tokio::test]
    async fn read_only_blocks_writes_allows_reads_and_config() {
        // 写方法 → 403（守卫拦截，未触达 handler）
        assert_eq!(status_of(state_with_read_only(true), "POST", "/api/notes", "{}").await, StatusCode::FORBIDDEN);
        assert_eq!(status_of(state_with_read_only(true), "DELETE", "/api/notes/abc", "").await, StatusCode::FORBIDDEN);
        assert_eq!(status_of(state_with_read_only(true), "PUT", "/api/notes/abc", "{}").await, StatusCode::FORBIDDEN);
        assert_eq!(status_of(state_with_read_only(true), "POST", "/api/folders", "{}").await, StatusCode::FORBIDDEN);
        assert_eq!(status_of(state_with_read_only(true), "PUT", "/api/folders/abc", "{}").await, StatusCode::FORBIDDEN);
        assert_eq!(status_of(state_with_read_only(true), "DELETE", "/api/folders/abc", "").await, StatusCode::FORBIDDEN);
        // 读方法放行（空库 → 200）
        assert_eq!(status_of(state_with_read_only(true), "GET", "/api/folders", "").await, StatusCode::OK);
        // /api/config 豁免（PUT 不应被守卫拦成 403）
        assert_ne!(
            status_of(state_with_read_only(true), "PUT", "/api/config", "{\"source_type\":\"\"}").await,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn locale_persist_round_trip_and_read_only_exempt() {
        let state = state_with_read_only(false);
        // 默认回落 en
        let body = body_json(send(state.clone(), "GET", "/api/locale", "", None).await).await;
        assert_eq!(body["locale"], "en");
        // 写入 fr → 204，回读 fr
        let resp = send(state.clone(), "PUT", "/api/locale", "{\"locale\":\"fr\"}", None).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        let body = body_json(send(state, "GET", "/api/locale", "", None).await).await;
        assert_eq!(body["locale"], "fr");
        // 只读态豁免（UI 偏好，不被 guard_read_only 拦成 403）
        assert_ne!(
            status_of(state_with_read_only(true), "PUT", "/api/locale", "{\"locale\":\"ja\"}").await,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn non_read_only_does_not_block_writes() {
        // 非只读：写方法不再被守卫拦截（无存储 → 走到 handler 返回 503，而非 403）
        let st = status_of(state_with_read_only(false), "POST", "/api/notes", "{\"parent_id\":\"x\"}").await;
        assert_ne!(st, StatusCode::FORBIDDEN);
        assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE);
    }

    /// 未设访问密码：一切照常（写不被鉴权拦；无存储 → 503 而非 401）。
    #[tokio::test]
    async fn no_password_leaves_everything_open() {
        let state = state_with_auth(auth_config(None, false, "none", &[]));
        assert_eq!(
            send(state.clone(), "POST", "/api/notes", "{\"parent_id\":\"x\"}", None).await.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(send(state.clone(), "GET", "/api/config", "", None).await.status(), StatusCode::OK);
        // 未设密码时不能登录
        assert_eq!(
            send(state, "POST", "/api/auth/login", "{\"password\":\"x\"}", None).await.status(),
            StatusCode::BAD_REQUEST
        );
    }

    /// 设了密码：无 token 写 → 401；登录取 token；带 token 写不再 401；登出使 token 失效。
    #[tokio::test]
    async fn auth_gates_writes_and_login_issues_token() {
        let state = state_with_auth(auth_config(Some("open sesame"), false, "none", &[]));
        // 无 token 写 → 401
        assert_eq!(
            send(state.clone(), "POST", "/api/notes", "{\"parent_id\":\"x\"}", None).await.status(),
            StatusCode::UNAUTHORIZED
        );
        // 错密码 → 401
        assert_eq!(
            send(state.clone(), "POST", "/api/auth/login", "{\"password\":\"nope\"}", None).await.status(),
            StatusCode::UNAUTHORIZED
        );
        // 正确密码 → 200 + 64hex token
        let resp = send(state.clone(), "POST", "/api/auth/login", "{\"password\":\"open sesame\"}", None).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let token = body_json(resp).await["token"].as_str().unwrap().to_string();
        assert_eq!(token.len(), 64);
        // 带 token 写 → 不再 401（无存储 → 503）
        assert_eq!(
            send(state.clone(), "POST", "/api/notes", "{\"parent_id\":\"x\"}", Some(&token)).await.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        // 机密读 /api/config：匿名 401、带 token 200
        assert_eq!(send(state.clone(), "GET", "/api/config", "", None).await.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(send(state.clone(), "GET", "/api/config", "", Some(&token)).await.status(), StatusCode::OK);
        // 资源清单机密读同理
        assert_eq!(send(state.clone(), "GET", "/api/resources", "", None).await.status(), StatusCode::UNAUTHORIZED);
        // 登出后 token 失效
        assert_eq!(
            send(state.clone(), "POST", "/api/auth/logout", "", Some(&token)).await.status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            send(state, "POST", "/api/notes", "{\"parent_id\":\"x\"}", Some(&token)).await.status(),
            StatusCode::UNAUTHORIZED
        );
    }

    /// 匿名改访问控制设置被拦（PUT /api/auth/settings 非豁免写）；只读态登录端点不被 403。
    #[tokio::test]
    async fn anon_cannot_change_auth_settings_but_can_login_under_read_only() {
        let state = state_with_auth(auth_config(Some("pw"), false, "none", &[]));
        // 匿名 PUT /api/auth/settings → 401（否则可清密码绕过）
        assert_eq!(
            send(state.clone(), "PUT", "/api/auth/settings", "{\"clear_password\":true}", None).await.status(),
            StatusCode::UNAUTHORIZED
        );
        // 只读 + 有密码：登录端点不被只读守卫 403（返回 401 因密码错，而非 403）
        let ro = state_full(true, auth_config(Some("pw"), false, "none", &[]));
        assert_eq!(
            send(ro, "POST", "/api/auth/login", "{\"password\":\"wrong\"}", None).await.status(),
            StatusCode::UNAUTHORIZED
        );
    }

    /// /api/status 带 auth 字段：未设密码 / 已设未登录 / 已设已登录。
    #[tokio::test]
    async fn status_reports_auth_fields() {
        let open = body_json(send(state_with_auth(auth_config(None, false, "none", &[])), "GET", "/api/status", "", None).await).await;
        assert_eq!(open["auth_enabled"], false);
        assert_eq!(open["authenticated"], false);

        let prot = state_with_auth(auth_config(Some("pw"), true, "none", &[]));
        let anon = body_json(send(prot.clone(), "GET", "/api/status", "", None).await).await;
        assert_eq!(anon["auth_enabled"], true);
        assert_eq!(anon["authenticated"], false);
        assert_eq!(anon["passwordless_read"], true);

        let token = body_json(send(prot.clone(), "POST", "/api/auth/login", "{\"password\":\"pw\"}", None).await)
            .await["token"]
            .as_str()
            .unwrap()
            .to_string();
        let authed = body_json(send(prot, "GET", "/api/status", "", Some(&token)).await).await;
        assert_eq!(authed["authenticated"], true);
    }

    /// 设置描述符：分区顺序固定、带字段/当前值/动作；plugins=None → 无 AI 段。
    #[tokio::test]
    async fn settings_schema_lists_sections_in_order() {
        // 未设密码 → 人人 Full，可读描述符
        let state = state_with_auth(auth_config(None, false, "none", &[]));
        let body = body_json(send(state, "GET", "/api/settings/schema", "", None).await).await;
        let sections = body["sections"].as_array().unwrap();
        let ids: Vec<&str> = sections.iter().map(|s| s["id"].as_str().unwrap()).collect();
        // plugins=None → 无 ai 段；顺序固定。mcp 段只在 --features mcp 构建里出现。
        let expected: &[&str] = if crate::mcp::ENABLED {
            &["data-source", "access-control", "mcp", "appearance", "editor"]
        } else {
            &["data-source", "access-control", "appearance", "editor"]
        };
        assert_eq!(ids, expected);
        // 数据源段：带字段 / 当前值 / connect 动作
        let ds = &sections[0];
        assert!(ds["fields"].as_array().unwrap().iter().any(|f| f["key"] == "source_type"));
        assert_eq!(ds["values"]["read_only"], false);
        assert_eq!(ds["values"]["source_type"], "local"); // 空配置回显默认 local
        assert_eq!(ds["actions"][0]["request"]["url"], "/api/config");
        assert_eq!(ds["actions"][0]["request"]["convention"], "config-result");
        // 访问控制段：未设密码 → password_set=false，且绝不回哈希
        let ac = sections.iter().find(|s| s["id"] == "access-control").unwrap();
        assert_eq!(ac["values"]["password_set"], false);
        assert!(ac["values"].get("password_hash").is_none());
        // client 作用域段（外观 / 编辑器）无 values
        let appearance = sections.iter().find(|s| s["id"] == "appearance").unwrap();
        assert_eq!(appearance["scope"], "client");
        assert!(appearance.get("values").is_none());
    }

    /// MCP 段：默认开、下发的 endpoint/command 是**带占位符的模板**（服务端不知道客户端
    /// 从哪个地址访问、更不该知道浏览器里的 token，故不能在这里拼死），工具清单非空。
    #[cfg(feature = "mcp")]
    #[tokio::test]
    async fn settings_schema_exposes_mcp_section() {
        let state = state_with_auth(auth_config(None, false, "none", &[]));
        let body = body_json(send(state.clone(), "GET", "/api/settings/schema", "", None).await).await;
        let mcp = body["sections"].as_array().unwrap().iter().find(|s| s["id"] == "mcp").unwrap().clone();

        assert_eq!(mcp["values"]["enabled"], true); // 未设置过 → 默认开
        assert_eq!(mcp["values"]["api_key_set"], false); // 未生成过 key
        let endpoint = mcp["values"]["endpoint"].as_str().unwrap();
        let command = mcp["values"]["command"].as_str().unwrap();
        assert_eq!(endpoint, "{origin}/mcp");
        assert!(command.contains("{origin}/mcp") && command.contains("{header}"), "命令应留占位符: {command}");
        assert!(!command.contains("Bearer "), "没配 key 时服务端不该把 token 拼进命令");
        let tools = mcp["values"]["tools"].as_str().unwrap();
        assert!(tools.contains("search_notes") && tools.contains("delete_folder"), "工具清单: {tools}");
        assert_eq!(mcp["actions"][0]["request"]["url"], "/api/mcp/config");

        // 关掉 → 描述符回显 false（开关落 config.db）
        assert_eq!(
            send(state.clone(), "PUT", "/api/mcp/config", r#"{"enabled":false}"#, None).await.status(),
            StatusCode::NO_CONTENT
        );
        let body = body_json(send(state, "GET", "/api/settings/schema", "", None).await).await;
        let mcp = body["sections"].as_array().unwrap().iter().find(|s| s["id"] == "mcp").unwrap().clone();
        assert_eq!(mcp["values"]["enabled"], false);
    }

    /// MCP 独立 API key：生成 → 描述符回显并把完整命令拼好 → 端点只认这把 key
    /// （不带 / 带错 / 拿有效会话 token 都是 401）→ 清除后回落既有行为。
    #[cfg(feature = "mcp")]
    #[tokio::test]
    async fn mcp_api_key_locks_the_endpoint_independently_of_sessions() {
        let state = state_with_auth(auth_config(Some("pw"), true, "none", &[]));
        let ping = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        let session = state.auth.issue_token(); // 一枚有效的浏览器会话 token

        // 没配 key：会话 token 可用（既有行为）
        assert_ne!(
            send(state.clone(), "POST", "/mcp", ping, Some(&session)).await.status(),
            StatusCode::UNAUTHORIZED
        );

        // 生成 key（只发动作，不带 enabled → 开关不受影响）
        assert_eq!(
            send(state.clone(), "PUT", "/api/mcp/config", r#"{"regenerate_key":true}"#, Some(&session)).await.status(),
            StatusCode::NO_CONTENT
        );
        let key = state.config.lock().unwrap().mcp_api_key();
        assert!(key.starts_with(crate::auth::MCP_KEY_PREFIX) && key.len() > 40, "key 形如 jasper_mcp_<64hex>: {key}");

        // 描述符回显 key，并把完整命令拼好（key 本就在服务端，这个端点又是机密读）
        let body = body_json(send(state.clone(), "GET", "/api/settings/schema", "", Some(&session)).await).await;
        let mcp = body["sections"].as_array().unwrap().iter().find(|s| s["id"] == "mcp").unwrap().clone();
        assert_eq!(mcp["values"]["api_key_set"], true);
        assert_eq!(mcp["values"]["api_key"], key);
        let command = mcp["values"]["command"].as_str().unwrap();
        assert!(command.contains(&format!("Bearer {key}")), "命令应带上 key: {command}");
        assert!(!command.contains("{header}"), "配了 key 就不该再留占位符");

        // 端点只认这把 key
        assert_eq!(send(state.clone(), "POST", "/mcp", ping, None).await.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            send(state.clone(), "POST", "/mcp", ping, Some("jasper_mcp_wrong")).await.status(),
            StatusCode::UNAUTHORIZED
        );
        // 关键：有效的会话 token 也进不去——key 是给 MCP 单独上的锁
        assert_eq!(
            send(state.clone(), "POST", "/mcp", ping, Some(&session)).await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_ne!(
            send(state.clone(), "POST", "/mcp", ping, Some(&key)).await.status(),
            StatusCode::UNAUTHORIZED
        );

        // key 不随会话失效：吊销全部会话（等价于服务重启/改密码）后照常可用
        state.auth.revoke_all();
        assert_ne!(
            send(state.clone(), "POST", "/mcp", ping, Some(&key)).await.status(),
            StatusCode::UNAUTHORIZED
        );

        // 重新生成 → 旧 key 立即失效
        let session2 = state.auth.issue_token();
        send(state.clone(), "PUT", "/api/mcp/config", r#"{"regenerate_key":true}"#, Some(&session2)).await;
        let key2 = state.config.lock().unwrap().mcp_api_key();
        assert_ne!(key, key2);
        assert_eq!(send(state.clone(), "POST", "/mcp", ping, Some(&key)).await.status(), StatusCode::UNAUTHORIZED);

        // 清除 → 回落既有行为（会话 token 又能用了）
        send(state.clone(), "PUT", "/api/mcp/config", r#"{"clear_key":true}"#, Some(&session2)).await;
        assert!(state.config.lock().unwrap().mcp_api_key().is_empty());
        assert_ne!(
            send(state.clone(), "POST", "/mcp", ping, Some(&session2)).await.status(),
            StatusCode::UNAUTHORIZED
        );

        // 自相矛盾的请求不猜意图
        assert_eq!(
            send(state.clone(), "PUT", "/api/mcp/config", r#"{"regenerate_key":true,"clear_key":true}"#, Some(&session2)).await.status(),
            StatusCode::BAD_REQUEST
        );
    }

    /// 管理端点本身不豁免守卫——否则谁都能把 key 换掉。
    #[cfg(feature = "mcp")]
    #[tokio::test]
    async fn mcp_config_endpoint_is_guarded_like_any_other_setting() {
        // 设了密码的匿名请求 → 401
        let state = state_with_auth(auth_config(Some("pw"), true, "none", &[]));
        assert_eq!(
            send(state, "PUT", "/api/mcp/config", r#"{"regenerate_key":true}"#, None).await.status(),
            StatusCode::UNAUTHORIZED
        );
        // 只读模式 → 403
        assert_eq!(
            status_of(state_with_read_only(true), "PUT", "/api/mcp/config", r#"{"enabled":false}"#).await,
            StatusCode::FORBIDDEN
        );
    }

    /// MCP 端点从两道「按 HTTP 方法」的守卫里豁免（否则只读态连 tools/list 都 403、
    /// 匿名连工具清单都列不出），门控下沉到工具层——这里只验证守卫确实没拦下它。
    /// 关掉开关后整个端点 503。
    #[cfg(feature = "mcp")]
    #[tokio::test]
    async fn mcp_endpoint_bypasses_method_guards_and_honors_switch() {
        // 只读 + 设了密码的匿名请求：若被守卫拦，会是 403/401；豁免后应走到 MCP 自身的处理
        let state = state_full(true, auth_config(Some("pw"), false, "none", &[]));
        let st = send(state.clone(), "POST", "/mcp", r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#, None)
            .await
            .status();
        assert_ne!(st, StatusCode::FORBIDDEN, "只读守卫不该拦 MCP（一切 MCP 请求都是 POST）");
        assert_ne!(st, StatusCode::UNAUTHORIZED, "鉴权守卫不该拦 MCP（门控在工具层）");

        // 关掉开关 → 端点整体 503（config 写入不受只读守卫影响：/api/mcp/config 需 Full，
        // 故这里换一个未设密码、非只读的 state 来验证）
        let state = state_with_auth(auth_config(None, false, "none", &[]));
        assert_eq!(
            send(state.clone(), "PUT", "/api/mcp/config", r#"{"enabled":false}"#, None).await.status(),
            StatusCode::NO_CONTENT
        );
        let st = send(state, "POST", "/mcp", r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#, None)
            .await
            .status();
        assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE);
    }

    /// 设置描述符是机密读：设了密码后匿名 401，登录后 200 且回显访问控制当前值。
    #[tokio::test]
    async fn settings_schema_is_secret_read_gated() {
        let cfg = auth_config(Some("pw"), true, "whitelist", &["a".repeat(32).as_str()]);
        let state = state_with_auth(cfg.clone());
        // 描述符从 ConfigStore 读访问控制当前值（生产里 store 与运行态由 save+reload 同步）；
        // 测试助手只填了运行态 AuthState，这里补填 store 以断言回显。
        state.config.lock().unwrap().save_auth_config(&cfg).unwrap();
        // 匿名 → 401
        assert_eq!(
            send(state.clone(), "GET", "/api/settings/schema", "", None).await.status(),
            StatusCode::UNAUTHORIZED
        );
        // 登录后 → 200
        let token = body_json(send(state.clone(), "POST", "/api/auth/login", "{\"password\":\"pw\"}", None).await)
            .await["token"]
            .as_str()
            .unwrap()
            .to_string();
        let resp = send(state, "GET", "/api/settings/schema", "", Some(&token)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        let ac = body["sections"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == "access-control")
            .unwrap()
            .clone();
        assert_eq!(ac["values"]["password_set"], true);
        assert_eq!(ac["values"]["passwordless_read"], true);
        assert_eq!(ac["values"]["list_mode"], "whitelist");
    }

    /// 匿名读范围过滤：passwordless 关 → 什么都看不到。
    #[tokio::test]
    async fn anonymous_private_sees_nothing() {
        let (contents, _f, notes) = sample_contents();
        let state = state_with_lib(auth_config(Some("pw"), false, "none", &[]), contents);
        // folders 空
        assert_eq!(body_json(send(state.clone(), "GET", "/api/folders", "", None).await).await, serde_json::json!([]));
        // 某笔记 detail → 404
        assert_eq!(
            send(state.clone(), "GET", &format!("/api/notes/{}", notes[0]), "", None).await.status(),
            StatusCode::NOT_FOUND
        );
        // search 空
        assert_eq!(body_json(send(state, "GET", "/api/search?q=note", "", None).await).await, serde_json::json!([]));
    }

    /// 匿名读范围过滤：passwordless 开 + none → 全库可读。
    #[tokio::test]
    async fn anonymous_passwordless_none_sees_all() {
        let (contents, folders, notes) = sample_contents();
        let state = state_with_lib(auth_config(Some("pw"), true, "none", &[]), contents);
        let f = body_json(send(state.clone(), "GET", "/api/folders", "", None).await).await;
        // 顶层 a、d 两个根
        assert_eq!(f.as_array().unwrap().len(), 2);
        // 笔记 detail 可读
        assert_eq!(
            send(state.clone(), "GET", &format!("/api/notes/{}", notes[2]), "", None).await.status(),
            StatusCode::OK
        );
        // search 命中 3 篇
        let s = body_json(send(state.clone(), "GET", "/api/search?q=body", "", None).await).await;
        assert_eq!(s.as_array().unwrap().len(), 3);
        let _ = folders;
    }

    /// 匿名读范围过滤：passwordless 开 + whitelist[a] → 仅 a 子树（a,b）可见，d 不可见。
    #[tokio::test]
    async fn anonymous_whitelist_scopes_to_subtree() {
        let (contents, folders, notes) = sample_contents();
        let [a, b, d] = folders;
        let [n_a, n_b, n_d] = notes;
        let state = state_with_lib(auth_config(Some("pw"), true, "whitelist", &[&a]), contents);
        // folders：只有 a（含子 b）
        let f = body_json(send(state.clone(), "GET", "/api/folders", "", None).await).await;
        let arr = f.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], a);
        assert_eq!(arr[0]["children"].as_array().unwrap()[0]["id"], b);
        // a、b 的笔记可读；d 的笔记 404
        assert_eq!(send(state.clone(), "GET", &format!("/api/notes/{n_a}"), "", None).await.status(), StatusCode::OK);
        assert_eq!(send(state.clone(), "GET", &format!("/api/notes/{n_b}"), "", None).await.status(), StatusCode::OK);
        assert_eq!(send(state.clone(), "GET", &format!("/api/notes/{n_d}"), "", None).await.status(), StatusCode::NOT_FOUND);
        // d 笔记本的列表为空
        assert_eq!(
            body_json(send(state.clone(), "GET", &format!("/api/notes?folder={d}"), "", None).await).await,
            serde_json::json!([])
        );
        // search 只命中 a、b 两篇
        let s = body_json(send(state, "GET", "/api/search?q=body", "", None).await).await;
        assert_eq!(s.as_array().unwrap().len(), 2);
    }

    /// 匿名读范围过滤：passwordless 开 + blacklist[a] → a 子树被挡，d 可见。
    #[tokio::test]
    async fn anonymous_blacklist_is_complement() {
        let (contents, folders, notes) = sample_contents();
        let [a, _b, d] = folders;
        let [n_a, _n_b, n_d] = notes;
        let state = state_with_lib(auth_config(Some("pw"), true, "blacklist", &[&a]), contents);
        // folders：只有 d
        let f = body_json(send(state.clone(), "GET", "/api/folders", "", None).await).await;
        let arr = f.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], d);
        // a 笔记 404、d 笔记 200
        assert_eq!(send(state.clone(), "GET", &format!("/api/notes/{n_a}"), "", None).await.status(), StatusCode::NOT_FOUND);
        assert_eq!(send(state, "GET", &format!("/api/notes/{n_d}"), "", None).await.status(), StatusCode::OK);
    }

    /// 资源二进制的 ACL：resource→note→folder 权限链路。白名单/黑名单只放行被可见笔记本内笔记
    /// 引用过的资源；孤儿资源（无引用）受限时一律拒绝；多篇引用取并集（有一篇可见即放行）；
    /// passwordless 关则恒 404；Full 访问不受影响。
    #[tokio::test]
    async fn anonymous_resource_acl_scopes_by_referencing_note_folder() {
        let visible_f = "a".repeat(32);
        let hidden_f = "b".repeat(32);
        let n_visible = "1".repeat(32);
        let n_hidden = "2".repeat(32);
        let n_visible2 = "3".repeat(32);
        let r_in_visible = "c".repeat(32); // 只被可见笔记本内的笔记引用
        let r_in_hidden = "d".repeat(32); // 只被隐藏笔记本内的笔记引用
        let r_shared = "e".repeat(32); // 同时被可见/隐藏笔记引用（并集：至少一篇可见即放行）
        let r_orphan = "f".repeat(32); // 无笔记引用

        let dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(crate::storage::local::LocalStorage::new(dir.path())) as Arc<dyn StorageBackend>;
        for (rid, body) in [
            (&r_in_visible, b"visible-bytes" as &[u8]),
            (&r_in_hidden, b"hidden-bytes"),
            (&r_shared, b"shared-bytes"),
        ] {
            storage.put_resource(rid, body).unwrap();
        }

        // n_hidden 引用 r_in_hidden；另有一篇隐藏笔记本内的笔记也引用 r_shared（并集测试的另一半）。
        let build = || {
            vec![
                format!("Visible\n\nid: {visible_f}\nparent_id: \ntype_: 2"),
                format!("Hidden\n\nid: {hidden_f}\nparent_id: \ntype_: 2"),
                serialize::new_note_md(&n_visible, &visible_f, "in visible", &format!("![x](:/{r_in_visible})"), false, 1),
                serialize::new_note_md(&n_hidden, &hidden_f, "in hidden", &format!("![x](:/{r_in_hidden})"), false, 2),
                serialize::new_note_md(&n_visible2, &visible_f, "also visible", &format!("![x](:/{r_shared})"), false, 3),
                serialize::new_note_md(&"9".repeat(32), &hidden_f, "hidden shares too", &format!("![x](:/{r_shared})"), false, 4),
            ]
        };

        let state = state_with_lib(auth_config(Some("pw"), true, "whitelist", &[&visible_f]), build());
        *state.storage.write().unwrap() = Some(storage.clone());

        // 仅被可见笔记本引用 → 200 + 正确字节
        let resp = send(state.clone(), "GET", &format!("/api/resources/{r_in_visible}"), "", None).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        assert_eq!(&bytes[..], b"visible-bytes");

        // 仅被隐藏笔记本引用 → 404（即使二进制本身在存储里存在，前面短路拒绝）
        assert_eq!(
            send(state.clone(), "GET", &format!("/api/resources/{r_in_hidden}"), "", None).await.status(),
            StatusCode::NOT_FOUND
        );

        // 同时被可见/隐藏笔记引用 → 并集放行，200
        assert_eq!(
            send(state.clone(), "GET", &format!("/api/resources/{r_shared}"), "", None).await.status(),
            StatusCode::OK
        );

        // 孤儿资源（无引用）在受限范围下没有权限链路 → 404
        assert_eq!(
            send(state.clone(), "GET", &format!("/api/resources/{r_orphan}"), "", None).await.status(),
            StatusCode::NOT_FOUND
        );

        // 黑名单（挡 hidden_f）：可见资源同上，隐藏资源仍拒绝
        let bl_state = state_with_lib(auth_config(Some("pw"), true, "blacklist", &[&hidden_f]), build());
        *bl_state.storage.write().unwrap() = Some(storage.clone());
        assert_eq!(
            send(bl_state.clone(), "GET", &format!("/api/resources/{r_in_visible}"), "", None).await.status(),
            StatusCode::OK
        );
        assert_eq!(
            send(bl_state, "GET", &format!("/api/resources/{r_in_hidden}"), "", None).await.status(),
            StatusCode::NOT_FOUND
        );

        // passwordless 关（整站私有）：即使资源被引用，也一律 404
        let priv_state = state_with_lib(auth_config(Some("pw"), false, "none", &[]), build());
        *priv_state.storage.write().unwrap() = Some(storage.clone());
        assert_eq!(
            send(priv_state, "GET", &format!("/api/resources/{r_in_visible}"), "", None).await.status(),
            StatusCode::NOT_FOUND
        );

        // Full（已登录）：Scope::All 跳过 ACL 检查——r_in_hidden 在匿名 whitelist 下 404（上面已验证），
        // 带 token 则放行，证明限制只对匿名生效。
        let token = body_json(send(state.clone(), "POST", "/api/auth/login", "{\"password\":\"pw\"}", None).await)
            .await["token"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(
            send(state, "GET", &format!("/api/resources/{r_in_hidden}"), "", Some(&token)).await.status(),
            StatusCode::OK
        );
    }

    /// 标签读端点也按范围过滤：篇数只数可见笔记、tag_notes 过滤、私有笔记的 tags 404、私有态全空。
    #[tokio::test]
    async fn anonymous_tag_endpoints_filtered() {
        let pub_f = "a".repeat(32);
        let priv_f = "b".repeat(32);
        let n_pub = "1".repeat(32);
        let n_priv = "2".repeat(32);
        let tag = "c".repeat(32);
        // 一个标签同时打在公开笔记与私有笔记上
        let build = || {
            vec![
                format!("Public\n\nid: {pub_f}\nparent_id: \ntype_: 2"),
                format!("Private\n\nid: {priv_f}\nparent_id: \ntype_: 2"),
                serialize::new_note_md(&n_pub, &pub_f, "pub note", "x", false, 1),
                serialize::new_note_md(&n_priv, &priv_f, "priv note", "x", false, 2),
                format!("mytag\n\nid: {tag}\nparent_id: \ntype_: 5"),
                format!("id: {}\nnote_id: {n_pub}\ntag_id: {tag}\ntype_: 6", "d".repeat(32)),
                format!("id: {}\nnote_id: {n_priv}\ntag_id: {tag}\ntype_: 6", "e".repeat(32)),
            ]
        };

        // passwordless + whitelist(Public)：标签可见但篇数只数公开那 1 篇
        let st = state_with_lib(auth_config(Some("pw"), true, "whitelist", &[&pub_f]), build());
        let tags = body_json(send(st.clone(), "GET", "/api/tags", "", None).await).await;
        assert_eq!(tags.as_array().unwrap().len(), 1);
        assert_eq!(tags[0]["id"], tag);
        assert_eq!(tags[0]["note_count"], 1);
        // tag_notes 只含公开笔记
        let tn = body_json(send(st.clone(), "GET", &format!("/api/tags/{tag}/notes"), "", None).await).await;
        assert_eq!(tn.as_array().unwrap().len(), 1);
        assert_eq!(tn[0]["id"], n_pub);
        // 某笔记的标签：公开 200、私有 404
        assert_eq!(send(st.clone(), "GET", &format!("/api/notes/{n_pub}/tags"), "", None).await.status(), StatusCode::OK);
        assert_eq!(send(st, "GET", &format!("/api/notes/{n_priv}/tags"), "", None).await.status(), StatusCode::NOT_FOUND);

        // passwordless 关（整站私有）：标签相关端点全空
        let priv_st = state_with_lib(auth_config(Some("pw"), false, "none", &[]), build());
        assert_eq!(body_json(send(priv_st.clone(), "GET", "/api/tags", "", None).await).await, serde_json::json!([]));
        assert_eq!(
            body_json(send(priv_st, "GET", &format!("/api/tags/{tag}/notes"), "", None).await).await,
            serde_json::json!([])
        );
    }

    /// SSE 端点：订阅后广播的事件应以 `event: change` + ChangeEvent JSON 流出。
    #[tokio::test]
    async fn events_sse_streams_changes() {
        use tokio_stream::StreamExt as _;

        let state = state_with_read_only(false);
        let req = Request::builder().method("GET").uri("/api/events").body(Body::empty()).unwrap();
        let resp = router(state.clone()).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .starts_with("text/event-stream"));

        // handler 已在 oneshot 里订阅；现在发事件、再读第一帧
        state.events.note_upserted("abc123");
        let mut body = resp.into_body().into_data_stream();
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), body.next())
            .await
            .expect("5s 内应收到事件帧")
            .expect("流不应结束")
            .expect("帧不应出错");
        let text = String::from_utf8_lossy(&frame);
        assert!(text.contains("event: change"), "SSE 帧: {text}");
        assert!(text.contains(r#""kind":"note""#) && text.contains("abc123"), "SSE 帧: {text}");
    }

    /// 设了访问密码时：EventSource 带不了 Authorization 头，故 `/api/events?token=` 也认 token。
    /// 不带 token 的订阅是受限匿名 → 事件被粗化成 library reload；带上有效 token → 细粒度事件。
    #[tokio::test]
    async fn events_sse_accepts_query_token() {
        use tokio_stream::StreamExt as _;

        async fn first_frame(state: Arc<AppState>, uri: &str) -> String {
            let req = Request::builder().method("GET").uri(uri).body(Body::empty()).unwrap();
            let resp = router(state.clone()).oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            state.events.note_upserted("abc123");
            let mut body = resp.into_body().into_data_stream();
            let frame = tokio::time::timeout(std::time::Duration::from_secs(5), body.next())
                .await
                .expect("5s 内应收到事件帧")
                .expect("流不应结束")
                .expect("帧不应出错");
            String::from_utf8_lossy(&frame).into_owned()
        }

        // 有密码 + 不许无密码阅读 → 匿名可见范围为空（受限）
        let state = state_with_auth(auth_config(Some("pw"), false, "none", &[]));
        let resp = send(state.clone(), "POST", "/api/auth/login", "{\"password\":\"pw\"}", None).await;
        let token = body_json(resp).await["token"].as_str().unwrap().to_string();

        let anon = first_frame(state.clone(), "/api/events").await;
        assert!(anon.contains(r#""kind":"library""#) && !anon.contains("abc123"), "匿名帧: {anon}");

        let authed = first_frame(state.clone(), &format!("/api/events?token={token}")).await;
        assert!(authed.contains(r#""kind":"note""#) && authed.contains("abc123"), "带 token 帧: {authed}");

        // 无效 token 仍按匿名处理
        let bogus = first_frame(state, "/api/events?token=deadbeef").await;
        assert!(bogus.contains(r#""kind":"library""#), "坏 token 帧: {bogus}");
    }

    /// 保存原语是事件的单一咽喉：写入成功后广播 note upsert（插件直写路径共用此原语）。
    #[tokio::test]
    async fn persist_note_emits_change_event() {
        let dir = tempfile::tempdir().unwrap();
        let storage = crate::storage::local::LocalStorage::new(dir.path());
        let state = state_with_read_only(false);
        let mut rx = state.events.subscribe();

        let id = "a".repeat(32);
        let content = serialize::new_note_md(&id, "", "标题", "正文", false, 0);
        persist_note_blocking(&state.library, &storage, &state.events, &id, &content).unwrap();

        let ev = rx.try_recv().expect("写入后应有事件");
        assert_eq!((ev.kind, ev.op), ("note", "upsert"));
        assert_eq!(ev.id, id);
    }

    async fn get_json(state: Arc<AppState>, uri: &str) -> serde_json::Value {
        let req = Request::builder().method("GET").uri(uri).body(Body::empty()).unwrap();
        let resp = router(state).oneshot(req).await.unwrap();
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// /api/tags 列表（含篇数）与 /api/tags/{id}/notes 过滤。
    #[tokio::test]
    async fn tags_endpoints_list_and_filter() {
        let state = state_with_read_only(false);
        let n1 = "1".repeat(32);
        let n2 = "2".repeat(32);
        let tag = "a".repeat(32);
        let contents = vec![
            serialize::new_note_md(&n1, "", "笔记一", "正文", false, 100),
            serialize::new_note_md(&n2, "", "笔记二", "正文", false, 200),
            // 标签（type_=5）+ 关联（type_=6）：仅 n1 打了该标签
            format!("待办\n\nid: {tag}\nparent_id: \ntype_: 5"),
            format!("id: {}\nnote_id: {n1}\ntag_id: {tag}\ntype_: 6", "b".repeat(32)),
        ];
        let (lib, _) = Library::from_contents(contents);
        *state.library.write().unwrap() = lib;

        let tags = get_json(state.clone(), "/api/tags").await;
        let arr = tags.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], tag);
        assert_eq!(arr[0]["title"], "待办");
        assert_eq!(arr[0]["note_count"], 1);

        // 该标签下只有 n1
        let notes = get_json(state.clone(), &format!("/api/tags/{tag}/notes")).await;
        let narr = notes.as_array().unwrap();
        assert_eq!(narr.len(), 1);
        assert_eq!(narr[0]["id"], n1);

        // 未知标签 → 空列表（非 404）
        let empty = get_json(state.clone(), &format!("/api/tags/{}/notes", "f".repeat(32))).await;
        assert_eq!(empty, serde_json::json!([]));
    }

    async fn send_json(
        state: Arc<AppState>,
        method: &str,
        uri: &str,
        body: &str,
    ) -> (StatusCode, serde_json::Value) {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = router(state).oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let val = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, val)
    }

    /// 打标签全链路：新建/复用（不区分大小写）/幂等 + 去标签 + Joplin 兼容落盘。
    #[tokio::test]
    async fn note_tag_add_reuse_and_remove() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(crate::storage::local::LocalStorage::new(dir.path())) as Arc<dyn StorageBackend>;
        let state = state_with_read_only(false);
        *state.storage.write().unwrap() = Some(storage.clone());
        let note_id = "1".repeat(32);
        state
            .library
            .write()
            .unwrap()
            .upsert_note(&serialize::new_note_md(&note_id, "", "N", "body", false, 100))
            .unwrap();

        let tags_uri = format!("/api/notes/{note_id}/tags");

        // 打标签 "Work" → 新建标签 + 关联
        let (st, body) = send_json(state.clone(), "POST", &tags_uri, r#"{"title":"Work"}"#).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(body.as_array().unwrap().len(), 1);
        assert_eq!(body[0]["title"], "Work");
        let work_id = body[0]["id"].as_str().unwrap().to_string();

        // 大小写不同 + 幂等：复用同一标签，笔记标签数不增
        let (_, body) = send_json(state.clone(), "POST", &tags_uri, r#"{"title":"  work "}"#).await;
        assert_eq!(body.as_array().unwrap().len(), 1);

        // /api/tags 里 Work 篇数为 1
        let (_, tags) = send_json(state.clone(), "GET", "/api/tags", "").await;
        let work = tags.as_array().unwrap().iter().find(|t| t["title"] == "Work").unwrap();
        assert_eq!(work["note_count"], 1);

        // Joplin 兼容：落盘的 note_tag 文件是纯元数据（无标题、type_=6），标签文件 type_=5
        let files: Vec<String> = storage.list_items().unwrap().into_iter().map(|i| i.name).collect();
        let mut saw_note_tag = false;
        let mut saw_tag = false;
        for name in &files {
            if name == &format!("{note_id}.md") {
                continue;
            }
            let content = storage.get_item(name).unwrap();
            if content.contains("\ntype_: 6") {
                saw_note_tag = true;
                assert!(content.starts_with("id: "), "note_tag 应无标题: {content}");
                assert!(content.contains(&format!("note_id: {note_id}")));
            } else if content.contains("\ntype_: 5") {
                saw_tag = true;
                assert!(content.starts_with("Work\n\nid: "), "tag 首行应为标题: {content}");
            }
        }
        assert!(saw_note_tag && saw_tag, "应已落盘 tag + note_tag");

        // 第二个标签
        let (_, body) = send_json(state.clone(), "POST", &tags_uri, r#"{"title":"Idea"}"#).await;
        assert_eq!(body.as_array().unwrap().len(), 2);

        // 去掉 Work → 只剩 Idea；Work 变孤儿（仍在 /api/tags，篇数 0）
        let (st, body) = send_json(state.clone(), "DELETE", &format!("{tags_uri}/{work_id}"), "").await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(body.as_array().unwrap().len(), 1);
        assert_eq!(body[0]["title"], "Idea");
        let (_, tags) = send_json(state.clone(), "GET", "/api/tags", "").await;
        let work = tags.as_array().unwrap().iter().find(|t| t["title"] == "Work").unwrap();
        assert_eq!(work["note_count"], 0);

        // 空标题 400；不存在的笔记 404
        let (st, _) = send_json(state.clone(), "POST", &tags_uri, r#"{"title":"   "}"#).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        let (st, _) = send_json(state.clone(), "POST", &format!("/api/notes/{}/tags", "9".repeat(32)), r#"{"title":"x"}"#).await;
        assert_eq!(st, StatusCode::NOT_FOUND);
    }

    /// 只读模式拦截打/去标签（守卫按方法拦截，无需触达 handler）。
    #[tokio::test]
    async fn read_only_blocks_tag_writes() {
        let id = "1".repeat(32);
        assert_eq!(
            send_json(state_with_read_only(true), "POST", &format!("/api/notes/{id}/tags"), r#"{"title":"x"}"#).await.0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send_json(state_with_read_only(true), "DELETE", &format!("/api/notes/{id}/tags/{}", "2".repeat(32)), "").await.0,
            StatusCode::FORBIDDEN
        );
    }

    /// 删除笔记本：级联删掉整棵子树（子笔记本 + 其中全部笔记），落盘与内存索引同步；
    /// 兄弟分支与「未分类」笔记不受牵连；未分类根（空 id）与不存在的 id → 404。
    #[tokio::test]
    async fn delete_folder_cascades_subtree() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(crate::storage::local::LocalStorage::new(dir.path())) as Arc<dyn StorageBackend>;
        let state = state_with_read_only(false);
        *state.storage.write().unwrap() = Some(storage.clone());

        // a(root) > b，独立 d(root)；a/b/d 各一篇笔记；外加一篇未分类笔记
        let (contents, [a, b, d], [n_a, n_b, n_d]) = sample_contents();
        let unfiled = "9".repeat(32);
        let mut contents = contents;
        contents.push(serialize::new_note_md(&unfiled, "", "未分类", "body u", false, 4));
        for c in &contents {
            let id = c.lines().find_map(|l| l.strip_prefix("id: ")).unwrap().to_string();
            storage.put_item(&format!("{id}.md"), c).unwrap();
        }
        let (lib, _) = Library::from_contents(contents);
        *state.library.write().unwrap() = lib;

        // 删 a → a、子笔记本 b、两者的笔记全没了
        let mut rx = state.events.subscribe();
        let st = send(state.clone(), "DELETE", &format!("/api/folders/{a}"), "", None).await.status();
        assert_eq!(st, StatusCode::NO_CONTENT);

        let names: HashSet<String> = storage.list_items().unwrap().into_iter().map(|i| i.name).collect();
        for gone in [&a, &b, &n_a, &n_b] {
            assert!(!names.contains(&format!("{gone}.md")), "{gone} 应已从存储删除");
        }
        for kept in [&d, &n_d, &unfiled] {
            assert!(names.contains(&format!("{kept}.md")), "{kept} 不该被牵连");
        }
        {
            let lib = state.library.read().unwrap();
            assert!(!lib.folders.contains_key(&a) && !lib.folders.contains_key(&b));
            assert!(lib.note(&n_a).is_none() && lib.note(&n_b).is_none());
            assert!(lib.folders.contains_key(&d) && lib.note(&n_d).is_some());
            assert_eq!(lib.note_count(""), 1); // 未分类笔记还在
        }
        // 广播了逐条删除事件（条数少于 CASCADE_EVENT_LIMIT），前端据此关掉打开的笔记
        let mut evs = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            evs.push((ev.kind, ev.op, ev.id));
        }
        assert!(evs.contains(&("note", "delete", n_a.clone())));
        assert!(evs.contains(&("folder", "delete", a.clone())));
        assert!(evs.contains(&("folder", "delete", b.clone())));

        // /api/folders 里 a 与 b 都不在了，d 还在
        let nodes = folders_json(state.clone()).await;
        let ids: Vec<&str> = nodes.as_array().unwrap().iter().map(|n| n["id"].as_str().unwrap()).collect();
        assert!(!ids.contains(&a.as_str()) && ids.contains(&d.as_str()));

        // 再删一次（已不存在）→ 404
        let st = send(state.clone(), "DELETE", &format!("/api/folders/{a}"), "", None).await.status();
        assert_eq!(st, StatusCode::NOT_FOUND);
    }

    async fn folders_json(state: Arc<AppState>) -> serde_json::Value {
        let req = Request::builder().method("GET").uri("/api/folders").body(Body::empty()).unwrap();
        let resp = router(state).oneshot(req).await.unwrap();
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// 没有笔记本归属(parent_id=="")的笔记（Bug: 不能看没有分组的笔记）——
    /// /api/folders 应合成一个 id=="" 的节点带上根笔记数，前端据此渲染「未分类笔记」入口。
    #[tokio::test]
    async fn folders_endpoint_synthesizes_unfiled_node_for_root_notes() {
        let state = state_with_read_only(false);

        // 空库：没有未分类笔记 → 不出现合成节点
        assert_eq!(folders_json(state.clone()).await, serde_json::json!([]));

        // 插入一条不属于任何笔记本的笔记
        let id = "b".repeat(32);
        let content = serialize::new_note_md(&id, "", "无归属", "正文", false, 0);
        state.library.write().unwrap().upsert_note(&content).unwrap();

        let nodes = folders_json(state.clone()).await;
        let arr = nodes.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], "");
        assert_eq!(arr[0]["note_count"], 1);
        assert_eq!(arr[0]["children"], serde_json::json!([]));
    }
}
