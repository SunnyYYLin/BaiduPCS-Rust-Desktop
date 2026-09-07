use axum::{
    body::Body,
    http::{header, HeaderValue, StatusCode, Uri},
    middleware,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use baidu_netdisk_rust::{
    common::proxy_fallback::ProxyHotUpdater,
    config::{LogConfig, WebAuthConfig},
    logging,
    server::{self, handlers, websocket},
    web_auth::{self, create_auth_store, WebAuthState},
    AppState,
};
use rust_embed::RustEmbed;
use serde::Serialize;
use std::sync::Arc;
use tower::ServiceBuilder;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing::info;

#[derive(RustEmbed)]
#[folder = "../upstream/BaiduPCS-Rust/frontend/dist"]
struct FrontendAssets;

async fn static_handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    if let Some(content) = FrontendAssets::get(path) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, HeaderValue::from_str(mime.as_ref()).unwrap())
            .body(Body::from(content.data))
            .unwrap();
    }

    // SPA 回退
    if let Some(index) = FrontendAssets::get("index.html") {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"))
            .body(Body::from(index.data))
            .unwrap()
    } else {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("404 Not Found"))
            .unwrap()
    }
}

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    service: String,
}

async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        service: "baidu-netdisk-rust".to_string(),
    })
}

async fn init_web_auth_state(config: &WebAuthConfig) -> Arc<WebAuthState> {
    let auth_store = Arc::new(create_auth_store());
    if let Err(e) = auth_store.load().await {
        tracing::warn!("加载认证凭证失败，使用空凭证: {}", e);
    }
    let credentials = auth_store.get_credentials().await;
    let state = Arc::new(WebAuthState::new(
        config.clone(),
        credentials,
        None,
        auth_store,
    ));
    state.start_cleanup_tasks().await;
    state
}

pub struct ServerHandle {
    pub port: u16,
    pub app_state: AppState,
    pub web_auth_state: Arc<WebAuthState>,
}

