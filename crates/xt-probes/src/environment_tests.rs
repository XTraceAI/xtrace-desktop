//! Unit regressions for the probe's read primitives, where a file or a
//! directory can be changed between the walk and the read deterministically.
use super::*;
use std::io;

fn temp() -> (tempfile::TempDir, PathBuf) {
    let guard = tempfile::TempDir::new().unwrap();
    // Test setup only: the probe itself never resolves a path.
    let base = guard.path().canonicalize().unwrap();
    (guard, base)
}

/// A reader that never ends, standing for a file that keeps growing after
/// its size was checked: the read still stops one byte past the bound.
#[test]
fn bounded_read_stops_one_byte_past_the_bound_for_an_endless_source() {
    assert_eq!(
        read_bounded(io::repeat(b'x')),
        Err(SourceStatus::Unsupported {
            reason: UnsupportedReason::FileTooLarge
        })
    );
    let exact = usize::try_from(MAX_SOURCE_BYTES).unwrap();
    assert_eq!(
        read_bounded(io::repeat(b'x').take(MAX_SOURCE_BYTES))
            .unwrap()
            .len(),
        exact
    );
    assert!(read_bounded(io::repeat(b'x').take(MAX_SOURCE_BYTES + 1)).is_err());
}

/// A file that grows past the bound after its size was checked is refused,
/// though the stale size said it was small.
#[test]
fn bounded_read_refuses_a_file_that_grew_after_it_was_located() {
    let (_guard, base) = temp();
    let path = base.join("mcp.json");
    fs::write(&path, "{}").unwrap();
    let located = fs::symlink_metadata(&path).unwrap();
    assert!(located.len() < MAX_SOURCE_BYTES);
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    io::Write::write_all(
        &mut file,
        &vec![b' '; usize::try_from(MAX_SOURCE_BYTES).unwrap()],
    )
    .unwrap();
    assert_eq!(
        read_located(&path, &located),
        Err(SourceStatus::Unsupported {
            reason: UnsupportedReason::FileTooLarge
        })
    );
}

/// A located file replaced by a link, or by another file, before it is
/// opened is refused, and nothing the replacement holds is returned.
#[cfg(unix)]
#[test]
fn located_file_replaced_by_a_link_or_another_file_is_refused() {
    let (_guard, base) = temp();
    let outside = base.join("outside.json");
    fs::write(&outside, r#"{"mcpServers":{"escaped":{}}}"#).unwrap();
    let path = base.join("mcp.json");
    fs::write(&path, "{}").unwrap();
    let located = fs::symlink_metadata(&path).unwrap();
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&outside, &path).unwrap();
    assert_eq!(
        read_located(&path, &located),
        Err(SourceStatus::Unsupported {
            reason: UnsupportedReason::Symlink
        })
    );
    let replacement = base.join("replacement.json");
    fs::write(&replacement, r#"{"mcpServers":{"swapped":{}}}"#).unwrap();
    fs::remove_file(&path).unwrap();
    fs::rename(&replacement, &path).unwrap();
    assert_eq!(read_located(&path, &located), Err(SourceStatus::Unreadable));
}

/// A located directory replaced by a link to another directory before it is
/// enumerated is refused: the other directory's entries are discarded.
#[cfg(unix)]
#[test]
fn located_directory_replaced_by_a_link_is_refused() {
    let (_guard, base) = temp();
    let elsewhere = base.join("elsewhere");
    fs::create_dir_all(elsewhere.join("escaped")).unwrap();
    let path = base.join("skills");
    fs::create_dir_all(&path).unwrap();
    let located = fs::symlink_metadata(&path).unwrap();
    fs::remove_dir(&path).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &path).unwrap();
    assert!(matches!(
        list_located(path, &located),
        Err(SourceStatus::Unsupported {
            reason: UnsupportedReason::Symlink
        })
    ));
}

