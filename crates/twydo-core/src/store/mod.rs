//! Crash-safe storage primitives: atomic file writes, the vdir lock and the
//! operation journal with recovery. See `docs/journal.md`.

pub mod fsio;
pub mod journal;
