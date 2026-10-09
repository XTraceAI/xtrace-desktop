//! Unit regressions for the probe's read primitives, where a file or a
//! directory can be changed between the walk and the read deterministically.
use super::*;
use std::{fs, io};

fn locate_path(path: &Path) -> Located {
    let parent = DirHandle::open_root(path.parent().unwrap()).unwrap();
    let name = path.file_name().unwrap().to_owned();
    let metadata = parent.metadata(&name).unwrap();
    Located {
        parent,
        name,
        metadata,
    }
}

fn temp() -> (tempfile::TempDir, PathBuf) {
    let guard = tempfile::TempDir::new().unwrap();
    // Test setup only: the probe itself never resolves a path.
    let base = guard.path().canonicalize().unwrap();
    (guard, base)
}

/// The root check must not allow a later source read to enter a replacement
/// root or a replacement ancestor. All paths here belong to one temporary tree.
#[cfg(unix)]
#[test]
fn checked_root_or_ancestor_replaced_by_a_link_never_reads_the_target() {
    let mut redirects = Vec::new();
    for replace_ancestor in [false, true] {
        let (_guard, base) = temp();
        let parent = base.join("parent");
        let root = parent.join("home");
        let outside = base.join("outside");
        fs::create_dir_all(&root).unwrap();
        let outside_root = if replace_ancestor {
            outside.join("home")
        } else {
            outside.clone()
        };
        fs::create_dir_all(&outside_root).unwrap();
        fs::write(root.join(".claude.json"), "{}").unwrap();
        fs::write(
            outside_root.join(".claude.json"),
            r#"{"mcpServers":{"redirected-target":{}}}"#,
        )
        .unwrap();
        let pinned_root = DirHandle::open_root(&root).unwrap();
        let changed = if replace_ancestor { &parent } else { &root };
        fs::rename(changed, base.join("original")).unwrap();
        std::os::unix::fs::symlink(&outside, changed).unwrap();
        let result = bounded_text(Known::new(&pinned_root, ".claude.json"));
        if result
            .as_ref()
            .is_ok_and(|text| text.contains("redirected-target"))
        {
            redirects.push(replace_ancestor);
        }
    }
    assert!(
        redirects.is_empty(),
        "redirected with ancestor replacements: {redirects:?}"
    );
}

/// Replace a skill child or an ancestor only after the listing has saved
/// the child's type. A replacement target's SKILL.md must prove nothing.
#[cfg(unix)]
#[test]
fn listed_skill_child_or_ancestor_replaced_by_a_link_is_not_accepted() {
    let mut redirects = Vec::new();
    for replace_ancestor in [false, true] {
        let (_guard, base) = temp();
        let parent = base.join("parent");
        let skills = parent.join("skills");
        let child = skills.join("candidate");
        let outside = base.join("outside");
        fs::create_dir_all(&child).unwrap();
        let target_child = if replace_ancestor {
            outside.join("skills/candidate")
        } else {
            outside.clone()
        };
        fs::create_dir_all(&target_child).unwrap();
        fs::write(target_child.join("SKILL.md"), "replacement target").unwrap();
        let located = locate_path(&skills);
        let listing = list_located(&located).unwrap();
        assert_eq!(listing.entries.len(), 1);
        let changed = if replace_ancestor { &parent } else { &child };
        fs::rename(changed, base.join("original")).unwrap();
        std::os::unix::fs::symlink(&outside, changed).unwrap();
        if !stated_skills(listing).entries.is_empty() {
            redirects.push(replace_ancestor);
        }
    }
    assert!(redirects.is_empty(), "accepted replacements: {redirects:?}");
}

/// A source parent replaced after location cannot redirect a later file open
/// or directory enumeration. Both operations use the original parent handle.
#[cfg(unix)]
#[test]
fn located_sources_use_the_original_parent_after_an_intermediate_link_swap() {
    let (_guard, base) = temp();
    let root = base.join("home");
    let cursor = root.join(".cursor");
    let outside = base.join("outside");
    fs::create_dir_all(cursor.join("plugins/cache/original")).unwrap();
    fs::create_dir_all(outside.join("plugins/cache/redirected")).unwrap();
    fs::write(cursor.join("mcp.json"), "original file").unwrap();
    fs::write(outside.join("mcp.json"), "redirected file").unwrap();
    let pinned = DirHandle::open_root(&root).unwrap();
    let file = Known::new(&pinned, ".cursor/mcp.json").locate().unwrap();
    let cache = Known::new(&pinned, ".cursor/plugins/cache")
        .locate()
        .unwrap();
    fs::rename(&cursor, base.join("original-cursor")).unwrap();
    std::os::unix::fs::symlink(&outside, &cursor).unwrap();
    assert_eq!(read_located(&file).unwrap(), b"original file");
    file.confirm().unwrap();
    let listing = list_located(&cache).unwrap();
    assert_eq!(
        listing
            .entries
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["original"]
    );
}

