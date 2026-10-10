#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod access;
mod http;

use std::{net::SocketAddr, path::PathBuf};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
            listener.set_nonblocking(true)?;
            let address: SocketAddr = listener.local_addr()?;
            let port = address.port();
            let token = uuid::Uuid::new_v4().to_string();
            let resource_dir = app.path().resource_dir().unwrap_or_else(|_| PathBuf::new());
            let helper_dir = if resource_dir.join("scripts/export-access.ps1").is_file() {
                resource_dir.join("scripts")
            } else {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scripts")
            };
            let state = http::AppState::new(port, token, helper_dir);
            let server_state = state.clone();
            tauri::async_runtime::spawn(async move {
                let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
                    return;
                };
                let _ = axum::serve(listener, http::router(server_state)).await;
            });

            WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::External(format!("http://127.0.0.1:{port}/").parse()?),
            )
            .on_navigation(move |url| {
                url.scheme() == "http"
                    && url.host_str() == Some("127.0.0.1")
                    && url.port() == Some(port)
            })
            .title("Access2Future")
            .inner_size(1240.0, 840.0)
            .min_inner_size(760.0, 640.0)
            .center()
            .build()?;
            app.manage(state);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let close_state = window.app_handle().state::<http::AppState>();
                if close_state.import_is_active() {
                    api.prevent_close();
                    let close_state = close_state.inner().clone();
                    let window = window.clone();
                    tauri::async_runtime::spawn(async move {
                        close_state.cancel_active_import().await;
                        let _ = window.close();
                    });
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("Access2Future の起動に失敗しました");
}