/// A located object that is replaced after the read fails the repeated walk.
#[test]
fn a_walk_that_no_longer_reaches_the_read_object_invalidates_the_read() {
    let (_guard, base) = temp();
    fs::create_dir_all(base.join(".cursor")).unwrap();
    fs::write(base.join(".cursor/mcp.json"), "{}").unwrap();
    let known = Known::new(&base, ".cursor/mcp.json");
    let (_, read) = known.locate().unwrap();
    fs::remove_file(base.join(".cursor/mcp.json")).unwrap();
    fs::write(base.join(".cursor/other.json"), "{}").unwrap();
    fs::write(base.join(".cursor/mcp.json"), "{}").unwrap();
    // A new file at the same path is another object.
    let (_, now) = known.locate().unwrap();
    if !same_object(&read, &now) {
        assert_eq!(known.confirm(&read), Err(SourceStatus::Unreadable));
    }
    fs::remove_file(base.join(".cursor/mcp.json")).unwrap();
    assert_eq!(known.confirm(&read), Err(SourceStatus::Missing));
}

#[test]
fn installation_timestamps_use_the_locked_rfc3339_parser() {
    for valid in [
        "2026-09-01T00:00:00Z",
        "2026-09-01T00:00:00.000Z",
        "2026-09-01T00:00:00+02:00",
        "2026-09-01t00:00:00.5-08:00",
        "2028-02-29T12:00:00Z",
    ] {
        assert!(rfc3339_timestamp(valid), "{valid}");
    }
    for invalid in [
        "",
        "yesterday",
        "2026-09-01",
        "2026-09-01T00:00:00",
        "2026-09-01T00:00:00.Z",
        "2026-09-01T00:00:00+0200",
        "2026-09-01T00:00:00Zjunk",
        // Shape-valid but impossible dates and times.
        "2026-02-30T00:00:00Z",
        "2026-02-29T00:00:00Z",
        "2026-13-01T00:00:00Z",
        "2026-00-10T00:00:00Z",
        "2026-04-31T00:00:00Z",
        "2026-09-01T25:00:00Z",
        "2026-09-01T23:60:00Z",
        "2026-09-01T23:59:61Z",
        "2026-09-01T00:00:00+24:00",
    ] {
        assert!(!rfc3339_timestamp(invalid), "{invalid}");
    }
}

/// Run a read on another thread and require it to return within a bound, so
/// a regression that blocks on a FIFO fails the test instead of hanging it.
#[cfg(unix)]
fn promptly(path: PathBuf, located: fs::Metadata) -> Result<Vec<u8>, SourceStatus> {
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = send.send(read_located(&path, &located));
    });
    receive
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("read_located blocked on a replacement")
}

#[cfg(unix)]
fn fifo(path: &Path) {
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: `name` is a valid NUL-terminated path for the call's duration.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
}

/// A located regular file replaced by a FIFO with no writer, or by a link to
/// one, returns promptly with an honest status and never blocks: the open is
/// nonblocking and no-follow, and the descriptor is validated before reading.
#[cfg(unix)]
#[test]
fn located_file_replaced_by_a_fifo_or_a_link_to_one_returns_promptly() {
    let (_guard, base) = temp();
    let path = base.join("mcp.json");
    fs::write(&path, "{}").unwrap();
    let located = fs::symlink_metadata(&path).unwrap();
    fs::remove_file(&path).unwrap();
    fifo(&path);
    assert_eq!(
        promptly(path.clone(), located.clone()),
        Err(SourceStatus::Unsupported {
            reason: UnsupportedReason::NotRegularFile
        })
    );
    fs::remove_file(&path).unwrap();
    let elsewhere = base.join("elsewhere.fifo");
    fifo(&elsewhere);
    std::os::unix::fs::symlink(&elsewhere, &path).unwrap();
    assert_eq!(
        promptly(path.clone(), located),
        Err(SourceStatus::Unsupported {
            reason: UnsupportedReason::Symlink
        })
    );
    // The whole probe over a root whose source is a FIFO also returns.
    let home = base.join("home");
    fs::create_dir_all(home.join(".cursor")).unwrap();
    fifo(&home.join(".cursor/mcp.json"));
    let (send, receive) = std::sync::mpsc::channel();
    let roots = ProbeRoots::new().with_home(&home).unwrap();
    std::thread::spawn(move || {
        let _ = send.send(probe(&roots));
    });
    let reading = receive
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("the probe blocked on a FIFO source");
    assert!(
        reading
            .sources
            .iter()
            .any(|row| row.source == ConfigSource::CursorMcpConfig
                && row.status
                    == SourceStatus::Unsupported {
                        reason: UnsupportedReason::NotRegularFile
                    })
    );
}
