//! Wire inputs and structural session metadata. Optional fields remain unknown
//! until measured; absent usage counters are never converted to zero.

use serde::{Deserialize, Serialize};
use serde_json::Value;

macro_rules! text_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name {
            $(#[serde(rename = $value)] $variant),+
        }
        impl $name {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $value),+ }
            }
        }
        impl rusqlite::types::ToSql for $name {
            fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
                Ok(self.as_str().into())
            }
        }
        impl rusqlite::types::FromSql for $name {
            fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
                match value.as_str()? {
                    $($value => Ok(Self::$variant)),+,
                    _ => Err(rusqlite::types::FromSqlError::InvalidType),
                }
            }
        }
    };
}

pub(crate) use text_enum;

text_enum!(Host { Claude => "claude", Codex => "codex", Cursor => "cursor", Other => "other" });
text_enum!(SessionSource { Plugin => "plugin", Transcript => "transcript", ReadersCli => "readers_cli", Fixture => "fixture" });
text_enum!(RecordType { User => "user", Assistant => "assistant" });

impl Host {
    pub fn from_platform(platform: &str) -> Self {
        match platform {
            "claude" => Self::Claude,
            "codex" => Self::Codex,
            "cursor" => Self::Cursor,
            _ => Self::Other,
        }
    }
}

/// Structural adapter identity, never a transcript excerpt, path or raw payload.
/// Labels accept ASCII letters/digits and `._-`; unknown label values survive.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceEvidence {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMeta {
    pub session_id: String,
    pub host: Host,
    /// Raw platform is retained separately from the closed, mapped host enum.
    pub source_platform: Option<String>,
    pub source: SessionSource,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub title: Option<String>,
    pub surface: Option<String>,
    pub surface_evidence: Option<SurfaceEvidence>,
    pub native_session_id: Option<String>,
    /// Native session start, independent of the first imported event timestamp.
    pub started_at_ms: Option<i64>,
}

impl SessionMeta {
    pub fn new(
        session_id: impl Into<String>,
        source_platform: impl Into<String>,
        source: SessionSource,
    ) -> Self {
        let source_platform = source_platform.into();
        Self {
            session_id: session_id.into(),
            host: Host::from_platform(&source_platform),
            source_platform: Some(source_platform),
            source,
            cwd: None,
            git_branch: None,
            title: None,
            surface: None,
            surface_evidence: None,
            native_session_id: None,
            started_at_ms: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalRecord {
    /// Missing or blank UUIDs are counted and omitted by `upsert_records`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    #[serde(rename = "type")]
    pub record_type: RecordType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(
        default,
        rename = "gitBranch",
        alias = "git_branch",
        skip_serializing_if = "Option::is_none"
    )]
    pub git_branch: Option<String>,
    #[serde(default, rename = "isMeta")]
    pub is_meta: bool,
    #[serde(default, rename = "isSidechain")]
    pub is_sidechain: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_message_id: Option<String>,
    #[serde(
        default,
        rename = "requestId",
        alias = "request_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_surface: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_evidence: Option<SurfaceEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_session_id: Option<String>,
    pub message: CanonicalMessage,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalMessage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Missing content is unknown; an explicitly empty array is a measured zero.
    /// Keeping blocks as JSON preserves future block shapes in content mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_input_tokens: Option<i64>,
    pub cache_creation_input_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation: Option<CacheCreation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheCreation {
    pub ephemeral_5m_input_tokens: Option<i64>,
    pub ephemeral_1h_input_tokens: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WriteStats {
    pub inserted: usize,
    pub enriched: usize,
    pub ignored: usize,
    pub dropped_no_uuid: usize,
}
