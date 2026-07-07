mod daemon;
mod diagnostics;

use daemon::DaemonSupervisor;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if let Err(error) = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(
            tauri_plugin_log::Builder::new()
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::LogDir {
                        file_name: Some("susun-studio".into()),
                    },
                ))
                .build(),
        )
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(DaemonSupervisor::default())
        .invoke_handler(tauri::generate_handler![
            resolve_daemon_connection,
            export_diagnostics_bundle
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                window.app_handle().state::<DaemonSupervisor>().shutdown();
            }
        })
        .run(tauri::generate_context!())
    {
        eprintln!("failed to run Susun Studio: {error}");
    }
}

#[tauri::command]
async fn resolve_daemon_connection(
    app: tauri::AppHandle,
) -> Result<daemon::DaemonConnection, String> {
    daemon::resolve_connection(&app)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn export_diagnostics_bundle(app: tauri::AppHandle) -> Result<(), String> {
    diagnostics::export_bundle(&app)
        .await
        .map_err(|error| error.to_string())
}
