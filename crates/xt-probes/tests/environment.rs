//! Environment probe acceptance. Every case supplies its roots explicitly:
//! nothing in this file resolves a home directory, and the probe has no
//! default root, so no test can accidentally read the developer's machine.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use xt_probes::environment::{
    self, CacheSource, ComponentKind, ConfigSource, ConfiguredComponent, EnvironmentProbe,
    IncompleteReason, MAX_NAME_CHARS, MAX_SOURCE_BYTES, MAX_SOURCE_ENTRIES, ProbeHost, ProbeRoots,
    RootRejected, RootScope, RootState, SourceStatus, UnsupportedReason,
};

/// The committed F16 matrix root, declared in F16's `env` snapshot index.
/// Built without a relative component, because the probe refuses one.
fn matrix() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .join("fixtures/F16/input/probes/env")
}

fn index() -> serde_json::Value {
    let text =
        fs::read_to_string(matrix().parent().unwrap().join("env.json")).expect("F16 env index");
    serde_json::from_str(&text).expect("F16 env index is JSON")
}

fn f16_roots() -> ProbeRoots {
    let index = index();
    let base = matrix().parent().unwrap().to_path_buf();
    let home = base.join(index["roots"]["home"].as_str().unwrap());
    let mut roots = ProbeRoots::new().with_home(home).unwrap();
    for repository in index["roots"]["repositories"].as_array().unwrap() {
        roots = roots
            .with_repository(base.join(repository.as_str().unwrap()).to_str().unwrap())
            .unwrap();
    }
    roots
}

fn status(
    probe: &EnvironmentProbe,
    source: ConfigSource,
    scope: RootScope,
    root: u32,
) -> SourceStatus {
    probe
        .sources
        .iter()
        .find(|row| row.source == source && row.scope == scope && row.root_index == root)
        .unwrap_or_else(|| panic!("{source:?} {scope:?} {root} was never looked at"))
        .status
}

fn names(probe: &EnvironmentProbe, source: ConfigSource) -> Vec<&str> {
    probe
        .components
        .iter()
        .filter(|component| component.source == source)
        .map(|component| component.name.as_str())
        .collect()
}

/// A temporary root with no linked ancestor. On macOS the default temporary
/// directory sits under the linked `/var`, which the probe refuses; the test,
/// not the probe, resolves it once.
fn temp() -> (tempfile::TempDir, PathBuf) {
    let guard = tempfile::TempDir::new().unwrap();
    let root = guard.path().canonicalize().unwrap();
    (guard, root)
}

fn incomplete(stated: u32, skipped: u32) -> SourceStatus {
    SourceStatus::Incomplete {
        stated,
        skipped,
        reason: IncompleteReason::UnprovenEntry,
    }
}

