#[tauri::command]
async fn host_ping() -> Result<(), String> {
    wallpaper_ipc::client::ping()
        .await
        .map_err(|error| error.to_string())
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![host_ping])
        .run(tauri::generate_context!())
        .expect("failed to run wallpaper-lite UI");
}
