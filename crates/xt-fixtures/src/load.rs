use crate::{Error, FixtureId, Result, RuleId, TempDb, invalid};
use chrono::{DateTime, Duration, FixedOffset};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, DeserializeOwned, MapAccess, Visitor},
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};
use xt_store::{CanonicalRecord, Host, SessionMeta, SessionSource};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureStatus {
    Populated,
    Skeleton,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionInput {
    pub file: PathBuf,
    pub session_id: String,
    pub host: Host,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_surface: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: FixtureId,
    pub status: FixtureStatus,
    pub author: String,
    pub shape: String,
    /// Planned coverage for skeletons; asserted golden keys for populated inputs.
    pub proves: Vec<RuleId>,
    pub now: String,
    pub window_days: u32,
    pub sessions: Vec<SessionInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gh: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub snapshots: BTreeMap<String, PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadedSession {
    pub metadata: SessionMeta,
    pub records: Vec<CanonicalRecord>,
}

/// Canonical fixture transport. IPC DTO generation is owned by the later IPC
/// boundary; consumers use these same canonical records rather than reloading files.
#[derive(Debug, Serialize)]
pub struct FixtureExport<'a> {
    pub manifest: &'a Manifest,
    pub sessions: &'a [LoadedSession],
    pub gh: &'a Option<Value>,
    pub snapshots: &'a BTreeMap<String, Value>,
    pub expected: &'a BTreeMap<RuleId, Value>,
}

pub struct Fixture {
    manifest: Manifest,
    sessions: Vec<LoadedSession>,
    gh: Option<Value>,
    snapshots: BTreeMap<String, Value>,
    expected: BTreeMap<RuleId, Value>,
    now: DateTime<FixedOffset>,
    window_start: DateTime<FixedOffset>,
}

impl Fixture {
    /// Load only manifest-declared files. Paths must remain inside this fixture.
    pub fn load(directory: impl AsRef<Path>) -> Result<Self> {
        let root = directory
            .as_ref()
            .canonicalize()
            .map_err(|e| invalid(directory.as_ref().display().to_string(), e.to_string()))?;
        let manifest: Manifest = read_json(&declared_path(&root, Path::new("manifest.json"))?)?;
        let location = root.join("manifest.json").display().to_string();
        if manifest.author.trim().is_empty() || manifest.shape.trim().is_empty() {
            return Err(invalid(&location, "author and shape must be nonempty"));
        }
        if manifest.proves.is_empty()
            || manifest.proves.iter().collect::<BTreeSet<_>>().len() != manifest.proves.len()
        {
            return Err(invalid(
                format!("{location}:proves"),
                "expected unique rule keys",
            ));
        }
        let now = parse_time(&manifest.now, &format!("{location}:now"))?;
        let window_start = now
            .checked_sub_signed(Duration::days(i64::from(manifest.window_days)))
            .filter(|_| manifest.window_days > 0)
            .ok_or_else(|| {
                invalid(
                    format!("{location}:window_days"),
                    "expected a positive, representable window",
                )
            })?;
        let Expected(expected) = read_json(&declared_path(&root, Path::new("expected.json"))?)?;
        let expected_location = root.join("expected.json").display().to_string();
        if manifest.status == FixtureStatus::Skeleton && !expected.is_empty() {
            return Err(invalid(
                &expected_location,
                "skeleton expectations must be empty; null means unmeasured, not unimplemented",
            ));
        }
        if manifest.status == FixtureStatus::Populated
            && expected.keys().collect::<BTreeSet<_>>() != manifest.proves.iter().collect()
        {
            return Err(invalid(
                &expected_location,
                "populated expectations must match manifest.proves exactly",
            ));
        }
        if manifest.sessions.is_empty() {
            return Err(invalid(
                format!("{location}:sessions"),
                "declare at least one canonical input, even for an empty skeleton",
            ));
        }
        let mut sessions = Vec::new();
        let mut session_ids = BTreeSet::new();
        for (index, input) in manifest.sessions.iter().enumerate() {
            let loc = format!("{location}:sessions[{index}]");
            if input.session_id.trim().is_empty() || !session_ids.insert(&input.session_id) {
                return Err(invalid(
                    &loc,
                    "session_id must be nonempty and unique within a manifest",
                ));
            }
            if let Some(platform) = &input.source_platform
                && (platform.trim().is_empty() || Host::from_platform(platform) != input.host)
            {
                return Err(invalid(
                    &loc,
                    "source_platform must map to the declared host",
                ));
            }
            let metadata = SessionMeta {
                session_id: input.session_id.clone(),
                host: input.host,
                source_platform: input.source_platform.clone(),
                source: SessionSource::Fixture,
                cwd: None,
                git_branch: None,
                title: None,
                surface: input.source_surface.clone(),
                surface_evidence: None,
                native_session_id: input.native_session_id.clone(),
                started_at_ms: input
                    .started_at
                    .as_ref()
                    .map(|value| {
                        parse_time(value, &format!("{loc}:started_at"))
                            .map(|ts| ts.timestamp_millis())
                    })
                    .transpose()?,
            };
            let path = declared_path(&root, &input.file)?;
            let text = fs::read_to_string(&path)
                .map_err(|e| invalid(path.display().to_string(), e.to_string()))?;
            let mut records = Vec::new();
            for (line, text) in text.lines().enumerate() {
                if text.trim().is_empty() {
                    continue;
                }
                let loc = format!("{}:{}", path.display(), line + 1);
                let record: CanonicalRecord = serde_json::from_str(text)
                    .map_err(|e| invalid(&loc, format!("invalid canonical JSON: {e}")))?;
                let uuid = record.uuid.as_deref().ok_or_else(|| {
                    invalid(format!("{loc}:uuid"), "UUID is required in fixture input")
                })?;
                uuid::Uuid::parse_str(uuid)
                    .map_err(|_| invalid(format!("{loc}:uuid"), "invalid UUID"))?;
                if let Some(ts) = &record.timestamp {
                    parse_time(ts, &format!("{loc}:timestamp"))?;
                }
                records.push(record);
            }
            sessions.push(LoadedSession { metadata, records });
        }
        if manifest.status == FixtureStatus::Skeleton
            && sessions.iter().any(|session| !session.records.is_empty())
        {
            return Err(invalid(
                &location,
                "skeleton canonical inputs must be empty",
            ));
        }
        if manifest.status == FixtureStatus::Populated
            && sessions.iter().all(|session| session.records.is_empty())
        {
            return Err(invalid(
                &location,
                "populated fixture has no canonical records",
            ));
        }
        let gh = manifest
            .gh
            .as_ref()
            .map(|path| read_json(&declared_path(&root, path)?))
            .transpose()?;
        let snapshots = manifest
            .snapshots
            .iter()
            .map(|(name, path)| {
                if name.trim().is_empty() {
                    return Err(invalid(&location, "snapshot name must be nonempty"));
                }
                Ok((name.clone(), read_json(&declared_path(&root, path)?)?))
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            manifest,
            sessions,
            gh,
            snapshots,
            expected,
            now,
            window_start,
        })
    }

    /// Load the complete registry in numeric order and reject accidental F01/F22
    /// directories. Other documentation files alongside the catalog are allowed.
    pub fn all(catalog: impl AsRef<Path>) -> Result<Vec<Self>> {
        let root = catalog.as_ref();
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type()?.is_dir() && name.starts_with('F') {
                FixtureId::parse(&name)
                    .map_err(|e| invalid(entry.path().display().to_string(), e))?;
            }
        }
        FixtureId::all()
            .map(|id| {
                let directory = root.join(id.to_string());
                let fixture = Self::load(&directory)?;
                if fixture.manifest.id != id {
                    return Err(invalid(
                        directory.display().to_string(),
                        "manifest ID does not match catalog directory",
                    ));
                }
                Ok(fixture)
            })
            .collect()
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn sessions(&self) -> &[LoadedSession] {
        &self.sessions
    }
    pub fn gh(&self) -> Option<&Value> {
        self.gh.as_ref()
    }
    pub fn snapshots(&self) -> &BTreeMap<String, Value> {
        &self.snapshots
    }
    pub fn expected(&self) -> &BTreeMap<RuleId, Value> {
        &self.expected
    }
    pub fn now(&self) -> DateTime<FixedOffset> {
        self.now
    }
    pub fn window_start(&self) -> DateTime<FixedOffset> {
        self.window_start
    }

    pub fn export(&self) -> FixtureExport<'_> {
        FixtureExport {
            manifest: &self.manifest,
            sessions: &self.sessions,
            gh: &self.gh,
            snapshots: &self.snapshots,
            expected: &self.expected,
        }
    }

    /// Exact golden comparison; JSON null remains distinct from numeric zero.
    /// A caller must supply observed behavior, not echo this fixture's expected value.
    pub fn assert_expectation(&self, rule: &str, actual: &Value) -> Result<()> {
        self.require_populated()?;
        let key = RuleId::parse(rule).map_err(|e| invalid(rule, e))?;
        let expected = self.expected.get(&key).ok_or_else(|| {
            invalid(
                format!("{}:expected:{rule}", self.manifest.id),
                "rule expectation is not implemented",
            )
        })?;
        if expected != actual {
            return Err(invalid(
                format!("{}:expected:{rule}", self.manifest.id),
                "observed value does not match golden expectation",
            ));
        }
        Ok(())
    }

    /// Execute the currently implemented F1 reference assertions against stored
    /// rows. This is fixture arithmetic, not the application's metric engine.
    pub fn assert_reference(&self) -> Result<()> {
        self.require_populated()?;
        if self.manifest.id.to_string() != "F1" {
            return Err(invalid(
                self.manifest.id.to_string(),
                "reference assertions are not implemented for this fixture",
            ));
        }
        let db = self.build_db(true)?;
        crate::f1::assert_rows(self, db.store())
    }

    pub fn build_db(&self, keep_content: bool) -> Result<TempDb> {
        self.require_populated()?;
        TempDb::build(&self.sessions, keep_content)
    }

    /// Materialize a closed file database without overwriting an existing path.
    /// The internal temporary owner is never exposed to another reader.
    pub fn write_db(&self, path: impl AsRef<Path>, keep_content: bool) -> Result<()> {
        self.build_db(keep_content)?.write_to(path.as_ref())
    }

    pub(crate) fn require_populated(&self) -> Result<()> {
        if self.manifest.status == FixtureStatus::Skeleton {
            Err(Error::Unimplemented(self.manifest.id))
        } else {
            Ok(())
        }
    }
}