/// Every documented source of the F16 matrix, its status, and the components
/// it states: the whole supported registry matrix in one reading.
#[test]
fn environment_reads_every_documented_f16_source_and_states_its_status() {
    let probe = environment::probe(&f16_roots());
    assert_eq!(
        probe
            .roots
            .iter()
            .map(|root| (root.scope, root.index, root.state))
            .collect::<Vec<_>>(),
        [
            (RootScope::Home, 0, RootState::Read),
            (RootScope::Repository, 0, RootState::Read),
            (RootScope::Repository, 1, RootState::Read),
        ]
    );

    // A directory holding SKILL.md is a skill; a sibling without one is not,
    // so the source is read but incomplete rather than reported as two skills.
    assert_eq!(
        status(
            &probe,
            ConfigSource::ClaudeSkillDirectory,
            RootScope::Home,
            0
        ),
        incomplete(2, 1)
    );
    assert_eq!(
        status(
            &probe,
            ConfigSource::ClaudeSkillDirectory,
            RootScope::Repository,
            0
        ),
        SourceStatus::Read { stated: 1 }
    );
    assert_eq!(
        status(
            &probe,
            ConfigSource::ClaudeSkillDirectory,
            RootScope::Repository,
            1
        ),
        SourceStatus::Missing
    );
    assert_eq!(
        names(&probe, ConfigSource::ClaudeSkillDirectory),
        ["deep-review", "repo-only"]
    );

    // MCP configuration states servers, never those servers' tools.
    assert_eq!(
        status(
            &probe,
            ConfigSource::ClaudeUserMcpConfig,
            RootScope::Home,
            0
        ),
        SourceStatus::Read { stated: 2 }
    );
    assert_eq!(
        names(&probe, ConfigSource::ClaudeUserMcpConfig),
        ["github", "memhub"]
    );
    assert_eq!(
        names(&probe, ConfigSource::ClaudeProjectMcpConfig),
        ["memhub", "repo-only"]
    );
    assert_eq!(
        status(&probe, ConfigSource::CodexMcpConfig, RootScope::Home, 0),
        SourceStatus::Read { stated: 2 }
    );
    assert_eq!(
        names(&probe, ConfigSource::CodexMcpConfig),
        ["memhub", "stitch"]
    );
    assert_eq!(
        names(&probe, ConfigSource::CursorMcpConfig),
        ["html.to.design"]
    );
    assert!(
        probe
            .components
            .iter()
            .filter(|component| component.kind == ComponentKind::McpServer)
            .all(|component| !component.name.contains("mcp__")),
        "a configured MCP server must never be expanded into tool identities"
    );

    // The plugin registry counts an entry as installed only when the fields
    // that document an installation are all present.
    assert_eq!(
        status(
            &probe,
            ConfigSource::ClaudePluginRegistry,
            RootScope::Home,
            0
        ),
        incomplete(2, 1)
    );
    assert_eq!(
        names(&probe, ConfigSource::ClaudePluginRegistry),
        ["memhub@memhub"]
    );

    // A stated boolean is preserved exactly; a non-boolean value is unproven.
    assert_eq!(
        status(
            &probe,
            ConfigSource::ClaudeEnabledPlugins,
            RootScope::Home,
            0
        ),
        incomplete(3, 1)
    );
    assert_eq!(
        probe
            .components
            .iter()
            .filter(|component| component.source == ConfigSource::ClaudeEnabledPlugins)
            .map(|component| (component.name.as_str(), component.enabled))
            .collect::<Vec<_>>(),
        [
            ("cloudflare@cloudflare", Some(false)),
            ("memhub@memhub", Some(true))
        ]
    );

    // A hook entry is named by event and matcher; a group stating no hook is
    // not a configured hook entry.
    assert_eq!(
        status(&probe, ConfigSource::ClaudeHookConfig, RootScope::Home, 0),
        incomplete(3, 1)
    );
    assert_eq!(
        names(&probe, ConfigSource::ClaudeHookConfig),
        ["PreToolUse:Bash", "Stop:*"]
    );

    // A cache directory is counted separately and states no component.
    assert_eq!(
        probe
            .cache
            .iter()
            .map(|row| (row.source, row.status))
            .collect::<Vec<_>>(),
        [
            (
                CacheSource::CodexPluginCache,
                SourceStatus::Read { stated: 1 }
            ),
            (CacheSource::CursorPluginCache, SourceStatus::Missing),
        ]
    );
    assert_eq!(probe.sources.len(), 13);
    assert_eq!(probe.components.len(), 14);
    assert_eq!(
        probe
            .components
            .iter()
            .filter(|component| component.kind == ComponentKind::Plugin)
            .count(),
        3,
        "a cached download never becomes a plugin component"
    );
}

/// Deduplication is by host, source and structural name. Two repository roots
/// stating the same server yield one component and two source observations;
/// the same name under a different source stays a separate configured fact.
#[test]
fn environment_deduplicates_components_by_host_source_and_structural_name() {
    let probe = environment::probe(&f16_roots());
    let memhub: Vec<&ConfiguredComponent> = probe
        .components
        .iter()
        .filter(|component| component.name == "memhub")
        .collect();
    assert_eq!(
        memhub
            .iter()
            .map(|component| (component.host, component.source, component.scope))
            .collect::<Vec<_>>(),
        [
            (
                ProbeHost::Claude,
                ConfigSource::ClaudeUserMcpConfig,
                RootScope::Home
            ),
            (
                ProbeHost::Claude,
                ConfigSource::ClaudeProjectMcpConfig,
                RootScope::Repository
            ),
            (
                ProbeHost::Codex,
                ConfigSource::CodexMcpConfig,
                RootScope::Home
            ),
        ]
    );
    // Both repository roots state it; both readings are reported.
    for root in [0, 1] {
        assert_eq!(
            status(
                &probe,
                ConfigSource::ClaudeProjectMcpConfig,
                RootScope::Repository,
                root
            ),
            SourceStatus::Read {
                stated: if root == 0 { 2 } else { 1 }
            }
        );
    }
    // The first root to state a component states it: the home cursor entry
    // keeps its home scope although a repository root repeats it.
    let cursor: Vec<_> = probe
        .components
        .iter()
        .filter(|component| component.source == ConfigSource::CursorMcpConfig)
        .collect();
    assert_eq!(cursor.len(), 1);
    assert_eq!(cursor[0].scope, RootScope::Home);
    assert_eq!(
        status(
            &probe,
            ConfigSource::CursorMcpConfig,
            RootScope::Repository,
            1
        ),
        SourceStatus::Read { stated: 1 }
    );
    // The reading is a pure function of the supplied roots.
    assert_eq!(probe, environment::probe(&f16_roots()));
}

