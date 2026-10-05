//! The environment probe: which components a host's own configuration states,
//! read from a closed list of documented files under explicitly supplied roots.
//!
//! What this probe can and cannot establish is the whole point of its shape.
//!
//! * It reports **configured** components — an entry a supported registry
//!   states. Configured is not installed, not callable and not called. The
//!   registries available to a local reader cannot prove a complete callable
//!   host inventory, so this probe never claims one: it returns what it
//!   verified and a status for every source it looked at, and it leaves
//!   "not installed", "never called" and any used/installed ratio underivable.
//! * It never launches a process, opens a socket, walks a home directory
//!   recursively, reads a transcript, or activates anything. Every path it
//!   opens is one of the documented paths below, joined onto a root the caller
//!   supplied. Nothing is discovered; nothing is implicit.
//! * It never returns hook-command, argument, environment, credential, URL
//!   or installation-path field values. Only structural names, counts and
//!   statuses leave this module; a name can itself contain path-like text.
//! * It follows no link. Every component from the filesystem root to a
//!   supplied root, and from that root to each documented source and to each
//!   directory entry, is examined without being followed; a link anywhere on
//!   that path is skipped with the `symlink` reason. Directory handles pin each
//!   parent before a child is inspected or opened. An opened file must be the
//!   object found through that parent; replacement links cannot redirect IO.
//! * Its reads are bounded by construction: at most [`MAX_SOURCE_BYTES`] + 1
//!   bytes are consumed from a file and at most [`MAX_SOURCE_ENTRIES`] + 1
//!   entries are enumerated from a directory, whatever either later becomes.
//! * A fact is accepted only in its documented shape: an MCP server's value is
//!   an object, a hook group is an object with a valid matcher and a nonempty
//!   list of object entries, and a plugin installation states a documented
//!   scope, a nonempty install path and an install time the locked RFC 3339
//!   parser accepts. Anything
//!   else is counted as skipped and yields no component.
//!
//! # Documented source matrix
//!
//! | Host | Scope | Path under the root | States |
//! |---|---|---|---|
//! | claude | home, repository | `.claude/skills/<name>/SKILL.md` | standalone skill directories |
//! | claude | home | `.claude.json` → `mcpServers` | MCP server names |
//! | claude | repository | `.mcp.json` → `mcpServers` | MCP server names |
//! | claude | home | `.claude/plugins/installed_plugins.json` (schema 2) | installed plugins |
//! | claude | home | `.claude/settings.json` → `enabledPlugins` | plugin enablement |
//! | claude | home | `.claude/settings.json` → `hooks` | configured hook entries |
//! | codex | home | `.codex/config.toml` → `[mcp_servers]` | MCP server names |
//! | cursor | home, repository | `.cursor/mcp.json` → `mcpServers` | MCP server names |
//!
//! A plugin **cache** directory (`.codex/plugins/cache`, `.cursor/plugins/cache`)
//! is enumerated separately as a cache observation. A cached copy is not an
//! installed plugin and never becomes a configured component.
//!
//! An MCP server entry states a server, never that server's tools. This probe
//! therefore emits one [`ComponentKind::McpServer`] per configured server and
//! never synthesises a `mcp__server__tool` identity.

use serde::{Deserialize, Serialize, de::IgnoredAny};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    io::{self, ErrorKind, Read},
    path::{Component, Path, PathBuf},
};

#[path = "environment_io.rs"]
mod environment_io;
use environment_io::{DirHandle, Metadata};

/// Largest supported registry file. A larger file is reported unsupported
/// rather than read: this probe must never load an unbounded document.
pub const MAX_SOURCE_BYTES: u64 = 4 * 1024 * 1024;
/// Largest number of entries read from one source. A source stating more is
/// reported incomplete with the overflow counted, never silently truncated.
pub const MAX_SOURCE_ENTRIES: usize = 256;
/// Largest structural name returned. A longer name is skipped, not truncated.
pub const MAX_NAME_CHARS: usize = 128;

/// The hosts whose configuration this probe reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeHost {
    Claude,
    Codex,
    Cursor,
}

impl ProbeHost {
    /// The same host spelling stored sessions and metrics use.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Cursor => "cursor",
        }
    }
}

/// Which kind of supplied root a reading came from. The root itself is never
/// returned: a caller that supplied the roots already knows their paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootScope {
    Home,
    Repository,
}

