use tauri::Manager;

/// Returns the URL of the running gateway backend.
/// In dev this defaults to localhost:3210; in production the gateway
/// is expected to be running as a sidecar or on a known port.
#[tauri::command]
fn get_backend_url() -> String {
    std::env::var("SA_BACKEND_URL").unwrap_or_else(|_| "http://localhost:3210".to_string())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![get_backend_url])
        .setup(|app| {
            // In debug mode, open devtools automatically
            #[cfg(debug_assertions)]
            if let Some(window) = app.get_webview_window("main") {
                window.open_devtools();
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running SerialAssistant");
}