// A normal map deserializer silently replaces repeated keys. Goldens must reject
// even identical duplicates while the input entries are still observable.
struct Expected(BTreeMap<RuleId, Value>);

impl<'de> Deserialize<'de> for Expected {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct ExpectedVisitor;
        impl<'de> Visitor<'de> for ExpectedVisitor {
            type Value = Expected;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an object of unique golden rule keys")
            }

            fn visit_map<M: MapAccess<'de>>(
                self,
                mut entries: M,
            ) -> std::result::Result<Self::Value, M::Error> {
                let mut values = BTreeMap::new();
                while let Some(key) = entries.next_key::<RuleId>()? {
                    if values.contains_key(&key) {
                        return Err(de::Error::custom(format!("duplicate golden key {key}")));
                    }
                    values.insert(key, entries.next_value()?);
                }
                Ok(Expected(values))
            }
        }
        deserializer.deserialize_map(ExpectedVisitor)
    }
}

fn parse_time(value: &str, location: &str) -> Result<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value).map_err(|_| invalid(location, "invalid RFC3339 timestamp"))
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let text =
        fs::read_to_string(path).map_err(|e| invalid(path.display().to_string(), e.to_string()))?;
    serde_json::from_str(&text).map_err(|e| {
        invalid(
            format!("{}:{}:{}", path.display(), e.line(), e.column()),
            e.to_string(),
        )
    })
}

fn declared_path(root: &Path, relative: &Path) -> Result<PathBuf> {
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(invalid(
            relative.display().to_string(),
            "input path must be a relative path without traversal",
        ));
    }
    let path = root.join(relative);
    let resolved = path
        .canonicalize()
        .map_err(|e| invalid(path.display().to_string(), e.to_string()))?;
    if !resolved.starts_with(root) || !resolved.is_file() {
        return Err(invalid(
            path.display().to_string(),
            "input must be a file inside the fixture directory",
        ));
    }
    Ok(resolved)
}
