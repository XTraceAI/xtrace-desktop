//! Local metadata-only audit import. No native source content is read here.
use std::{env, fs};
use xt_store::{Store, human_input::OriginManifest};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().collect();
    if args.len() != 5 || args[1] != "--db" || args[3] != "--manifest" {
        return Err("usage: human_origins_import --db INDEX --manifest MANIFEST".into());
    }
    let metadata = fs::metadata(&args[4])?;
    if metadata.len() > 16 * 1024 * 1024 {
        return Err("manifest is too large".into());
    }
    let manifest: OriginManifest = serde_json::from_slice(&fs::read(&args[4])?)?;
    let report = Store::open(&args[2])?.import_human_session_origins(&manifest)?;
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}
