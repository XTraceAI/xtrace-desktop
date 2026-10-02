//! Transcript parsing, discovery, backfill, watching, and capture receipts.
//!
//! The canonical parser performs no I/O. Persistence and capture delivery are
//! implemented separately from native-line classification.

pub mod canonical;

pub mod native;

pub mod tool_use;

pub mod writer;
