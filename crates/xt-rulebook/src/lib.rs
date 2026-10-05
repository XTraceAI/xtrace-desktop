//! Scoped rule instances, approval, compilation, verification, and rule fires.
//!
//! Only the read-only [`activity`] snapshot reader is implemented so far. It
//! is a bounded, local reader of *recorded* rule activity; it is not the
//! Rulebook workflow (intake, proposals, activation, verification, judging),
//! which remains unimplemented.

pub mod activity;
