//! The wire contract and the running reads of the host-title overlay.
//!
//! Sessions and the Dashboard's lane list show a host's own title for the rows
//! they are showing, read from the original local sources on demand and never
//! stored. The view names the rows by canonical identifier and nothing else;
//! which file to read, and whether a title may be read at all, is resolved
//! against the index behind the command.
//!
//! The answer is sparse: a row with no host title, or one whose source could
//! not be verified, read in time or read at all, is simply absent, and keeps
//! its identifier. No reason crosses the boundary — the view's fallback is the
//! same whatever the reason, and a path or a failure's detail is local.
use serde::Serialize;
use ts_rs::TS;

/// One row's host title, as its host shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct SessionTitle {
    /// The canonical identifier the request named, exactly.
    pub id: String,
    pub title: String,
}

/// The titles found for a request, in request order. Only requested
/// identifiers appear, each at most once.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, TS)]
pub struct SessionTitles {
    pub titles: Vec<SessionTitle>,
}

/// The cancel tokens of the title reads running now, apart from transcript
/// reads so neither kind can crowd out or cancel the other. The same rules
/// apply: one entry per running read under the view's own name for it, a
/// cancel that arrives first is remembered, and shutdown cancels and waits.
#[derive(Default)]
pub struct TitleReads(crate::transcript_reads::TranscriptReads);

impl std::ops::Deref for TitleReads {
    type Target = crate::transcript_reads::TranscriptReads;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
