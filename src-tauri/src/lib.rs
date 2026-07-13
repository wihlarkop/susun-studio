mod backup;
mod daemon;
mod diagnostics;
mod restore;

use daemon::DaemonSupervisor;
use log::{error, info};
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    info!("event=studio_starting");
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
            export_diagnostics_bundle,
            backup_studio_data,
            preview_restore_studio_data,
            apply_restore_studio_data
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                window.app_handle().state::<DaemonSupervisor>().shutdown();
            }
        })
        .run(tauri::generate_context!())
    {
        error!("event=studio_run_failed error={error}");
        eprintln!("failed to run Susun Studio: {error}");
    }
}

#[tauri::command]
async fn resolve_daemon_connection(
    app: tauri::AppHandle,
) -> Result<daemon::DaemonConnection, String> {
    info!("event=resolve_daemon_connection_command_started");
    daemon::resolve_connection(&app).await.map_err(|error| {
        error!("event=resolve_daemon_connection_command_failed error={error}");
        error.to_string()
    })
}

#[tauri::command]
async fn export_diagnostics_bundle(
    app: tauri::AppHandle,
) -> Result<diagnostics::DiagnosticsExportOutcome, String> {
    diagnostics::export_bundle(&app).await.map_err(|error| {
        error!("event=export_diagnostics_bundle_command_failed error={error}");
        error.to_string()
    })
}

#[tauri::command]
async fn backup_studio_data(app: tauri::AppHandle) -> Result<backup::BackupOutcome, String> {
    backup::backup_studio_data(&app).await.map_err(|error| {
        error!("event=backup_studio_data_command_failed error={error}");
        error.to_string()
    })
}

#[tauri::command]
async fn preview_restore_studio_data(
    app: tauri::AppHandle,
) -> Result<backup::RestorePreviewOutcome, String> {
    backup::preview_restore(&app).await.map_err(|error| {
        error!("event=preview_restore_studio_data_command_failed error={error}");
        error.to_string()
    })
}

#[tauri::command]
async fn apply_restore_studio_data(
    app: tauri::AppHandle,
    archive_path: String,
    plan_id: String,
) -> Result<restore::RestoreOutcome, String> {
    restore::apply_restore(&app, &archive_path, &plan_id)
        .await
        .map_err(|error| {
            error!("event=apply_restore_studio_data_command_failed error={error}");
            error.to_string()
        })
}