/// One documented configuration source. The identifier names the source, not
/// the file, so the same source can be read under more than one scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSource {
    /// `<root>/.claude/skills`: one standalone skill per directory holding a
    /// regular `SKILL.md`.
    ClaudeSkillDirectory,
    /// `<home>/.claude.json` → `mcpServers`.
    ClaudeUserMcpConfig,
    /// `<repository>/.mcp.json` → `mcpServers`.
    ClaudeProjectMcpConfig,
    /// `<home>/.claude/plugins/installed_plugins.json`, schema 2 only.
    ClaudePluginRegistry,
    /// `<home>/.claude/settings.json` → `enabledPlugins`.
    ClaudeEnabledPlugins,
    /// `<home>/.claude/settings.json` → `hooks`.
    ClaudeHookConfig,
    /// `<home>/.codex/config.toml` → `[mcp_servers]`.
    CodexMcpConfig,
    /// `<root>/.cursor/mcp.json` → `mcpServers`.
    CursorMcpConfig,
}

/// A plugin cache directory. Its entries are observations of a cache, never
/// configured or installed components.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheSource {
    /// `<home>/.codex/plugins/cache`.
    CodexPluginCache,
    /// `<home>/.cursor/plugins/cache`.
    CursorPluginCache,
}

/// What a configured entry is. A component is a configuration fact; it makes
/// no claim about installation, availability or use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    /// One configured MCP server. It does not enumerate that server's tools.
    McpServer,
    Skill,
    Plugin,
    /// One configured hook entry, named by its event and matcher only.
    Hook,
}

/// Why a present source was deliberately not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnsupportedReason {
    /// The path is a symbolic link. This probe follows no link: a link can
    /// point outside the supplied root, so it is skipped and reported.
    Symlink,
    NotRegularFile,
    NotDirectory,
    /// Larger than [`MAX_SOURCE_BYTES`].
    FileTooLarge,
    /// A registry whose declared schema version this probe does not document.
    UnsupportedSchema,
}

/// Why a source that was read did not yield every entry it states.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncompleteReason {
    /// More than [`MAX_SOURCE_ENTRIES`] entries; the overflow was not read.
    EntryLimit,
    /// An entry whose documented fields do not prove what it claims, or whose
    /// structural name is unusable.
    UnprovenEntry,
}

/// What happened at one source under one root. `Missing` is a supported source
/// that is not there; `Empty` is one that is there and states nothing. Neither
/// is a measurement of zero installed components.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SourceStatus {
    /// The supported source does not exist under this root.
    Missing,
    /// Present, well formed, and stating no entry.
    Empty,
    /// Present and fully read. `stated` counts what the source stated, before
    /// components are deduplicated across roots.
    Read { stated: u32 },
    /// Present and read, but not every stated entry became a component.
    Incomplete {
        stated: u32,
        skipped: u32,
        reason: IncompleteReason,
    },
    /// Present, but its documented shape could not be parsed.
    Malformed,
    /// Present, but it could not be opened or read.
    Unreadable,
    /// Present, but deliberately not read.
    Unsupported { reason: UnsupportedReason },
}

/// One verified configured component. Every field is structural: no command,
/// argument, environment value, credential, URL or filesystem path appears.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ConfiguredComponent {
    pub host: ProbeHost,
    pub source: ConfigSource,
    /// The scope of the first root that stated this component.
    pub scope: RootScope,
    pub kind: ComponentKind,
    /// The name the source states. A hook is named `<event>:<matcher>`, with
    /// `*` standing for an unstated matcher.
    pub name: String,
    /// The documented enablement flag, when the source states one. `None` is
    /// unstated and is never read as `false`.
    pub enabled: Option<bool>,
}

/// One source under one supplied root.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceObservation {
    pub host: ProbeHost,
    pub source: ConfigSource,
    pub scope: RootScope,
    /// Position of the root inside the supplied list of that scope.
    pub root_index: u32,
    pub status: SourceStatus,
}

/// One plugin cache directory under one supplied root. A cache entry proves a
/// download, never an installed or configured component.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheObservation {
    pub host: ProbeHost,
    pub source: CacheSource,
    pub scope: RootScope,
    pub root_index: u32,
    pub status: SourceStatus,
}

/// Whether a supplied root could be read at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootState {
    /// An existing directory; its documented sources were read.
    Read,
    /// Nothing exists at this path; no source under it was opened.
    Missing,
    NotDirectory,
    /// A symbolic link. This probe follows no link.
    Symlink,
    /// The path exists but its metadata could not be read.
    Unreadable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootObservation {
    pub scope: RootScope,
    pub index: u32,
    pub state: RootState,
}

