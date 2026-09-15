fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "list_hosts",
            "save_host",
            "delete_host",
            "connect_host",
            "disconnect_host",
            "reconnect_host",
            "get_session",
            "trust_host_key",
            "refresh_host",
        ]),
    ))
    .expect("Tauri configuration and permission generation must succeed");
}
