use std::path::{Path, PathBuf};
use xt_ingest::native::{ProducerSource, readers_cli::read_pin};

pub fn bundle_mode() -> bool {
    match std::env::var("AGENT_PLUGINS_SOURCE").as_deref() {
        Err(std::env::VarError::NotPresent) | Ok("checkout") => false,
        Ok("bundle") => true,
        _ => panic!("AGENT_PLUGINS_SOURCE must be checkout or bundle"),
    }
}

pub fn source(repo: &Path, plugin_root: &Path) -> ProducerSource {
    if bundle_mode() {
        let pin = read_pin(&repo.join(".plugin-pin")).unwrap();
        let mut root = PathBuf::from(plugin_root);
        for part in Path::new(&pin.plugin_root).components().rev() {
            assert_eq!(root.file_name(), Some(part.as_os_str()));
            root.pop();
        }
        ProducerSource::Bundle { pin, root }
    } else {
        ProducerSource::Checkout {
            pin: repo.join(".plugin-pin"),
            plugin_root: Some(plugin_root.to_owned()),
        }
    }
}