/// The whole reading. `components` is what was verified, `sources` says what
/// every documented source did, `cache` is separate by construction, and
/// `roots` says which supplied roots were usable at all.
///
/// Nothing here derives a missing component, a never-called component or a
/// used/installed ratio, because the sources read cannot support one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentProbe {
    pub roots: Vec<RootObservation>,
    pub components: Vec<ConfiguredComponent>,
    pub sources: Vec<SourceObservation>,
    pub cache: Vec<CacheObservation>,
}

/// Why a supplied root was refused before any file was opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RootRejected {
    #[error("a probe root must be an absolute local path")]
    NotAbsolute,
    #[error("a remote repository identifier is not a local root")]
    RemoteIdentifier,
    #[error("a probe root must contain no relative component")]
    Traversal,
}

/// The explicitly supplied roots. There is no default and no implicit home:
/// a caller that wants the user's home directory read must say so.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProbeRoots {
    homes: Vec<PathBuf>,
    repositories: Vec<PathBuf>,
}

impl ProbeRoots {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a home root. The path is taken as given; it is never resolved,
    /// canonicalised or walked upwards from.
    pub fn with_home(mut self, path: impl AsRef<Path>) -> Result<Self, RootRejected> {
        let path = local_root(path.as_ref())?;
        if !self.homes.contains(&path) {
            self.homes.push(path);
        }
        Ok(self)
    }

    /// Add a repository root from stored local session metadata (a session's
    /// `cwd`, or its `repo` when that is itself a local path). A remote
    /// identifier is refused rather than guessed at, no Git command is run,
    /// and no parent directory is walked to find an enclosing checkout.
    pub fn with_repository(mut self, stored: &str) -> Result<Self, RootRejected> {
        let trimmed = stored.trim();
        if trimmed.contains("://") || trimmed.starts_with("git@") || trimmed.starts_with("ssh://") {
            return Err(RootRejected::RemoteIdentifier);
        }
        let path = local_root(Path::new(trimmed))?;
        if !self.repositories.contains(&path) {
            self.repositories.push(path);
        }
        Ok(self)
    }

    pub fn homes(&self) -> &[PathBuf] {
        &self.homes
    }
    pub fn repositories(&self) -> &[PathBuf] {
        &self.repositories
    }
}

/// An absolute path made only of normal components, rebuilt from those
/// components so two spellings of one root are one root. A relative value
/// (including `owner/name`), an empty value and any `..` are refused before
/// anything is opened. Nothing is resolved against the filesystem here: a
/// link is still a link, and is refused when the root is read.
fn local_root(path: &Path) -> Result<PathBuf, RootRejected> {
    if path.as_os_str().is_empty() {
        return Err(RootRejected::NotAbsolute);
    }
    let mut components = path.components();
    let mut root = match components.next() {
        Some(component @ (Component::RootDir | Component::Prefix(_))) => {
            PathBuf::from(component.as_os_str())
        }
        _ => return Err(RootRejected::NotAbsolute),
    };
    for component in components {
        match component {
            Component::Normal(part) => root.push(part),
            Component::RootDir | Component::Prefix(_) | Component::CurDir => {}
            Component::ParentDir => return Err(RootRejected::Traversal),
        }
    }
    Ok(root)
}

/// Read every documented source under the supplied roots.
///
/// Home roots are read before repository roots, and each list in its supplied
/// order, so the first statement of a component wins deduplication
/// deterministically. This function launches nothing and opens no socket.
pub fn probe(roots: &ProbeRoots) -> EnvironmentProbe {
    let mut probe = Reading::default();
    for (index, root) in roots.homes().iter().enumerate() {
        probe.read_root(RootScope::Home, index, root);
    }
    for (index, root) in roots.repositories().iter().enumerate() {
        probe.read_root(RootScope::Repository, index, root);
    }
    probe.finish()
}

#[derive(Default)]
struct Reading {
    roots: Vec<RootObservation>,
    /// Deduplicated by host, source and structural name, as the first root to
    /// state a component states it.
    components: BTreeMap<(ProbeHost, ConfigSource, String), ConfiguredComponent>,
    sources: Vec<SourceObservation>,
    cache: Vec<CacheObservation>,
}

/// What one source produced: the entries it stated, and whether reading them
/// was complete.
struct Stated {
    entries: Vec<(String, Option<bool>)>,
    status: SourceStatus,
}

