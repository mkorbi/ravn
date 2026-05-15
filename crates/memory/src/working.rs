//! Working memory: conversation buffer with auto-compact thresholds.
//!
//! Phase 1 keeps history directly in the agent loop in `ravn-core` —
//! this module is a placeholder so the four-layer Memory taxonomy from
//! `project.md` §1.4 is visible in the workspace.
//!
//! Phase 2 will move buffer management here, including the 80%-context
//! compact trigger and the memory-flush-pre-compress hook that writes
//! curated facts to `semantic.rs` before truncation.
