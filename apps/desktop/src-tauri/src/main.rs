#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod commands;
mod local_access;
use nexus_core::Application;
use tauri::Manager;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let application = tauri::Builder::default()
        .setup(|app| {
            let directory = match std::env::var_os("NEXUSOPS_TEST_APP_DATA_DIR") {
                Some(value) => {
                    let candidate = std::path::PathBuf::from(value);
                    let marker = candidate.join(".nexusops-isolated-test-profile");
                    if !candidate.is_absolute()
                        || !matches!(
                            std::fs::read_to_string(marker).as_deref(),
                            Ok("nexusops-goal-01a-isolated\n" | "nexusops-goal-02a-isolated\n")
                        )
                    {
                        return Err("Invalid isolated test profile directory".into());
                    }
                    candidate
                }
                None => app.path().app_data_dir()?,
            };
            std::fs::create_dir_all(&directory)?;
            let log=tracing_appender::rolling::Builder::new()
                .rotation(tracing_appender::rolling::Rotation::DAILY)
                .filename_prefix("nexusops").filename_suffix("jsonl").max_log_files(7)
                .build(directory.join("logs"))?;
            let (writer,guard)=tracing_appender::non_blocking(log);
            // Third-party debug logging is deliberately disabled, regardless of RUST_LOG.
            tracing_subscriber::fmt().json().with_ansi(false)
                .with_env_filter("off,nexus_core=info,nexus_ssh=info,nexus_discovery=info,nexus_operations=info,nexus_desktop=info")
                .with_writer(writer).try_init().map_err(|_|"Cannot initialize local logging")?;
            app.manage(guard);
            app.manage(Application::open(&directory)?);
            app.manage(local_access::LocalAccessService::default());
            tracing::info!(version=env!("CARGO_PKG_VERSION"),"NexusOps application ready");
            Ok(())
        })
        .plugin(tauri_plugin_clipboard_manager::init())
        .invoke_handler(tauri::generate_handler![commands::list_hosts,commands::save_host,commands::delete_host,commands::connect_host,commands::disconnect_host,commands::reconnect_host,commands::get_session,commands::trust_host_key,commands::refresh_host,commands::sample_host_monitor,commands::list_host_services,commands::list_terminals,commands::open_terminal,commands::poll_terminal,commands::write_terminal,commands::resize_terminal,commands::rename_terminal,commands::close_terminal,commands::open_sftp,commands::list_remote_directory,commands::remote_properties,commands::open_remote_text_file,commands::plan_remote_text_save,commands::discard_remote_text_document,commands::choose_upload_files,commands::choose_download_directory,commands::plan_upload,commands::plan_download,commands::plan_create_directory,commands::plan_rename,commands::plan_delete,commands::execute_file_plan,commands::discard_file_plan,commands::discard_local_grant,commands::list_transfers,commands::cancel_transfer,commands::plan_retry_transfer])
        .build(tauri::generate_context!())?;
    application.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            if let Err(error) =
                tauri::async_runtime::block_on(app.state::<Application>().shutdown())
            {
                tracing::error!(code=?error.code, "Shutdown transfer cleanup did not quiesce");
                api.prevent_exit();
            } else {
                app.state::<local_access::LocalAccessService>().revoke_all();
            }
        }
    });
    Ok(())
}