impl Stated {
    fn status(status: SourceStatus) -> Self {
        Self {
            entries: Vec::new(),
            status,
        }
    }

    /// Pair the accepted entries with the count the source stated. `skipped`
    /// is everything stated that could not be proven or named.
    fn counted(entries: Vec<(String, Option<bool>)>, stated: usize, limited: bool) -> Self {
        let status = count_status(entries.len(), stated, limited);
        Self { entries, status }
    }
}

/// The one rule every counted source follows. A source over its entry bound
/// is `incomplete` whatever was accepted, so a bound is never read as a
/// complete answer.
fn count_status(accepted: usize, stated: usize, limited: bool) -> SourceStatus {
    if limited {
        SourceStatus::Incomplete {
            stated: clamp(stated),
            skipped: clamp(stated.saturating_sub(accepted)),
            reason: IncompleteReason::EntryLimit,
        }
    } else if accepted < stated {
        SourceStatus::Incomplete {
            stated: clamp(stated),
            skipped: clamp(stated - accepted),
            reason: IncompleteReason::UnprovenEntry,
        }
    } else if accepted == 0 {
        SourceStatus::Empty
    } else {
        SourceStatus::Read {
            stated: clamp(stated),
        }
    }
}

fn clamp(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

impl Reading {
    fn read_root(&mut self, scope: RootScope, index: usize, root: &Path) {
        let index = clamp(index);
        let opened = DirHandle::open_root(root);
        let state = match &opened {
            Ok(_) => RootState::Read,
            Err(error) => match io_status(error) {
                SourceStatus::Missing => RootState::Missing,
                SourceStatus::Unsupported {
                    reason: UnsupportedReason::Symlink,
                } => RootState::Symlink,
                SourceStatus::Unsupported {
                    reason: UnsupportedReason::NotDirectory,
                } => RootState::NotDirectory,
                _ => RootState::Unreadable,
            },
        };
        self.roots.push(RootObservation {
            scope,
            index,
            state,
        });
        let Ok(root) = opened else {
            return;
        };
        let root = &root;
        self.claude_skills(scope, index, Known::new(root, ".claude/skills"));
        match scope {
            RootScope::Home => {
                self.mcp_json(
                    ProbeHost::Claude,
                    ConfigSource::ClaudeUserMcpConfig,
                    scope,
                    index,
                    Known::new(root, ".claude.json"),
                );
                self.claude_plugin_registry(
                    scope,
                    index,
                    Known::new(root, ".claude/plugins/installed_plugins.json"),
                );
                self.claude_settings(scope, index, Known::new(root, ".claude/settings.json"));
                self.codex_mcp(scope, index, Known::new(root, ".codex/config.toml"));
                self.mcp_json(
                    ProbeHost::Cursor,
                    ConfigSource::CursorMcpConfig,
                    scope,
                    index,
                    Known::new(root, ".cursor/mcp.json"),
                );
                self.plugin_cache(
                    ProbeHost::Codex,
                    CacheSource::CodexPluginCache,
                    scope,
                    index,
                    Known::new(root, ".codex/plugins/cache"),
                );
                self.plugin_cache(
                    ProbeHost::Cursor,
                    CacheSource::CursorPluginCache,
                    scope,
                    index,
                    Known::new(root, ".cursor/plugins/cache"),
                );
            }
            RootScope::Repository => {
                self.mcp_json(
                    ProbeHost::Claude,
                    ConfigSource::ClaudeProjectMcpConfig,
                    scope,
                    index,
                    Known::new(root, ".mcp.json"),
                );
                self.mcp_json(
                    ProbeHost::Cursor,
                    ConfigSource::CursorMcpConfig,
                    scope,
                    index,
                    Known::new(root, ".cursor/mcp.json"),
                );
            }
        }
    }

    /// Record one source: its status always, and its entries as components.
    fn record(
        &mut self,
        host: ProbeHost,
        source: ConfigSource,
        scope: RootScope,
        root_index: u32,
        kind: ComponentKind,
        stated: Stated,
    ) {
        self.sources.push(SourceObservation {
            host,
            source,
            scope,
            root_index,
            status: stated.status,
        });
        for (name, enabled) in stated.entries {
            self.components
                .entry((host, source, name.clone()))
                .or_insert(ConfiguredComponent {
                    host,
                    source,
                    scope,
                    kind,
                    name,
                    enabled,
                });
        }
    }

    /// A standalone skill is a real (unlinked) directory entry holding a
    /// regular, unlinked `SKILL.md`. Anything else in the directory is stated
    /// and skipped.
    fn claude_skills(&mut self, scope: RootScope, root_index: u32, known: Known<'_>) {
        let stated = match list_directory(known) {
            Err(status) => Stated::status(status),
            Ok(listing) => stated_skills(listing),
        };
        self.record(
            ProbeHost::Claude,
            ConfigSource::ClaudeSkillDirectory,
            scope,
            root_index,
            ComponentKind::Skill,
            stated,
        );
    }

    /// `mcpServers` names whose value is an object. Only the shape of a
    /// server's value is inspected: its command, arguments, environment, URL
    /// and headers are never kept, so they cannot be returned.
    fn mcp_json(
        &mut self,
        host: ProbeHost,
        source: ConfigSource,
        scope: RootScope,
        root_index: u32,
        known: Known<'_>,
    ) {
        #[derive(Deserialize)]
        struct McpConfig {
            #[serde(rename = "mcpServers")]
            mcp_servers: Option<BTreeMap<String, Shape>>,
        }
        let stated = match bounded_text(known) {
            Err(status) => Stated::status(status),
            Ok(text) => match serde_json::from_str::<McpConfig>(&text) {
                Err(_) => Stated::status(SourceStatus::Malformed),
                Ok(config) => servers(config.mcp_servers.unwrap_or_default()),
            },
        };
        self.record(
            host,
            source,
            scope,
            root_index,
            ComponentKind::McpServer,
            stated,
        );
    }

    /// `[mcp_servers]` names whose value is a table, through the workspace's
    /// locked TOML parser. `env` and `http_headers` are never kept.
    fn codex_mcp(&mut self, scope: RootScope, root_index: u32, known: Known<'_>) {
        #[derive(Deserialize)]
        struct CodexConfig {
            mcp_servers: Option<BTreeMap<String, Shape>>,
        }
        let stated = match bounded_text(known) {
            Err(status) => Stated::status(status),
            Ok(text) => match toml::from_str::<CodexConfig>(&text) {
                Err(_) => Stated::status(SourceStatus::Malformed),
                Ok(config) => servers(config.mcp_servers.unwrap_or_default()),
            },
        };
        self.record(
            ProbeHost::Codex,
            ConfigSource::CodexMcpConfig,
            scope,
            root_index,
            ComponentKind::McpServer,
            stated,
        );
    }

    /// Schema 2 only. A plugin counts as installed when one of its entries is
    /// an object whose `scope` is a documented scope, whose `installPath` is a
    /// nonempty string and whose `installedAt` is an RFC 3339 timestamp. The
    /// install path itself is never returned.
    fn claude_plugin_registry(&mut self, scope: RootScope, root_index: u32, known: Known<'_>) {
        #[derive(Deserialize)]
        struct Schema {
            version: Option<u32>,
        }
        #[derive(Deserialize)]
        struct Registry {
            plugins: Option<BTreeMap<String, Field>>,
        }
        let stated = match bounded_text(known) {
            Err(status) => Stated::status(status),
            Ok(text) => match serde_json::from_str::<Schema>(&text) {
                Err(_) => Stated::status(SourceStatus::Malformed),
                Ok(schema) if schema.version != Some(2) => {
                    Stated::status(SourceStatus::Unsupported {
                        reason: UnsupportedReason::UnsupportedSchema,
                    })
                }
                Ok(_) => match serde_json::from_str::<Registry>(&text) {
                    Err(_) => Stated::status(SourceStatus::Malformed),
                    Ok(registry) => {
                        let plugins = registry.plugins.unwrap_or_default();
                        let stated = plugins.len();
                        let accepted = plugins
                            .into_iter()
                            .take(MAX_SOURCE_ENTRIES)
                            .filter(|(_, entries)| match entries {
                                Field::List(entries) => entries.iter().any(proves_installation),
                                _ => false,
                            })
                            .filter_map(|(name, _)| structural_name(&name).map(|name| (name, None)))
                            .collect();
                        Stated::counted(accepted, stated, stated > MAX_SOURCE_ENTRIES)
                    }
                },
            },
        };
        self.record(
            ProbeHost::Claude,
            ConfigSource::ClaudePluginRegistry,
            scope,
            root_index,
            ComponentKind::Plugin,
            stated,
        );
    }

    /// One file, two sources: the plugin enablement map and the configured
    /// hook entries. A hook's command body is never kept.
    fn claude_settings(&mut self, scope: RootScope, root_index: u32, known: Known<'_>) {
        #[derive(Deserialize)]
        struct Settings {
            #[serde(rename = "enabledPlugins")]
            enabled_plugins: Option<BTreeMap<String, Flag>>,
            hooks: Option<BTreeMap<String, Field>>,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Flag {
            Stated(bool),
            Unstated(IgnoredAny),
        }
        let (enabled, hooks) = match bounded_text(known) {
            Err(status) => (Stated::status(status), Stated::status(status)),
            Ok(text) => match serde_json::from_str::<Settings>(&text) {
                Err(_) => (
                    Stated::status(SourceStatus::Malformed),
                    Stated::status(SourceStatus::Malformed),
                ),
                Ok(settings) => {
                    let plugins = settings.enabled_plugins.unwrap_or_default();
                    let stated = plugins.len();
                    let accepted = plugins
                        .into_iter()
                        .take(MAX_SOURCE_ENTRIES)
                        .filter_map(|(name, flag)| match flag {
                            Flag::Stated(value) => {
                                structural_name(&name).map(|name| (name, Some(value)))
                            }
                            Flag::Unstated(_) => None,
                        })
                        .collect();
                    let enabled = Stated::counted(accepted, stated, stated > MAX_SOURCE_ENTRIES);
                    (enabled, hook_entries(settings.hooks.unwrap_or_default()))
                }
            },
        };
        self.record(
            ProbeHost::Claude,
            ConfigSource::ClaudeEnabledPlugins,
            scope,
            root_index,
            ComponentKind::Plugin,
            enabled,
        );
        self.record(
            ProbeHost::Claude,
            ConfigSource::ClaudeHookConfig,
            scope,
            root_index,
            ComponentKind::Hook,
            hooks,
        );
    }

    /// A cache directory is counted only: no entry name becomes a component.
    /// A linked or non-UTF-8 entry is stated and skipped.
    fn plugin_cache(
        &mut self,
        host: ProbeHost,
        source: CacheSource,
        scope: RootScope,
        root_index: u32,
        known: Known<'_>,
    ) {
        let status = match list_directory(known) {
            Err(status) => status,
            Ok(listing) => count_status(listing.entries.len(), listing.stated, listing.limited),
        };
        self.cache.push(CacheObservation {
            host,
            source,
            scope,
            root_index,
            status,
        });
    }

    fn finish(self) -> EnvironmentProbe {
        let mut components: Vec<_> = self.components.into_values().collect();
        components.sort_by(|left, right| {
            (left.host, left.source, left.kind, &left.name).cmp(&(
                right.host,
                right.source,
                right.kind,
                &right.name,
            ))
        });
        EnvironmentProbe {
            roots: self.roots,
            components,
            sources: self.sources,
            cache: self.cache,
        }
    }
}

/// The shape of a value, and nothing else: an object's members are skipped
/// without being kept.
#[derive(Deserialize)]
#[serde(untagged)]
enum Shape {
    // Matched for its shape only; the members are skipped, never read.
    Object(#[allow(dead_code)] BTreeMap<String, IgnoredAny>),
    Other(IgnoredAny),
}

/// One member of a documented object, kept only as far as a proof needs it.
#[derive(Deserialize)]
#[serde(untagged)]
enum Field {
    Null(()),
    Text(String),
    List(Vec<Member>),
    Other(IgnoredAny),
}

/// A list element: an object whose members are fields, or anything else.
#[derive(Deserialize)]
#[serde(untagged)]
enum Member {
    Object(BTreeMap<String, Field>),
    Other(IgnoredAny),
}

/// Server names whose configuration value is an object. A scalar, `null` or
/// list is a stated server this probe cannot prove, so it is skipped.
fn servers(servers: BTreeMap<String, Shape>) -> Stated {
    let stated = servers.len();
    let accepted = servers
        .into_iter()
        .take(MAX_SOURCE_ENTRIES)
        .filter(|(_, value)| matches!(value, Shape::Object(_)))
        .filter_map(|(name, _)| structural_name(&name).map(|name| (name, None)))
        .collect();
    Stated::counted(accepted, stated, stated > MAX_SOURCE_ENTRIES)
}

/// Configured hook entries. Each matcher group of each event is one stated
/// entry. It becomes a component only when the event is a usable name, the
/// group is an object, its matcher is absent, `null` or a nonempty string, and
/// its `hooks` is a nonempty list whose every element is an object. An event
/// whose value is not a list states one entry, which is skipped.
fn hook_entries(events: BTreeMap<String, Field>) -> Stated {
    let mut stated = 0_usize;
    let mut accepted = Vec::new();
    for (event, groups) in events {
        let groups = match groups {
            Field::List(groups) => groups,
            _ => {
                stated += 1;
                continue;
            }
        };
        for group in groups {
            stated += 1;
            if accepted.len() >= MAX_SOURCE_ENTRIES {
                continue;
            }
            let Member::Object(group) = group else {
                continue;
            };
            let proven_hooks = matches!(
                group.get("hooks"),
                Some(Field::List(hooks))
                    if !hooks.is_empty()
                        && hooks.iter().all(|hook| matches!(hook, Member::Object(_)))
            );
            let matcher = match group.get("matcher") {
                None | Some(Field::Null(())) => Some("*"),
                Some(Field::Text(matcher)) => Some(matcher.trim()).filter(|m| !m.is_empty()),
                Some(_) => None,
            };
            let (Some(matcher), Some(event), true) =
                (matcher, structural_name(&event), proven_hooks)
            else {
                continue;
            };
            if let Some(name) = structural_name(&format!("{event}:{matcher}")) {
                accepted.push((name, None));
            }
        }
    }
    let limited = stated > MAX_SOURCE_ENTRIES;
    Stated::counted(accepted, stated, limited)
}

/// Scopes the schema-2 registry documents for an installation.
const PLUGIN_SCOPES: [&str; 3] = ["user", "project", "local"];

fn proves_installation(entry: &Member) -> bool {
    let Member::Object(entry) = entry else {
        return false;
    };
    let text = |key: &str| match entry.get(key) {
        Some(Field::Text(value)) => Some(value.trim()).filter(|value| !value.is_empty()),
        _ => None,
    };
    text("scope").is_some_and(|scope| PLUGIN_SCOPES.contains(&scope))
        && text("installPath").is_some_and(|path| !path.chars().any(char::is_control))
        && text("installedAt").is_some_and(rfc3339_timestamp)
}

/// An RFC 3339 date-time with an offset, as the workspace's locked `chrono`
/// parser accepts it. An impossible date or time (February 30, month 13,
/// hour 25, minute 60) is rejected by the parser, not by a shape check.
fn rfc3339_timestamp(value: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(value).is_ok()
}

/// A usable structural name: nonempty, bounded, and free of control
/// characters. Anything else is skipped rather than truncated or escaped.
fn structural_name(raw: &str) -> Option<String> {
    let name = raw.trim();
    if name.is_empty()
        || name.chars().count() > MAX_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return None;
    }
    Some(name.to_owned())
}

/// Map only filesystem outcomes; no path or OS error text leaves the probe.
fn io_status(error: &io::Error) -> SourceStatus {
    #[cfg(unix)]
    match error.raw_os_error() {
        Some(libc::ELOOP) => {
            return SourceStatus::Unsupported {
                reason: UnsupportedReason::Symlink,
            };
        }
        Some(libc::ENOTDIR) => {
            return SourceStatus::Unsupported {
                reason: UnsupportedReason::NotDirectory,
            };
        }
        Some(libc::ENXIO) => {
            return SourceStatus::Unsupported {
                reason: UnsupportedReason::NotRegularFile,
            };
        }
        _ => {}
    }
    if error.kind() == ErrorKind::NotFound {
        SourceStatus::Missing
    } else {
        SourceStatus::Unreadable
    }
}

/// A documented path relative to the already opened supplied root.
#[derive(Clone, Copy)]
struct Known<'a> {
    root: &'a DirHandle,
    relative: &'static str,
}

impl<'a> Known<'a> {
    fn new(root: &'a DirHandle, relative: &'static str) -> Self {
        Self { root, relative }
    }

    fn locate(self) -> Result<Located, SourceStatus> {
        let mut parts = self.relative.split('/').peekable();
        let mut parent = None;
        while let Some(part) = parts.next() {
            let directory = parent.as_ref().unwrap_or(self.root);
            let name = OsString::from(part);
            let metadata = directory
                .metadata(&name)
                .map_err(|error| io_status(&error))?;
            if metadata.is_symlink() {
                return Err(SourceStatus::Unsupported {
                    reason: UnsupportedReason::Symlink,
                });
            }
            if parts.peek().is_none() {
                // Even a source directly under the root owns its parent handle.
                let parent = match parent {
                    Some(parent) => parent,
                    None => self
                        .root
                        .clone_handle()
                        .map_err(|error| io_status(&error))?,
                };
                return Ok(Located {
                    parent,
                    name,
                    metadata,
                });
            }
            parent = Some(
                directory
                    .open_directory(&name, metadata)
                    .map_err(|error| io_status(&error))?,
            );
        }
        Err(SourceStatus::Missing)
    }
}

/// The final name and identity are always paired with their pinned parent.
struct Located {
    parent: DirHandle,
    name: OsString,
    metadata: Metadata,
}

impl Located {
    fn confirm(&self) -> Result<(), SourceStatus> {
        self.parent
            .confirm(&self.name, self.metadata)
            .map_err(|error| io_status(&error))
    }
}

/// Only a checked regular descriptor reaches the bounded reader.
fn bounded_text(known: Known<'_>) -> Result<String, SourceStatus> {
    let located = known.locate()?;
    if !located.metadata.is_file() {
        return Err(SourceStatus::Unsupported {
            reason: UnsupportedReason::NotRegularFile,
        });
    }
    if located.metadata.len() > MAX_SOURCE_BYTES {
        return Err(SourceStatus::Unsupported {
            reason: UnsupportedReason::FileTooLarge,
        });
    }
    let bytes = read_located(&located)?;
    located.confirm()?;
    String::from_utf8(bytes).map_err(|_| SourceStatus::Malformed)
}

fn read_located(located: &Located) -> Result<Vec<u8>, SourceStatus> {
    let file = located
        .parent
        .open_file(&located.name, located.metadata)
        .map_err(|error| io_status(&error))?;
    read_bounded(file)
}

/// Consume at most one byte past the bound, including a growing source.
fn read_bounded(reader: impl Read) -> Result<Vec<u8>, SourceStatus> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_SOURCE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| SourceStatus::Unreadable)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SOURCE_BYTES {
        return Err(SourceStatus::Unsupported {
            reason: UnsupportedReason::FileTooLarge,
        });
    }
    Ok(bytes)
}

