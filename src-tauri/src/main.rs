#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod server;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

struct AppServerState {
    handle: server::ServerHandle,
}

fn main() {
    let mut builder = tauri::Builder::default();

    // 单实例运行保护
    builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    }));

    builder
        .setup(|app| {
            // 在后台 Tokio 运行时启动 Axum 服务器
            let server_handle = tauri::async_runtime::block_on(async {
                server::start_server().await
            })?;

            let port = server_handle.port;
            app.manage(AppServerState {
                handle: server_handle,
            });

            // 动态创建主窗口直连本地 Axum 服务
            let server_url: url::Url = format!("http://127.0.0.1:{}", port).parse()?;
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(server_url))
                .title("BaiduPCS-Rust 百度网盘")
                .inner_size(1200.0, 800.0)
                .min_inner_size(900.0, 600.0)
                .center()
                .build()?;

            // 监听窗口关闭按钮：点击 X 时隐藏到托盘而非退出
            let window_clone = window.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window_clone.hide();
                }
            });

            // 构建托盘菜单
            let show_item = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
            let hide_item = MenuItem::with_id(app, "hide", "最小化到托盘", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "退出程序", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&show_item, &hide_item, &quit_item])?;

            // 挂载系统托盘
            let icon = app.default_window_icon().cloned().expect("缺少应用窗口图标");
            let _tray = TrayIconBuilder::new()
                .icon(icon)
                .tooltip("BaiduPCS-Rust 百度网盘")
                .menu(&tray_menu)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.unminimize();
                            let _ = w.set_focus();
                        }
                    }
                    "hide" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.hide();
                        }
                    }
                    "quit" => {
                        let app_handle = app.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Some(state) = app_handle.try_state::<AppServerState>() {
                                state.handle.web_auth_state.stop_cleanup_tasks().await;
                                state.handle.app_state.shutdown().await;
                            }
                            app_handle.exit(0);
                        });
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(w) = app.get_webview_window("main") {
                            if w.is_visible().unwrap_or(false) {
                                let _ = w.hide();
                            } else {
                                let _ = w.show();
                                let _ = w.unminimize();
                                let _ = w.set_focus();
                            }
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("运行 Tauri 桌面客户端失败");
}
