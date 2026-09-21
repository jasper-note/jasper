//! jasper —— 轻量只读 Joplin 客户端（本地 HTTP 服务 + 浏览器 UI）
//!
//! 启动后扫描本地数据目录 → 内存索引 → 提供 HTTP API + 托管前端 SPA。
//!
//! 用法：jasper [数据源]   （可选，首次也可在浏览器里完成配置）
//!   数据源可以是本地目录路径，或 http(s):// 开头的 WebDAV 地址。
//! 配置持久化在平台配置目录 jasper/config.db；命令行参数仅用于首次引导。
//! 环境变量：
//!   JASPER_SOURCE        引导用数据源（仅当尚无保存配置时生效）
//!   JASPER_WEBDAV_USER   引导用 WebDAV 用户名
//!   JASPER_WEBDAV_PASS   引导用 WebDAV 密码
//!   JASPER_HOST          监听地址（默认 127.0.0.1；局域网/容器设 0.0.0.0）
//!   JASPER_PORT          端口（默认 27583）
//!   JASPER_CONFIG_DIR    配置库目录（默认平台配置目录；容器里挂卷）
//!   JASPER_READ_ONLY     只读模式引导（truthy=1/true/yes/on；仅当尚无保存配置时生效）
//!   JASPER_WEB_DIR       前端静态目录覆盖（设了就从该磁盘目录托管，可热替换前端）；
//!                             不设时：embed 构建用内嵌资源，否则用源码旁 ../web/dist
//!   RUST_LOG              日志级别过滤（tracing_subscriber::EnvFilter 语法，如 `jasper=trace`）；
//!                             不设时默认 `jasper=debug,tower_http=debug,info`

mod api;
mod auth;
mod cache;
mod config;
mod events;
mod indexer;
mod mcp;
mod plugins;
mod storage;
#[cfg(feature = "embed")]
mod web_assets;

// 纯逻辑（数据模型/解析/序列化/索引）来自 jasper-core；以 crate 路径重导出，
// 让既有的 crate::{model,parser,serialize,library} 引用保持不变。
pub use jasper_core::{library, model, parser, serialize};

use anyhow::Result;
use api::AppState;
use config::{ConfigStore, SourceConfig};
use library::Library;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};
use storage::StorageBackend;
use tower_http::services::{ServeDir, ServeFile};

// 环境变量真值判断（1/true/yes/on，大小写不敏感）。
fn env_truthy(key: &str) -> bool {
    std::env::var(key)
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

// 首次引导：把命令行/环境变量里的数据源转成配置（仅当配置库里还没有时使用）。
fn bootstrap_config() -> Option<SourceConfig> {
    let src = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("JASPER_SOURCE").ok())?;
    let read_only = env_truthy("JASPER_READ_ONLY");
    if src.starts_with("http://") || src.starts_with("https://") {
        Some(SourceConfig {
            source_type: "webdav".to_string(),
            webdav_url: src,
            webdav_user: std::env::var("JASPER_WEBDAV_USER").unwrap_or_default(),
            webdav_pass: std::env::var("JASPER_WEBDAV_PASS").unwrap_or_default(),
            read_only,
            ..Default::default()
        })
    } else {
        Some(SourceConfig {
            source_type: "local".to_string(),
            local_path: src,
            read_only,
            ..Default::default()
        })
    }
}