/// One bounded level, sorted by name. Saved identities are rechecked when a
/// skill child is opened; a saved type alone cannot establish a skill.
struct Listing {
    directory: DirHandle,
    entries: Vec<(String, Metadata)>,
    stated: usize,
    limited: bool,
}

fn list_directory(known: Known<'_>) -> Result<Listing, SourceStatus> {
    list_located(&known.locate()?)
}

fn list_located(located: &Located) -> Result<Listing, SourceStatus> {
    let directory = located
        .parent
        .open_directory(&located.name, located.metadata)
        .map_err(|error| io_status(&error))?;
    let names = directory
        .names(MAX_SOURCE_ENTRIES + 1)
        .map_err(|error| io_status(&error))?;
    let stated = names.len();
    let limited = stated > MAX_SOURCE_ENTRIES;
    let mut entries = if limited {
        Vec::new()
    } else {
        listed_entries(&directory, names)?
    };
    located.confirm()?;
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(Listing {
        directory,
        entries,
        stated,
        limited,
    })
}

fn listed_entries(
    directory: &DirHandle,
    names: Vec<OsString>,
) -> Result<Vec<(String, Metadata)>, SourceStatus> {
    let mut entries = Vec::new();
    for name in names {
        let metadata = match directory.metadata(&name) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(_) => return Err(SourceStatus::Unreadable),
        };
        if !metadata.is_symlink()
            && let Some(name) = name.to_str()
        {
            entries.push((name.to_owned(), metadata));
        }
    }
    Ok(entries)
}

fn stated_skills(listing: Listing) -> Stated {
    let accepted = listing
        .entries
        .into_iter()
        .filter_map(|(name, metadata)| {
            // Opening through the listing handle refuses a replaced child and
            // pins the original child before SKILL.md is inspected.
            let child = listing
                .directory
                .open_directory(std::ffi::OsStr::new(&name), metadata)
                .ok()?;
            let skill = child.metadata(std::ffi::OsStr::new("SKILL.md")).ok()?;
            if !skill.is_file() {
                return None;
            }
            child
                .confirm(std::ffi::OsStr::new("SKILL.md"), skill)
                .ok()?;
            listing
                .directory
                .confirm(std::ffi::OsStr::new(&name), metadata)
                .ok()?;
            structural_name(&name).map(|name| (name, None))
        })
        .collect();
    Stated::counted(accepted, listing.stated, listing.limited)
}

#[cfg(test)]
#[path = "environment_tests.rs"]
mod tests;
