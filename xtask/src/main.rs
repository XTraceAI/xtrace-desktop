use std::{env, error::Error, fs::OpenOptions, io::Write, path::PathBuf, process::ExitCode};
use xt_fixtures::{Fixture, FixtureId, FixtureStatus};

const HELP: &str = "XTrace development tasks:
  cargo xtask fixture-validate [--catalog PATH]
  cargo xtask fixture-db F1 --out PATH [--catalog PATH]
  cargo xtask fixture-export ID [--out PATH] [--catalog PATH]

Validation reports skeletons as unimplemented, not accepted rules.
Database output requires a populated fixture and never overwrites files.
Exports use canonical fixture records; IPC DTO generation is a later boundary.";

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let Some(task) = args.first().map(String::as_str) else {
        println!("{HELP}");
        return Ok(());
    };
    if matches!(task, "help" | "--help" | "-h") && args.len() == 1 {
        println!("{HELP}");
        return Ok(());
    }
    if !matches!(task, "fixture-validate" | "fixture-db" | "fixture-export") {
        return Err(format!("unknown task {task}; run cargo xtask help").into());
    }
    let mut rest = args[1..].iter();
    let id = if task != "fixture-validate" {
        Some(FixtureId::parse(
            rest.next().ok_or("fixture ID is required")?,
        )?)
    } else {
        None
    };
    let mut catalog = None;
    let mut output = None;
    while let Some(option) = rest.next() {
        let target = match option.as_str() {
            "--catalog" => &mut catalog,
            "--out" if task != "fixture-validate" => &mut output,
            _ => return Err(format!("unexpected option {option}").into()),
        };
        if target.is_some() {
            return Err(format!("duplicate option {option}").into());
        }
        let value = rest
            .next()
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| format!("missing value for {option}"))?;
        *target = Some(PathBuf::from(value));
    }
    let catalog =
        catalog.unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures"));
    if task == "fixture-validate" {
        let fixtures = Fixture::all(catalog)?;
        let mut asserted = 0;
        for fixture in &fixtures {
            let id = fixture.manifest().id;
            match fixture.manifest().status {
                FixtureStatus::Skeleton => {
                    println!("{id}: SKELETON — structure valid; acceptance UNIMPLEMENTED")
                }
                FixtureStatus::Populated => {
                    fixture.assert_reference()?;
                    asserted += 1;
                    println!(
                        "{id}: POPULATED — reference assertions passed ({} golden keys)",
                        fixture.expected().len()
                    );
                }
            }
        }
        println!(
            "{} entries structurally valid; {asserted} populated/asserted; {} skeleton/unimplemented. Product metric acceptance is not implied.",
            fixtures.len(),
            fixtures.len() - asserted
        );
        return Ok(());
    }
    let id = id.unwrap();
    let fixture = Fixture::load(catalog.join(id.to_string()))?;
    if fixture.manifest().id != id {
        return Err("manifest ID does not match requested fixture".into());
    }
    if task == "fixture-db" {
        let path = output.ok_or("fixture-db requires --out PATH")?;
        fixture.write_db(&path, true)?;
        println!(
            "{id}: wrote file-backed fixture database to {}",
            path.display()
        );
    } else {
        let json = serde_json::to_string_pretty(&fixture.export())? + "\n";
        if let Some(path) = output {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(json.as_bytes())?;
            println!("{id}: wrote canonical fixture export to {}", path.display());
        } else {
            print!("{json}");
        }
    }
    Ok(())
}
