#[path = "src/update_config.rs"]
mod update_config;

fn main() {
    println!("cargo:rerun-if-env-changed=TAURI_SIGNING_PRIVATE_KEY");
    println!("cargo:rerun-if-env-changed=TAURI_CONFIG");
    println!("cargo:rerun-if-env-changed=XTRACE_UPDATER_RELEASE");
    // Tauri CLI supplies the merged --config overlay through TAURI_CONFIG.
    // No updater settings live in the base config.
    let require_updates = std::env::var_os("XTRACE_UPDATER_RELEASE").is_some();
    if require_updates {
        assert!(
            std::env::var_os("TAURI_CONFIG").is_some(),
            "updater releases require a release config overlay"
        );
    }
    if let Ok(overlay) = std::env::var("TAURI_CONFIG") {
        let config: serde_json::Value =
            serde_json::from_str(&overlay).expect("invalid TAURI_CONFIG");
        let enabled = update_config::validate(config.pointer("/plugins/updater"))
            .expect("invalid updater release configuration");
        let artifacts = config.pointer("/bundle/createUpdaterArtifacts");
        if require_updates {
            assert!(
                enabled && artifacts == Some(&serde_json::Value::Bool(true)),
                "updater releases require configured updater and createUpdaterArtifacts: true"
            );
        }
        if enabled || artifacts.is_some_and(|value| value != &serde_json::Value::Bool(false)) {
            assert!(
                enabled,
                "updater artifacts require updater release configuration"
            );
            assert!(
                std::env::var("TAURI_SIGNING_PRIVATE_KEY").is_ok_and(|key| !key.trim().is_empty()),
                "updater-enabled builds require TAURI_SIGNING_PRIVATE_KEY"
            );
        }
    }
    tauri_build::build();
}
