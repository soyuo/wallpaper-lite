use super::{APP_LOG_DIRECTORY, LEGACY_MUTEX_NAME, MUTEX_NAME};

fn identifier() -> String {
    let config: serde_json::Value =
        serde_json::from_str(include_str!("../../apps/ui/src-tauri/tauri.conf.json"))
            .expect("tauri.conf.json is valid JSON");
    config["identifier"]
        .as_str()
        .expect("identifier is a string")
        .to_owned()
}

#[test]
fn log_folder_and_mutex_follow_the_app_identifier() {
    let identifier = identifier();
    assert_eq!(APP_LOG_DIRECTORY, identifier);
    assert_eq!(MUTEX_NAME, format!("Local\\{identifier}.host"));
}

#[test]
fn the_legacy_mutex_name_is_different_so_an_old_host_is_still_noticed() {
    assert_ne!(LEGACY_MUTEX_NAME, MUTEX_NAME);
    assert!(LEGACY_MUTEX_NAME.starts_with("Local\\"));
}