/// A different real skill directory cannot reuse the old directory's saved
/// identity. A missing SKILL.md in the original must stay unproven.
#[cfg(unix)]
#[test]
fn listed_skill_replaced_by_another_real_directory_is_skipped() {
    let (_guard, base) = temp();
    let skills = base.join("skills");
    let child = skills.join("candidate");
    let replacement = base.join("replacement");
    fs::create_dir_all(&child).unwrap();
    fs::create_dir_all(&replacement).unwrap();
    fs::write(replacement.join("SKILL.md"), "replacement").unwrap();
    let listing = list_located(&locate_path(&skills)).unwrap();
    fs::rename(&child, base.join("original")).unwrap();
    fs::rename(&replacement, &child).unwrap();
    let stated = stated_skills(listing);
    assert!(stated.entries.is_empty());
    assert_eq!(
        stated.status,
        SourceStatus::Incomplete {
            stated: 1,
            skipped: 1,
            reason: IncompleteReason::UnprovenEntry,
        }
    );
}

/// Each enumeration owns a separate stream and offset. Dot entries never
/// consume the bound, and a cloned parent cannot advance the next listing.
#[test]
fn repeated_handle_enumerations_are_bounded_and_start_at_the_beginning() {
    let (_guard, base) = temp();
    for index in 0..4 {
        fs::create_dir(base.join(format!("entry-{index}"))).unwrap();
    }
    let directory = DirHandle::open_root(&base).unwrap();
    let cloned = directory.clone_handle().unwrap();
    for handle in [&directory, &cloned, &directory] {
        assert_eq!(handle.names(2).unwrap().len(), 2);
        let mut names = handle.names(MAX_SOURCE_ENTRIES + 1).unwrap();
        names.sort();
        assert_eq!(
            names,
            (0..4)
                .map(|index| OsString::from(format!("entry-{index}")))
                .collect::<Vec<_>>()
        );
    }
}

#[cfg(unix)]
struct RestoreSearchPermissions(Vec<PathBuf>);

#[cfg(unix)]
impl Drop for RestoreSearchPermissions {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        for path in &self.0 {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
}

/// Ancestors, source parents and skill children need only search permission.
/// The skill listing itself still has read permission for enumeration.
#[cfg(unix)]
#[test]
fn search_only_ancestors_and_skill_children_preserve_configured_facts() {
    use std::os::unix::fs::PermissionsExt;
    let (_guard, base) = temp();
    let parent = base.join("parent");
    let home = parent.join("home");
    let cursor = home.join(".cursor");
    let claude = home.join(".claude");
    let skill = claude.join("skills/search-only-skill");
    fs::create_dir_all(&cursor).unwrap();
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        cursor.join("mcp.json"),
        r#"{"mcpServers":{"search-only-server":{}}}"#,
    )
    .unwrap();
    fs::write(skill.join("SKILL.md"), "marker").unwrap();
    let restore = RestoreSearchPermissions(vec![
        parent,
        home.clone(),
        cursor.clone(),
        claude,
        skill.clone(),
    ]);
    for path in &restore.0 {
        fs::set_permissions(path, fs::Permissions::from_mode(0o100)).unwrap();
    }
    assert!(
        fs::read_to_string(cursor.join("mcp.json"))
            .unwrap()
            .contains("search-only-server")
    );
    assert!(
        fs::symlink_metadata(skill.join("SKILL.md"))
            .unwrap()
            .is_file()
    );
    // A privileged CI user can bypass these permissions. The configured
    // facts below must still be preserved, regardless of the test user's UID.
    if let Err(error) = fs::read_dir(&home) {
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
    }
    let reading = probe(&ProbeRoots::new().with_home(&home).unwrap());
    assert_eq!(reading.roots[0].state, RootState::Read);
    assert!(
        reading
            .components
            .iter()
            .any(|row| row.name == "search-only-server")
    );
    assert!(
        reading
            .components
            .iter()
            .any(|row| row.name == "search-only-skill")
    );
}

