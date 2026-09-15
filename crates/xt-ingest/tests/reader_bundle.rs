//! The bundled reader sources: the copy of the pinned producer's scripts the
//! app ships is verified against `.plugin-pin` by Git object identity computed
//! without Git, so an edited, incomplete or foreign bundle is refused, and a
//! verified bundle runs the readers in place.
use std::{
    fs,
    path::{Path, PathBuf},
};
use xt_ingest::native::{
    HostStatus, ImportRequest, ProducerSource, SessionOutcome, import_native,
    readers_cli::{Pin, ReaderError, git_object_id, read_pin, verify_bundle},
};
use xt_store::{Host, Store};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn bundle() -> PathBuf {
    repo().join("vendor/agent-plugins")
}

fn pin() -> Pin {
    read_pin(&repo().join(".plugin-pin")).unwrap()
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

#[test]
fn object_ids_are_computed_as_git_computes_them() {
    // Reference values from `git hash-object` and `git write-tree` over the
    // same layout: a blob, an executable blob, a subtree and the root tree.
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path();
    fs::write(root.join("a.py"), "hello\n").unwrap();
    fs::create_dir(root.join("sub")).unwrap();
    fs::write(root.join("sub/b.py"), "x").unwrap();
    fs::write(root.join("run.sh"), "#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert_eq!(
        git_object_id(&root.join("a.py")).unwrap(),
        "ce013625030ba8dba906f756967f9e9ca394464a"
    );
    assert_eq!(
        git_object_id(&root.join("sub")).unwrap(),
        "237cd01efd8a70bb4af65f70449313e722930d9e"
    );
    #[cfg(unix)]
    {
        assert_eq!(
            git_object_id(&root.join("run.sh")).unwrap(),
            "1a2485251c33a70432394c93fb89330ef214bfc9"
        );
        assert_eq!(
            git_object_id(root).unwrap(),
            "68fae95406169ee7c2a60dd95cbe974f88d6a7e1"
        );
        // Bytecode caches are not part of the tree, as Git never holds them.
        fs::create_dir(root.join("__pycache__")).unwrap();
        fs::write(root.join("__pycache__/a.cpython-312.pyc"), "cache").unwrap();
        assert_eq!(
            git_object_id(root).unwrap(),
            "68fae95406169ee7c2a60dd95cbe974f88d6a7e1"
        );
        // A symlink is neither a file nor a directory: refused, not hashed.
        std::os::unix::fs::symlink("a.py", root.join("link.py")).unwrap();
        assert!(
            git_object_id(root)
                .unwrap_err()
                .contains("neither a file nor a directory")
        );
    }
}

#[test]
fn the_bundled_sources_are_the_pinned_objects_without_git() {
    let pin = pin();
    let producer = verify_bundle(&pin, &bundle()).unwrap();
    assert_eq!(producer.commit, pin.commit);
    assert_eq!(producer.plugin_version, pin.plugin_version);
    assert_eq!(
        producer.script,
        bundle()
            .join(&pin.plugin_root)
            .join("scripts/readers_cli.py")
    );
    // The same verification through the producer source the app configures.
    let source = ProducerSource::Bundle {
        pin: pin.clone(),
        root: bundle(),
    };
    assert_eq!(source.producer().unwrap().script, producer.script);
    // Every listed object is verified individually as well as the tree.
    for path in pin.reader_sources.keys() {
        assert_eq!(
            git_object_id(&bundle().join(path)).unwrap(),
            pin.reader_sources[path],
            "{path}"
        );
    }
}

#[test]
fn an_edited_incomplete_or_foreign_bundle_is_refused() {
    let pin = pin();
    let scripts = format!("{}/scripts", pin.plugin_root);
    let mismatch = |root: &Path| match verify_bundle(&pin, root) {
        Err(ReaderError::PinMismatch(reason)) => reason,
        other => panic!("expected a pin mismatch, got {other:?}"),
    };
    let temp = tempfile::TempDir::new().unwrap();
    let copies = temp.path();
    let fresh = |name: &str| {
        let root = copies.join(name);
        copy_dir(&bundle(), &root);
        root
    };
    // Edited module: its blob and every tree above it change.
    let edited = fresh("edited");
    let module = edited.join(&scripts).join("readers/codex.py");
    let mut text = fs::read_to_string(&module).unwrap();
    text.push_str("\n# edited\n");
    fs::write(&module, text).unwrap();
    assert!(
        mismatch(&edited).contains("differs"),
        "{}",
        mismatch(&edited)
    );
    // A module the readers import removed.
    let incomplete = fresh("incomplete");
    fs::remove_file(incomplete.join(&scripts).join("session_title.py")).unwrap();
    assert!(mismatch(&incomplete).contains("differs"));
    // An extra module alongside the pinned ones.
    let extra = fresh("extra");
    fs::write(extra.join(&scripts).join("session_title_shadow.py"), "").unwrap();
    assert!(mismatch(&extra).contains("differs"));
    // A bytecode cache left by an interpreter changes nothing.
    let cached = fresh("cached");
    fs::create_dir(cached.join(&scripts).join("__pycache__")).unwrap();
    fs::write(cached.join(&scripts).join("__pycache__/x.pyc"), "cache").unwrap();
    assert!(verify_bundle(&pin, &cached).is_ok());
    // No bundle at all, and a bundle whose script is missing.
    assert!(mismatch(&copies.join("absent")).contains("entity not found"));
    // A pin that does not name the scripts tree cannot vouch for the modules
    // the readers import from it.
    let mut partial = pin.clone();
    partial.reader_sources.remove(&scripts);
    assert!(matches!(
        verify_bundle(&partial, &bundle()),
        Err(ReaderError::PinMismatch(reason)) if reason.contains("scripts tree")
    ));
}

#[test]
fn the_bundle_reads_a_reader_host_in_place() {
    // A verified bundle runs the pinned readers without a checkout or Git:
    // an empty Codex history is read to completion.
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    fs::create_dir_all(home.join(".codex/sessions")).unwrap();
    let mut store = Store::open(temp.path().join("index.sqlite")).unwrap();
    let source = ProducerSource::Bundle {
        pin: pin(),
        root: bundle(),
    };
    let report = import_native(
        &mut store,
        &ImportRequest {
            home: &home,
            hosts: &[Host::Codex],
            producer: &source,
            python: None,
            observed_at: 1,
            cancel: None,
        },
    );
    let host = &report.hosts[0];
    assert_eq!(host.status, HostStatus::Complete, "{report:?}");
    assert!(host.detail.as_deref().unwrap().contains("pinned producer"));
    assert!(
        host.sessions
            .iter()
            .all(|s| matches!(s.outcome, SessionOutcome::Imported { .. }))
    );
}