/// The configured facts carry structural names only. No command, argument,
/// environment value, credential, URL or installation path can reach a caller.
#[test]
fn environment_configured_facts_carry_no_command_argument_environment_or_url() {
    let probe = environment::probe(&f16_roots());
    let marker = index()["secret_marker"].as_str().unwrap().to_owned();
    let serialized = serde_json::to_string(&probe).unwrap();
    // The marker really is in the matrix, so its absence here is a result.
    let sources = [
        "home/.claude.json",
        "home/.claude/settings.json",
        "home/.codex/config.toml",
    ];
    assert!(sources.iter().all(|path| {
        fs::read_to_string(matrix().join(path))
            .unwrap()
            .contains(&marker)
    }));
    for forbidden in [
        marker.as_str(),
        "/synthetic/bin",
        "/synthetic/home",
        "https://",
        "installPath",
        "Authorization",
        "Bearer",
        "GITHUB_TOKEN",
        "MEMHUB_TOKEN",
        "--token",
        "npx",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "{forbidden} reached the probe result"
        );
    }
    assert!(
        probe
            .components
            .iter()
            .all(|component| !component.name.contains('/') && !component.name.contains('\\')),
        "a structural name is never a path"
    );
}

/// Missing, empty, malformed, unsupported and unreadable are five different
/// answers. None of them measures zero configured components.
#[test]
fn environment_distinguishes_missing_empty_malformed_and_unreadable_sources() {
    let (_guard, root) = temp();
    let home = root.join("home");
    fs::create_dir_all(home.join(".claude/plugins")).unwrap();
    fs::create_dir_all(home.join(".cursor")).unwrap();
    fs::create_dir_all(home.join(".codex")).unwrap();
    // Present and stating nothing.
    fs::write(home.join(".claude.json"), "{}").unwrap();
    fs::write(home.join(".cursor/mcp.json"), r#"{"mcpServers":{}}"#).unwrap();
    // Present and unparseable in its documented shape.
    fs::write(home.join(".codex/config.toml"), "mcp_servers = 3").unwrap();
    fs::write(
        home.join(".claude/settings.json"),
        r#"{"hooks":"not an object"}"#,
    )
    .unwrap();
    // Present with a schema this probe does not document.
    fs::write(
        home.join(".claude/plugins/installed_plugins.json"),
        r#"{"version":1,"plugins":{"memhub@memhub":{"version":"1"}}}"#,
    )
    .unwrap();
    let probe = environment::probe(&ProbeRoots::new().with_home(&home).unwrap());
    assert!(probe.components.is_empty());
    assert_eq!(
        status(
            &probe,
            ConfigSource::ClaudeUserMcpConfig,
            RootScope::Home,
            0
        ),
        SourceStatus::Empty
    );
    assert_eq!(
        status(&probe, ConfigSource::CursorMcpConfig, RootScope::Home, 0),
        SourceStatus::Empty
    );
    assert_eq!(
        status(&probe, ConfigSource::CodexMcpConfig, RootScope::Home, 0),
        SourceStatus::Malformed
    );
    for source in [
        ConfigSource::ClaudeHookConfig,
        ConfigSource::ClaudeEnabledPlugins,
    ] {
        assert_eq!(
            status(&probe, source, RootScope::Home, 0),
            SourceStatus::Malformed
        );
    }
    assert_eq!(
        status(
            &probe,
            ConfigSource::ClaudePluginRegistry,
            RootScope::Home,
            0
        ),
        SourceStatus::Unsupported {
            reason: UnsupportedReason::UnsupportedSchema
        }
    );
    // A supported source this root simply does not have.
    assert_eq!(
        status(
            &probe,
            ConfigSource::ClaudeSkillDirectory,
            RootScope::Home,
            0
        ),
        SourceStatus::Missing
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let unreadable = root.join("locked");
        fs::create_dir_all(unreadable.join(".cursor")).unwrap();
        let path = unreadable.join(".cursor/mcp.json");
        fs::write(&path, r#"{"mcpServers":{"x":{}}}"#).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        let probe = environment::probe(&ProbeRoots::new().with_home(&unreadable).unwrap());
        let observed = status(&probe, ConfigSource::CursorMcpConfig, RootScope::Home, 0);
        // A shell that can read anything (a root CI user) would read it.
        assert!(
            observed == SourceStatus::Unreadable || observed == SourceStatus::Read { stated: 1 },
            "{observed:?}"
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}

/// A symbolic link is never followed: it can point outside the supplied root.
/// A source that is not a regular file, or a directory source that is not a
/// directory, is refused with its own reason instead of being parsed.
#[cfg(unix)]
#[test]
fn environment_skips_symlinks_and_nonregular_files_with_an_explicit_reason() {
    use std::os::unix::fs::symlink;
    let (_guard, root) = temp();
    let outside = root.join("outside.json");
    fs::write(&outside, r#"{"mcpServers":{"escaped":{}}}"#).unwrap();
    let home = root.join("home");
    fs::create_dir_all(home.join(".cursor")).unwrap();
    fs::create_dir_all(home.join(".claude")).unwrap();
    symlink(&outside, home.join(".cursor/mcp.json")).unwrap();
    symlink(root.join("elsewhere"), home.join(".claude/skills")).unwrap();
    // A documented file path occupied by a directory is not a regular file.
    fs::create_dir_all(home.join(".claude.json")).unwrap();
    let probe = environment::probe(&ProbeRoots::new().with_home(&home).unwrap());
    assert!(probe.components.is_empty(), "no link was followed");
    assert_eq!(
        status(&probe, ConfigSource::CursorMcpConfig, RootScope::Home, 0),
        SourceStatus::Unsupported {
            reason: UnsupportedReason::Symlink
        }
    );
    assert_eq!(
        status(
            &probe,
            ConfigSource::ClaudeSkillDirectory,
            RootScope::Home,
            0
        ),
        SourceStatus::Unsupported {
            reason: UnsupportedReason::Symlink
        }
    );
    assert_eq!(
        status(
            &probe,
            ConfigSource::ClaudeUserMcpConfig,
            RootScope::Home,
            0
        ),
        SourceStatus::Unsupported {
            reason: UnsupportedReason::NotRegularFile
        }
    );

    // A linked entry inside a real skills directory is skipped too.
    let second = root.join("second");
    fs::create_dir_all(second.join(".claude/skills/real")).unwrap();
    fs::write(second.join(".claude/skills/real/SKILL.md"), "---\n").unwrap();
    symlink(
        second.join(".claude/skills/real"),
        second.join(".claude/skills/linked"),
    )
    .unwrap();
    let probe = environment::probe(&ProbeRoots::new().with_home(&second).unwrap());
    assert_eq!(names(&probe, ConfigSource::ClaudeSkillDirectory), ["real"]);
    // The linked entry is stated and skipped, never silently dropped.
    assert_eq!(
        status(
            &probe,
            ConfigSource::ClaudeSkillDirectory,
            RootScope::Home,
            0
        ),
        incomplete(2, 1)
    );

    // A root that is itself a link, a file, or absent is never read.
    let linked_home = root.join("linked-home");
    symlink(&second, &linked_home).unwrap();
    let probe = environment::probe(
        &ProbeRoots::new()
            .with_home(&linked_home)
            .unwrap()
            .with_home(&outside)
            .unwrap()
            .with_home(root.join("absent"))
            .unwrap(),
    );
    assert_eq!(
        probe.roots.iter().map(|row| row.state).collect::<Vec<_>>(),
        [
            RootState::Symlink,
            RootState::NotDirectory,
            RootState::Missing
        ]
    );
    assert!(probe.sources.is_empty() && probe.components.is_empty());
}

/// Reads are bounded in bytes and in entries. Neither bound silently truncates
/// a result: an over-long name is skipped and an over-full source is reported
/// incomplete with the overflow counted.
#[test]
fn environment_bounds_file_size_entry_count_and_name_length() {
    let (_guard, root) = temp();
    let home = root.join("home");
    fs::create_dir_all(home.join(".cursor")).unwrap();
    fs::create_dir_all(home.join(".claude/skills")).unwrap();
    fs::write(
        home.join(".cursor/mcp.json"),
        vec![b' '; usize::try_from(MAX_SOURCE_BYTES).unwrap() + 1],
    )
    .unwrap();
    let overflow = MAX_SOURCE_ENTRIES + 10;
    let servers: BTreeMap<String, serde_json::Value> = (0..overflow)
        .map(|index| (format!("server-{index:04}"), serde_json::json!({})))
        .collect();
    fs::write(
        home.join(".claude.json"),
        serde_json::to_string(&serde_json::json!({ "mcpServers": servers })).unwrap(),
    )
    .unwrap();
    for name in [
        "kept".to_owned(),
        "x".repeat(MAX_NAME_CHARS + 1),
        " ".to_owned(),
    ] {
        let directory = home.join(".claude/skills").join(&name);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("SKILL.md"), "---\n").unwrap();
    }
    let probe = environment::probe(&ProbeRoots::new().with_home(&home).unwrap());
    assert_eq!(
        status(&probe, ConfigSource::CursorMcpConfig, RootScope::Home, 0),
        SourceStatus::Unsupported {
            reason: UnsupportedReason::FileTooLarge
        }
    );
    assert_eq!(
        status(
            &probe,
            ConfigSource::ClaudeUserMcpConfig,
            RootScope::Home,
            0
        ),
        SourceStatus::Incomplete {
            stated: u32::try_from(overflow).unwrap(),
            skipped: 10,
            reason: IncompleteReason::EntryLimit
        }
    );
    assert_eq!(
        names(&probe, ConfigSource::ClaudeUserMcpConfig).len(),
        MAX_SOURCE_ENTRIES
    );
    assert_eq!(names(&probe, ConfigSource::ClaudeSkillDirectory), ["kept"]);
    assert_eq!(
        status(
            &probe,
            ConfigSource::ClaudeSkillDirectory,
            RootScope::Home,
            0
        ),
        incomplete(3, 2)
    );
}

/// A repository root comes from stored local session metadata only. A remote
/// identifier, a relative path and any relative component are refused before a
/// file is opened; no Git command runs and no parent directory is consulted.
#[test]
fn environment_refuses_remote_identifiers_relative_paths_and_traversal() {
    for remote in [
        "https://github.com/XTraceAI/xtrace-desktop",
        "git@github.com:XTraceAI/xtrace-desktop.git",
        "ssh://git@github.com/XTraceAI/xtrace-desktop",
    ] {
        assert_eq!(
            ProbeRoots::new().with_repository(remote).unwrap_err(),
            RootRejected::RemoteIdentifier
        );
    }
    for relative in ["XTraceAI/xtrace-desktop", "work/repo", "", "   "] {
        assert_eq!(
            ProbeRoots::new().with_repository(relative).unwrap_err(),
            RootRejected::NotAbsolute
        );
    }
    for traversal in ["/work/../etc", "/..", "/work/repo/.."] {
        assert_eq!(
            ProbeRoots::new().with_repository(traversal).unwrap_err(),
            RootRejected::Traversal
        );
        assert_eq!(
            ProbeRoots::new()
                .with_home(Path::new(traversal))
                .unwrap_err(),
            RootRejected::Traversal
        );
    }
    assert_eq!(
        ProbeRoots::new()
            .with_home(Path::new("relative"))
            .unwrap_err(),
        RootRejected::NotAbsolute
    );
    // A repeated root, however spelled, is supplied once and read once.
    let roots = ProbeRoots::new()
        .with_repository("/work/repo")
        .unwrap()
        .with_repository(" /work/repo ")
        .unwrap()
        .with_repository("/work/./repo")
        .unwrap()
        .with_repository("/work//repo/")
        .unwrap();
    assert_eq!(roots.repositories(), [PathBuf::from("/work/repo")]);
}

/// There is no implicit home: a probe with no supplied root opens nothing.
#[test]
fn environment_reads_nothing_without_an_explicitly_supplied_root() {
    let probe = environment::probe(&ProbeRoots::new());
    assert_eq!(
        probe,
        EnvironmentProbe {
            roots: Vec::new(),
            components: Vec::new(),
            sources: Vec::new(),
            cache: Vec::new(),
        }
    );
    assert!(ProbeRoots::new().homes().is_empty() && ProbeRoots::new().repositories().is_empty());
}

/// No process is launched and no socket is opened, by construction: the probe
/// module names no process, network or ambient-environment API, and the crate
/// declares no dependency that could provide one.
#[test]
fn environment_launches_no_process_and_opens_no_socket_by_construction() {
    let module = include_str!("../src/environment.rs");
    let root = include_str!("../src/lib.rs");
    for forbidden in [
        "std::process",
        "Command",
        "std::net",
        "TcpStream",
        "UdpSocket",
        "std::env",
        "env::var",
        "home_dir",
        "read_link",
        "canonicalize",
        "WalkDir",
        "unsafe",
    ] {
        for (name, source) in [("environment.rs", module), ("lib.rs", root)] {
            assert!(!source.contains(forbidden), "{name} names {forbidden}");
        }
    }
    let manifest = include_str!("../Cargo.toml");
    for dependency in ["reqwest", "ureq", "tokio", "hyper", "which", "dirs", "home"] {
        assert!(!manifest.contains(dependency), "{dependency} is declared");
    }
    // The only filesystem entry points this probe uses are reads: files are
    // opened read-only, no-follow and nonblocking, and consumed only through
    // the bounded reader.
    for allowed in [
        "fs::symlink_metadata",
        "fs::read_dir",
        ".read(true)",
        "libc::O_NOFOLLOW | libc::O_NONBLOCK",
        ".take(MAX_SOURCE_BYTES + 1)",
    ] {
        assert!(module.contains(allowed), "{allowed} is missing");
    }
    for forbidden in [
        "fs::read(",
        "fs::read_to_string",
        "fs::metadata(",
        "fs::write",
        "fs::create_dir",
        "fs::remove",
        ".write(true)",
        ".append(",
        ".create(",
        ".create_new(",
        ".truncate(",
    ] {
        assert!(
            !module.contains(forbidden),
            "{forbidden} is unbounded or writes"
        );
    }
}

/// A link at any intermediate component, from a supplied root down to a
/// documented source, is skipped with the `symlink` reason. Nothing the
/// outside target states is read into a fact.
#[cfg(unix)]
#[test]
fn environment_skips_linked_intermediate_directories_and_reads_no_outside_fact() {
    use std::os::unix::fs::symlink;
    let (_guard, root) = temp();
    let outside = root.join("outside");
    for (path, text) in [
        (
            "claude/settings.json",
            r#"{"enabledPlugins":{"outside@m":true},"hooks":{"Stop":[{"hooks":[{"type":"command"}]}]}}"#,
        ),
        (
            "claude/plugins/installed_plugins.json",
            r#"{"version":2,"plugins":{"outside@m":[{"scope":"user","installPath":"/p","installedAt":"2026-09-01T00:00:00Z"}]}}"#,
        ),
        ("claude/skills/outside-skill/SKILL.md", "---\n"),
        (
            "codex/config.toml",
            "[mcp_servers.outside]\ncommand = \"x\"\n",
        ),
        ("codex/plugins/cache/outside/marker", "x"),
        ("cursor/mcp.json", r#"{"mcpServers":{"outside":{}}}"#),
        ("cursor/plugins/cache/outside/marker", "x"),
    ] {
        let file = outside.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
    }
    let home = root.join("home");
    let repo = root.join("repo");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&repo).unwrap();
    for (link, target) in [
        (home.join(".claude"), outside.join("claude")),
        (home.join(".codex"), outside.join("codex")),
        (home.join(".cursor"), outside.join("cursor")),
        (repo.join(".claude"), outside.join("claude")),
        (repo.join(".cursor"), outside.join("cursor")),
    ] {
        symlink(target, link).unwrap();
    }
    let probe = environment::probe(
        &ProbeRoots::new()
            .with_home(&home)
            .unwrap()
            .with_repository(repo.to_str().unwrap())
            .unwrap(),
    );
    assert!(probe.components.is_empty(), "{:?}", probe.components);
    let linked = SourceStatus::Unsupported {
        reason: UnsupportedReason::Symlink,
    };
    for (source, scope) in [
        (ConfigSource::ClaudeSkillDirectory, RootScope::Home),
        (ConfigSource::ClaudePluginRegistry, RootScope::Home),
        (ConfigSource::ClaudeEnabledPlugins, RootScope::Home),
        (ConfigSource::ClaudeHookConfig, RootScope::Home),
        (ConfigSource::CodexMcpConfig, RootScope::Home),
        (ConfigSource::CursorMcpConfig, RootScope::Home),
        (ConfigSource::ClaudeSkillDirectory, RootScope::Repository),
        (ConfigSource::CursorMcpConfig, RootScope::Repository),
    ] {
        assert_eq!(
            status(&probe, source, scope, 0),
            linked,
            "{source:?} {scope:?}"
        );
    }
    assert_eq!(probe.cache.len(), 2);
    assert!(probe.cache.iter().all(|row| row.status == linked));
    // A source whose own path has no linked component is still examined.
    assert_eq!(
        status(
            &probe,
            ConfigSource::ClaudeUserMcpConfig,
            RootScope::Home,
            0
        ),
        SourceStatus::Missing
    );
    let serialized = serde_json::to_string(&probe).unwrap();
    assert!(!serialized.contains("outside"), "{serialized}");

    // A deeper link (`.claude/plugins`) is refused the same way.
    let deeper = root.join("deeper");
    fs::create_dir_all(deeper.join(".claude")).unwrap();
    symlink(
        outside.join("claude/plugins"),
        deeper.join(".claude/plugins"),
    )
    .unwrap();
    let probe = environment::probe(&ProbeRoots::new().with_home(&deeper).unwrap());
    assert_eq!(
        status(
            &probe,
            ConfigSource::ClaudePluginRegistry,
            RootScope::Home,
            0
        ),
        linked
    );
    assert!(probe.components.is_empty());

    // A supplied root reached through a linked ancestor is not read at all.
    let real_parent = root.join("real-parent");
    fs::create_dir_all(real_parent.join("home/.cursor")).unwrap();
    fs::write(
        real_parent.join("home/.cursor/mcp.json"),
        r#"{"mcpServers":{"through-ancestor":{}}}"#,
    )
    .unwrap();
    symlink(&real_parent, root.join("linked-parent")).unwrap();
    let probe = environment::probe(
        &ProbeRoots::new()
            .with_home(root.join("linked-parent/home"))
            .unwrap()
            .with_repository(root.join("linked-parent/home").to_str().unwrap())
            .unwrap(),
    );
    assert_eq!(
        probe.roots.iter().map(|row| row.state).collect::<Vec<_>>(),
        [RootState::Symlink, RootState::Symlink]
    );
    assert!(probe.sources.is_empty() && probe.cache.is_empty() && probe.components.is_empty());
}

/// A directory past the entry bound is enumerated only to the bound, reported
/// incomplete, and yields no component: an unordered listing cannot choose a
/// deterministic subset, so the answer is conservative rather than partial.
#[test]
fn environment_oversized_directories_are_incomplete_and_yield_no_component() {
    let (_guard, root) = temp();
    let home = root.join("home");
    for index in 0..MAX_SOURCE_ENTRIES + 20 {
        let skill = home.join(format!(".claude/skills/skill-{index:04}"));
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "---\n").unwrap();
        fs::create_dir_all(home.join(format!(".codex/plugins/cache/entry-{index:04}"))).unwrap();
    }
    let probe = environment::probe(&ProbeRoots::new().with_home(&home).unwrap());
    let bound = u32::try_from(MAX_SOURCE_ENTRIES + 1).unwrap();
    let limited = SourceStatus::Incomplete {
        stated: bound,
        skipped: bound,
        reason: IncompleteReason::EntryLimit,
    };
    assert_eq!(
        status(
            &probe,
            ConfigSource::ClaudeSkillDirectory,
            RootScope::Home,
            0
        ),
        limited
    );
    assert!(names(&probe, ConfigSource::ClaudeSkillDirectory).is_empty());
    assert_eq!(probe.cache[0].source, CacheSource::CodexPluginCache);
    assert_eq!(probe.cache[0].status, limited);
    // Exactly at the bound is complete.
    let (_second_guard, second) = temp();
    for index in 0..MAX_SOURCE_ENTRIES {
        let skill = second.join(format!(".claude/skills/skill-{index:04}"));
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "---\n").unwrap();
    }
    let probe = environment::probe(&ProbeRoots::new().with_home(&second).unwrap());
    assert_eq!(
        status(
            &probe,
            ConfigSource::ClaudeSkillDirectory,
            RootScope::Home,
            0
        ),
        SourceStatus::Read {
            stated: u32::try_from(MAX_SOURCE_ENTRIES).unwrap()
        }
    );
    assert_eq!(
        names(&probe, ConfigSource::ClaudeSkillDirectory).len(),
        MAX_SOURCE_ENTRIES
    );
}

/// An MCP server is a configured fact only when its value is an object.
#[test]
fn environment_mcp_server_values_must_be_objects() {
    let (_guard, root) = temp();
    let home = root.join("home");
    fs::create_dir_all(home.join(".cursor")).unwrap();
    fs::create_dir_all(home.join(".codex")).unwrap();
    fs::write(
        home.join(".claude.json"),
        r#"{"mcpServers":{"ok":{"command":"x"},"number":42,"null":null,"list":[],"text":"x","flag":true}}"#,
    )
    .unwrap();
    fs::write(home.join(".cursor/mcp.json"), r#"{"mcpServers":42}"#).unwrap();
    fs::write(
        home.join(".codex/config.toml"),
        "[mcp_servers]\nnumber = 42\ntext = \"x\"\n\n[mcp_servers.ok]\ncommand = \"x\"\n",
    )
    .unwrap();
    let probe = environment::probe(&ProbeRoots::new().with_home(&home).unwrap());
    assert_eq!(
        status(
            &probe,
            ConfigSource::ClaudeUserMcpConfig,
            RootScope::Home,
            0
        ),
        incomplete(6, 5)
    );
    assert_eq!(names(&probe, ConfigSource::ClaudeUserMcpConfig), ["ok"]);
    assert_eq!(
        status(&probe, ConfigSource::CursorMcpConfig, RootScope::Home, 0),
        SourceStatus::Malformed
    );
    assert_eq!(
        status(&probe, ConfigSource::CodexMcpConfig, RootScope::Home, 0),
        incomplete(3, 2)
    );
    assert_eq!(names(&probe, ConfigSource::CodexMcpConfig), ["ok"]);
}

/// A hook entry is a configured fact only in its documented shape: an object
/// group under a usable event, with an absent, `null` or nonempty string
/// matcher and a nonempty list of object entries.
#[test]
fn environment_hook_groups_and_entries_must_have_their_documented_shape() {
    let (_guard, root) = temp();
    let home = root.join("home");
    fs::create_dir_all(home.join(".claude")).unwrap();
    fs::write(home.join(".claude/settings.json"), r#"{"hooks":null}"#).unwrap();
    let probe = environment::probe(&ProbeRoots::new().with_home(&home).unwrap());
    assert_eq!(
        status(&probe, ConfigSource::ClaudeHookConfig, RootScope::Home, 0),
        SourceStatus::Empty
    );
    fs::write(
        home.join(".claude/settings.json"),
        r#"{"hooks":{
            "PreToolUse": null,
            "PostToolUse": 42,
            "Stop": [
                null,
                42,
                "Bash",
                {"hooks": null},
                {"hooks": []},
                {"hooks": [null]},
                {"hooks": [{"type": "command"}, 42]},
                {"matcher": "", "hooks": [{"type": "command"}]},
                {"matcher": 7, "hooks": [{"type": "command"}]},
                {"matcher": "Bash", "hooks": [{"type": "command", "command": "/secret/bin"}]},
                {"matcher": null, "hooks": [{"type": "command"}]}
            ],
            " ": [{"hooks": [{"type": "command"}]}]
        }}"#,
    )
    .unwrap();
    let probe = environment::probe(&ProbeRoots::new().with_home(&home).unwrap());
    assert_eq!(
        names(&probe, ConfigSource::ClaudeHookConfig),
        ["Stop:*", "Stop:Bash"]
    );
    assert_eq!(
        status(&probe, ConfigSource::ClaudeHookConfig, RootScope::Home, 0),
        incomplete(14, 12)
    );
    assert!(
        !serde_json::to_string(&probe)
            .unwrap()
            .contains("/secret/bin")
    );
}

/// An installation is proven only by a documented scope, a nonempty install
/// path and an RFC 3339 install time, all strings, in an object entry.
#[test]
fn environment_plugin_installation_proof_must_be_nonempty_and_schema_valid() {
    let (_guard, root) = temp();
    let home = root.join("home");
    fs::create_dir_all(home.join(".claude/plugins")).unwrap();
    fs::write(
        home.join(".claude/plugins/installed_plugins.json"),
        r#"{"version":2,"plugins":{
            "empty@m": [{"scope": "", "installPath": "", "installedAt": ""}],
            "blank@m": [{"scope": "user", "installPath": "  ", "installedAt": "2026-09-01T00:00:00Z"}],
            "scope@m": [{"scope": "galaxy", "installPath": "/p", "installedAt": "2026-09-01T00:00:00Z"}],
            "time@m": [{"scope": "user", "installPath": "/p", "installedAt": "yesterday"}],
            "typed@m": [{"scope": "user", "installPath": 7, "installedAt": "2026-09-01T00:00:00Z"}],
            "null@m": [null],
            "none@m": [],
            "scalar@m": 42,
            "ok@m": [
                {"scope": "user"},
                {"scope": "project", "installPath": "/p", "installedAt": "2026-09-01T00:00:00.000Z"}
            ]
        }}"#,
    )
    .unwrap();
    let probe = environment::probe(&ProbeRoots::new().with_home(&home).unwrap());
    assert_eq!(names(&probe, ConfigSource::ClaudePluginRegistry), ["ok@m"]);
    assert_eq!(
        status(
            &probe,
            ConfigSource::ClaudePluginRegistry,
            RootScope::Home,
            0
        ),
        incomplete(9, 8)
    );
}