/// A child can vanish after readdir returned its name. The containing source
/// remains present; it must count the child as skipped rather than be Missing.
#[test]
fn vanished_listed_child_is_skipped_without_marking_the_source_missing() {
    let (_guard, base) = temp();
    let skills = base.join("skills");
    fs::create_dir_all(skills.join("vanishing")).unwrap();
    let directory = DirHandle::open_root(&skills).unwrap();
    let names = directory.names(MAX_SOURCE_ENTRIES + 1).unwrap();
    assert_eq!(names.len(), 1);
    fs::remove_dir(skills.join("vanishing")).unwrap();
    let entries = listed_entries(&directory, names).expect("a vanished child is skipped");
    assert!(entries.is_empty());
    let listing = Listing {
        directory,
        entries,
        stated: 1,
        limited: false,
    };
    let expected = SourceStatus::Incomplete {
        stated: 1,
        skipped: 1,
        reason: IncompleteReason::UnprovenEntry,
    };
    assert_eq!(
        count_status(listing.entries.len(), listing.stated, listing.limited),
        expected
    );
    assert_eq!(stated_skills(listing).status, expected);
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
    let located = locate_path(&path);
    assert!(located.metadata.len() < MAX_SOURCE_BYTES);
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    io::Write::write_all(
        &mut file,
        &vec![b' '; usize::try_from(MAX_SOURCE_BYTES).unwrap()],
    )
    .unwrap();
    assert_eq!(
        read_located(&located),
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
    let located = locate_path(&path);
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&outside, &path).unwrap();
    assert_eq!(
        read_located(&located),
        Err(SourceStatus::Unsupported {
            reason: UnsupportedReason::Symlink
        })
    );
    let replacement = base.join("replacement.json");
    fs::write(&replacement, r#"{"mcpServers":{"swapped":{}}}"#).unwrap();
    fs::remove_file(&path).unwrap();
    fs::rename(&replacement, &path).unwrap();
    assert_eq!(read_located(&located), Err(SourceStatus::Unreadable));
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
    let located = locate_path(&path);
    fs::remove_dir(&path).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &path).unwrap();
    assert!(matches!(
        list_located(&located),
        Err(SourceStatus::Unsupported {
            reason: UnsupportedReason::Symlink
        })
    ));
}

/// A final name that is replaced after the read fails the parent-handle check.
#[test]
fn a_walk_that_no_longer_reaches_the_read_object_invalidates_the_read() {
    let (_guard, base) = temp();
    fs::create_dir_all(base.join(".cursor")).unwrap();
    fs::write(base.join(".cursor/mcp.json"), "{}").unwrap();
    let pinned = DirHandle::open_root(&base).unwrap();
    let known = Known::new(&pinned, ".cursor/mcp.json");
    let read = known.locate().unwrap();
    fs::rename(
        base.join(".cursor/mcp.json"),
        base.join(".cursor/original.json"),
    )
    .unwrap();
    fs::write(base.join(".cursor/other.json"), "{}").unwrap();
    fs::write(base.join(".cursor/mcp.json"), "{}").unwrap();
    // A new file at the same path is another object.
    let now = known.locate().unwrap();
    assert!(!read.metadata.same_object(now.metadata));
    assert_eq!(read.confirm(), Err(SourceStatus::Unreadable));
    fs::remove_file(base.join(".cursor/mcp.json")).unwrap();
    assert_eq!(read.confirm(), Err(SourceStatus::Missing));
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
fn promptly(located: Located) -> Result<Vec<u8>, SourceStatus> {
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = send.send(read_located(&located));
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
    let located = locate_path(&path);
    fs::remove_file(&path).unwrap();
    fifo(&path);
    assert_eq!(
        promptly(Located {
            parent: located.parent.clone_handle().unwrap(),
            name: located.name.clone(),
            metadata: located.metadata
        }),
        Err(SourceStatus::Unsupported {
            reason: UnsupportedReason::NotRegularFile
        })
    );
    fs::remove_file(&path).unwrap();
    let elsewhere = base.join("elsewhere.fifo");
    fifo(&elsewhere);
    std::os::unix::fs::symlink(&elsewhere, &path).unwrap();
    assert_eq!(
        promptly(located),
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