/// 默认日志过滤：可被 RUST_LOG 覆盖。故意比常见的 `info` 更低一档（含 debug），
/// 让 HTTP 请求（tower_http）与本项目内部（jasper）的细节默认就可见，排障不用先设环境变量。
const DEFAULT_LOG_FILTER: &str = "jasper=debug,tower_http=debug,info";

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER));
    tracing_subscriber::fmt().with_env_filter(filter).with_target(true).init();
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let host = std::env::var("JASPER_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = std::env::var("JASPER_PORT").unwrap_or_else(|_| "27583".to_string());

    tracing::info!("jasper starting…");

    // Arc 化：与插件宿主共享同一配置库连接
    let config_store = Arc::new(Mutex::new(ConfigStore::open()?));
    // 增量缓存库；打开失败则退化为内存缓存（等同禁用缓存，不影响功能）。
    let cache_store = cache::CacheStore::open().unwrap_or_else(|e| {
        tracing::warn!("failed to open cache store, disabling incremental cache for this run: {e}");
        cache::CacheStore::in_memory().expect("failed to initialize in-memory cache")
    });
    // 已保存配置优先；否则用命令行/环境变量引导
    let cfg = config_store.lock().unwrap().load().or_else(bootstrap_config);

    // 插件宿主先于数据源初始化（"plugin" 数据源的 build_storage 需要它；
    // 默认构建为 None，零开销）
    let plugin_host = plugins::init(config_store.clone());

    let mut library = Library::default();
    let mut storage_opt: Option<Arc<dyn StorageBackend>> = None;

    if let Some(cfg) = &cfg {
        match config::build_storage(cfg, plugin_host.as_ref()) {
            Ok(storage) => match indexer::build_cached(
                storage.as_ref(),
                &cache_store,
                &config::source_key(cfg),
            ) {
                Ok((lib, stats)) => {
                    tracing::info!(
                        source = %cfg.source_type,
                        notes = stats.notes, folders = stats.folders, resources = stats.resources,
                        tags = stats.tags, errors = stats.errors, cached = stats.cached, fetched = stats.fetched,
                        "index built",
                    );
                    library = lib;
                    storage_opt = Some(storage);
                    // 引导成功则持久化，省得每次传参
                    let store = config_store.lock().unwrap();
                    if store.load().is_none() {
                        store.save(cfg).ok();
                    }
                }
                Err(e) => tracing::warn!("indexing failed (entering unconfigured state, reconfigure in the browser): {e}"),
            },
            Err(e) => tracing::warn!("invalid data source (entering unconfigured state): {e}"),
        }
    }

    if storage_opt.is_none() {
        tracing::info!("no data source configured yet — complete first-time setup in the browser.");
    }

    let read_only = cfg.as_ref().map(|c| c.read_only).unwrap_or(false);
    if read_only {
        tracing::info!("read-only mode enabled: all write operations will be rejected.");
    }

    // 访问鉴权（access control）：从配置库载入访问密码哈希/无密码阅读/黑白名单。
    let auth_state = {
        let auth_cfg = config_store.lock().unwrap().auth_config();
        if !auth_cfg.password_hash.is_empty() {
            tracing::info!(
                passwordless_read = auth_cfg.passwordless_read,
                list_mode = %auth_cfg.list_mode,
                "access password set: writes require login; anonymous read gated by passwordless_read + notebook list",
            );
        }
        auth::AuthState::from_config(&auth_cfg)
    };

    let state = Arc::new(AppState {
        library: Arc::new(RwLock::new(library)),
        storage: RwLock::new(storage_opt),
        config: config_store,
        cache: cache_store,
        read_only: AtomicBool::new(read_only),
        auth: auth_state,
        plugins: plugin_host,
        events: events::EventBus::new(),
    });

    // 托管前端：SPA 回退到 index.html。开发期前端跑 Vite 即可。
    let app = attach_web(api::router(state));

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("\n  ➜  open in browser: http://{}/", display_host(&host, &port));
    println!("  ➜  API root:        http://{addr}/api/folders\n");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

// 给路由挂上前端静态服务（fallback）：
//   1. 设了 JASPER_WEB_DIR → 从该磁盘目录托管（两种构建都支持，便于热替换前端）；
//   2. 否则 embed 构建 → 内嵌资源（单文件，运行时不依赖磁盘）；
//   3. 否则 → 源码旁 ../web/dist（开发/源码构建默认，Docker 等编译期路径不存在时用 1 覆盖）。
fn attach_web(router: axum::Router) -> axum::Router {
    if let Some(dir) = std::env::var("JASPER_WEB_DIR").ok().map(PathBuf::from) {
        if !dir.exists() {
            tracing::warn!("JASPER_WEB_DIR={} does not exist", dir.display());
        } else {
            tracing::debug!("serving frontend from JASPER_WEB_DIR={}", dir.display());
        }
        return serve_disk(router, dir);
    }
    #[cfg(feature = "embed")]
    {
        tracing::debug!("serving frontend from embedded assets");
        router.fallback(web_assets::handler)
    }
    #[cfg(not(feature = "embed"))]
    {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../web/dist");
        if !dir.exists() {
            tracing::warn!("frontend not built yet, web/dist does not exist; you can still hit /api/* to verify the backend");
        } else {
            tracing::debug!("serving frontend from {}", dir.display());
        }
        serve_disk(router, dir)
    }
}

fn serve_disk(router: axum::Router, dir: PathBuf) -> axum::Router {
    let index = dir.join("index.html");
    let serve = ServeDir::new(&dir).not_found_service(ServeFile::new(&index));
    router.fallback_service(serve)
}

fn display_host(host: &str, port: &str) -> String {
    let h = if host == "0.0.0.0" { "localhost" } else { host };
    format!("{h}:{port}")
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal received, stopping the server.");
}