pub async fn start_server() -> anyhow::Result<ServerHandle> {
    let log_config = LogConfig::default();
    let _log_guard = logging::init_logging(&log_config);

    info!("Baidu Netdisk Rust Desktop v{} 后端启动中...", env!("CARGO_PKG_VERSION"));

    let app_state = AppState::new().await?;

    {
        let config = app_state.config.read().await;
        let download_dir = config.download.download_dir.clone();
        drop(config);
        if !download_dir.exists() {
            let _ = std::fs::create_dir_all(&download_dir);
        }
    }

    app_state.fallback_mgr.set_updater(
        Arc::new(app_state.clone()) as Arc<dyn ProxyHotUpdater>
    ).await;

    {
        let cfg = app_state.config.read().await;
        let proxy = &cfg.network.proxy;
        if proxy.proxy_type != baidu_netdisk_rust::common::ProxyType::None {
            let proxy_clone = proxy.clone();
            let allow_fallback = proxy.allow_fallback;
            drop(cfg);

            app_state.fallback_mgr
                .set_user_proxy_config(Some(proxy_clone.clone()))
                .await;

            match baidu_netdisk_rust::common::probe_proxy(&proxy_clone).await {
                Ok(()) => {
                    info!("✓ 启动代理探测成功");
                    app_state.fallback_mgr
                        .set_runtime_status(baidu_netdisk_rust::common::ProxyRuntimeStatus::Normal)
                        .await;
                }
                Err(e) => {
                    tracing::warn!("✗ 启动代理探测失败: {}，触发回退", e);
                    if allow_fallback {
                        app_state.fallback_mgr.execute_fallback().await;
                    } else {
                        app_state.fallback_mgr
                            .set_runtime_status(baidu_netdisk_rust::common::ProxyRuntimeStatus::FallenBackToDirect)
                            .await;
                    }
                }
            }
        }
    }

    let config = app_state.config.read().await.clone();
    let web_auth_state = init_web_auth_state(&config.web_auth).await;

    let middleware = ServiceBuilder::new()
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        );

    let api_routes = Router::new()
        .route("/auth/qrcode/generate", post(handlers::generate_qrcode))
        .route("/auth/qrcode/status", get(handlers::qrcode_status))
        .route("/auth/cookie/login", post(handlers::cookie_login))
        .route("/auth/user", get(handlers::get_current_user))
        .route("/auth/logout", post(handlers::logout))
        .route("/accounts/list", get(handlers::accounts::list_accounts))
        .route("/accounts/switch", post(handlers::accounts::switch_account))
        .route("/accounts/:uid", delete(handlers::accounts::delete_account))
        .route("/files", get(handlers::get_file_list))
        .route("/files/search", get(handlers::search_files))
        .route("/files/download", get(handlers::get_download_url))
        .route("/files/folder", post(handlers::create_folder))
        .route("/files/delete", post(handlers::delete_files))
        .route("/files/copy", post(handlers::copy_files))
        .route("/files/move", post(handlers::move_files))
        .route("/files/rename", post(handlers::rename_file))
        .route("/downloads", post(handlers::create_download))
        .route("/downloads", get(handlers::get_all_downloads))
        .route("/downloads/all", get(handlers::get_all_downloads_mixed))
        .route("/downloads/active", get(handlers::get_active_downloads))
        .route("/downloads/batch", post(handlers::create_batch_download))
        .route("/downloads/:id", get(handlers::get_download))
        .route("/downloads/:id/pause", post(handlers::pause_download))
        .route("/downloads/:id/resume", post(handlers::resume_download))
        .route("/downloads/:id", delete(handlers::delete_download))
        .route("/downloads/clear/completed", delete(handlers::clear_completed))
        .route("/downloads/clear/failed", delete(handlers::clear_failed))
        .route("/downloads/batch/pause", post(handlers::batch_pause_downloads))
        .route("/downloads/batch/resume", post(handlers::batch_resume_downloads))
        .route("/downloads/batch/delete", post(handlers::batch_delete_downloads))
        .route("/downloads/folder", post(handlers::create_folder_download))
        .route("/downloads/folders", get(handlers::get_all_folder_downloads))
        .route("/downloads/folder/:id", get(handlers::get_folder_download))
        .route("/downloads/folder/:id/skipped", get(handlers::get_folder_skipped_files))
        .route("/downloads/folder/:id/pause", post(handlers::pause_folder_download))
        .route("/downloads/folder/:id/resume", post(handlers::resume_folder_download))
        .route("/downloads/folder/:id", delete(handlers::cancel_folder_download))
        .route("/uploads", post(handlers::create_upload))
        .route("/uploads", get(handlers::get_all_uploads))
        .route("/uploads/:id", get(handlers::get_upload))
        .route("/uploads/:id/pause", post(handlers::pause_upload))
        .route("/uploads/:id/resume", post(handlers::resume_upload))
        .route("/uploads/:id", delete(handlers::delete_upload))
        .route("/uploads/folder", post(handlers::create_folder_upload))
        .route("/uploads/batch", post(handlers::create_batch_upload))
        .route("/uploads/scan/:id", get(handlers::get_scan_status))
        .route("/uploads/scan/:id/cancel", post(handlers::cancel_scan))
        .route("/uploads/clear/completed", post(handlers::clear_completed_uploads))
        .route("/uploads/clear/failed", post(handlers::clear_failed_uploads))
        .route("/uploads/batch/pause", post(handlers::batch_pause_uploads))
        .route("/uploads/batch/resume", post(handlers::batch_resume_uploads))
        .route("/uploads/batch/delete", post(handlers::batch_delete_uploads))
        .route("/transfers", post(handlers::create_transfer))
        .route("/transfers", get(handlers::get_all_transfers))
        .route("/transfers/preview", post(handlers::preview_share_files))
        .route("/transfers/preview/dir", post(handlers::preview_share_dir))
        .route("/transfers/cleanup", post(handlers::cleanup_orphaned_temp_dirs))
        .route("/transfers/:id", get(handlers::get_transfer))
        .route("/transfers/:id", delete(handlers::delete_transfer))
        .route("/transfers/:id/cancel", post(handlers::cancel_transfer))
        .route("/fs/list", get(handlers::list_directory))
        .route("/fs/goto", get(handlers::goto_path))
        .route("/fs/validate", get(handlers::validate_path))
        .route("/fs/roots", get(handlers::get_roots))
        .route("/local-files", get(handlers::local_files::list_local_files))
        .route("/local-files/delete", post(handlers::local_files::delete_local_files))
        .route("/config", get(handlers::get_config))
        .route("/config", put(handlers::update_config))
        .route("/config/recommended", get(handlers::get_recommended_config))
        .route("/config/reset", post(handlers::reset_to_recommended))
        .route("/config/recent-dir", post(handlers::update_recent_dir))
        .route("/config/default-download-dir", post(handlers::set_default_download_dir))
        .route("/config/transfer", get(handlers::get_transfer_config))
        .route("/config/transfer", put(handlers::update_transfer_config))
        .route("/budget", get(handlers::budget::get_budget))
        .route("/config/multi_account_budget", put(handlers::budget::update_multi_account_budget))
        .route("/config/vip_recommended", put(handlers::budget::update_vip_recommended))
        .route("/accounts/:uid/custom_config", put(handlers::budget::update_account_custom_config))
        .route("/proxy/status", get(handlers::get_proxy_status))
        .route("/proxy/test", post(handlers::test_proxy_connection))
        .route("/autobackup/configs", get(handlers::autobackup::list_backup_configs))
        .route("/autobackup/configs", post(handlers::autobackup::create_backup_config))
        .route("/autobackup/configs/:id", get(handlers::autobackup::get_backup_config))
        .route("/autobackup/configs/:id", put(handlers::autobackup::update_backup_config))
        .route("/autobackup/configs/:id", delete(handlers::autobackup::delete_backup_config))
        .route("/autobackup/configs/:id/enable", post(handlers::autobackup::enable_backup_config))
        .route("/autobackup/configs/:id/disable", post(handlers::autobackup::disable_backup_config))
        .route("/autobackup/configs/:id/trigger", post(handlers::autobackup::trigger_backup))
        .route("/autobackup/configs/:id/tasks", get(handlers::autobackup::list_backup_tasks))
        .route("/autobackup/tasks/:id", get(handlers::autobackup::get_backup_task))
        .route("/autobackup/tasks/:id/cancel", post(handlers::autobackup::cancel_backup_task))
        .route("/autobackup/tasks/:id/pause", post(handlers::autobackup::pause_backup_task))
        .route("/autobackup/tasks/:id/resume", post(handlers::autobackup::resume_backup_task))
        .route("/autobackup/tasks/:id", delete(handlers::autobackup::delete_backup_task))
        .route("/autobackup/tasks/:id/files", get(handlers::autobackup::list_file_tasks))
        .route("/autobackup/tasks/:task_id/files/:file_task_id/retry", post(handlers::autobackup::retry_file_task))
        .route("/autobackup/configs/:id/sync-state/reset", post(handlers::autobackup::reset_sync_state))
        .route("/autobackup/configs/:id/sync-state/tombstones", get(handlers::autobackup::list_tombstones))
        .route("/autobackup/status", get(handlers::autobackup::get_manager_status))
        .route("/autobackup/stats", get(handlers::autobackup::get_record_stats))
        .route("/autobackup/cleanup", post(handlers::autobackup::cleanup_records))
        .route("/encryption/status", get(handlers::autobackup::get_encryption_status))
        .route("/encryption/key/generate", post(handlers::autobackup::generate_encryption_key))
        .route("/encryption/key/import", post(handlers::autobackup::import_encryption_key))
        .route("/encryption/key/export", get(handlers::autobackup::export_encryption_key))
        .route("/encryption/key", delete(handlers::autobackup::delete_encryption_key))
        .route("/encryption/key/force", delete(handlers::autobackup::force_delete_encryption_key))
        .route("/share-sync/subscriptions", get(handlers::list_subscriptions))
        .route("/share-sync/subscriptions", post(handlers::create_subscription))
        .route("/share-sync/subscriptions/:id", get(handlers::get_subscription))
        .route("/share-sync/subscriptions/:id", put(handlers::update_subscription))
        .route("/share-sync/subscriptions/:id", delete(handlers::delete_subscription))
        .route("/share-sync/subscriptions/:id/enable", post(handlers::enable_subscription))
        .route("/share-sync/subscriptions/:id/disable", post(handlers::disable_subscription))
        .route("/share-sync/subscriptions/:id/trigger", post(handlers::trigger_subscription))
        .route("/share-sync/subscriptions/:id/resume", post(handlers::resume_subscription))
        .route("/share-sync/subscriptions/:id/runs", get(handlers::list_runs))
        .route("/share-sync/subscriptions/:id/subtasks", get(handlers::list_subtasks))
        .route("/share-sync/runs/:id", get(handlers::get_run))
        .route("/share-sync/runs/:id/items", get(handlers::list_run_items))
        .route("/share-sync/subscriptions/:id/snapshots/latest", get(handlers::latest_snapshot))
        .route("/share-sync/preview-tree", post(handlers::preview_tree))
        .route("/encryption/export-bundle", post(handlers::export_bundle))
        .route("/encryption/export-mapping", get(handlers::export_mapping))
        .route("/encryption/export-keys", get(handlers::export_keys))
        .route("/cloud-dl/tasks", post(handlers::cloud_dl::add_task))
        .route("/cloud-dl/tasks", get(handlers::cloud_dl::list_tasks))
        .route("/cloud-dl/tasks/clear", delete(handlers::cloud_dl::clear_tasks))
        .route("/cloud-dl/tasks/refresh", post(handlers::cloud_dl::refresh_tasks))
        .route("/cloud-dl/tasks/:task_id", get(handlers::cloud_dl::query_task))
        .route("/cloud-dl/tasks/:task_id", delete(handlers::cloud_dl::delete_task))
        .route("/cloud-dl/tasks/:task_id/cancel", post(handlers::cloud_dl::cancel_task))
        .route("/shares", post(handlers::create_share))
        .route("/shares", get(handlers::get_share_list))
        .route("/shares/cancel", post(handlers::cancel_share))
        .route("/shares/:id", get(handlers::get_share_detail))
        .route("/system/watch-capability", get(handlers::autobackup::get_watch_capability))
        .route("/config/autobackup/trigger", get(handlers::autobackup::get_trigger_config))
        .route("/config/autobackup/trigger", put(handlers::autobackup::update_trigger_config))
        .route("/ws", get(websocket::handle_websocket))
        .layer(middleware::from_fn_with_state(
            app_state.clone(),
            server::middleware::readonly_middleware,
        ))
        .with_state(app_state.clone())
        .layer(middleware::from_fn_with_state(
            web_auth_state.clone(),
            web_auth::web_auth_middleware,
        ));

    let web_auth_routes = Router::new()
        .route("/login", post(web_auth::login))
        .route("/refresh", post(web_auth::refresh))
        .route("/logout", post(web_auth::logout))
        .route("/status", get(web_auth::status))
        .route("/config", get(web_auth::get_config))
        .route("/config", put(web_auth::update_config))
        .route("/password/set", post(web_auth::set_password))
        .route("/totp/setup", post(web_auth::totp_setup))
        .route("/totp/verify", post(web_auth::totp_verify))
        .route("/totp/disable", post(web_auth::totp_disable))
        .route("/recovery-codes/regenerate", post(web_auth::regenerate_recovery_codes))
        .with_state(web_auth_state.clone());

    app_state.load_initial_session().await?;
    info!("应用状态初始化完成");

    if let Err(e) = app_state.preheat_inactive_clients().await {
        tracing::warn!("ClientPool 预热出现非致命错误: {}", e);
    }

    let app = Router::new()
        .nest("/api/v1", api_routes)
        .nest("/api/v1/web-auth", web_auth_routes)
        .route("/health", get(health_check))
        .fallback(static_handler)
        .layer(middleware);

    let default_addr = "127.0.0.1:18888";
    let listener = match tokio::net::TcpListener::bind(default_addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("端口 18888 绑定失败 ({})，自动尝试动态分配端口...", e);
            tokio::net::TcpListener::bind("127.0.0.1:0").await?
        }
    };

    let bound_addr = listener.local_addr()?;
    let bound_port = bound_addr.port();

    info!("✓ 后端服务启动成功: http://{}", bound_addr);

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!("后端服务器退出错误: {}", e);
        }
    });

    Ok(ServerHandle {
        port: bound_port,
        app_state,
        web_auth_state,
    })
}
