//! Transcript parsing, discovery, backfill, watching, and capture receipts.
//!
//! The canonical parser performs no I/O. Writers, watching and capture delivery
//! are introduced by their owning ingestion stages.

pub mod canonical;
