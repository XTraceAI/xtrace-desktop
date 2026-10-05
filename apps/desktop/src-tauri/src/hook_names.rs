//! Bounded, transient labels from original Claude stop summaries.
use serde::Serialize;
use ts_rs::TS;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, TS)]
pub struct HookNames {
    pub requested_summaries: u32,
    pub checked_summaries: u32,
    pub unavailable_summaries: u32,
    pub summaries_with_unnamed_commands: u32,
    pub labels: Vec<HookNameCount>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct HookNameCount {
    pub script_basename: String,
    pub display_label: String,
    pub summaries_mentioning: u32,
}

/// Independent from transcript/title reads, so closing one dialog cancels
/// exactly its own request.
#[derive(Default)]
pub struct HookNameReads(crate::transcript_reads::TranscriptReads);

impl std::ops::Deref for HookNameReads {
    type Target = crate::transcript_reads::TranscriptReads;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
