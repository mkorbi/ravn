//! Episodic memory: FTS5 + (Phase 2) vector search across past sessions.
//!
//! The actual storage lives in `ravn-persistence`. The
//! [`session_search`] tool in Phase 1.4 calls
//! `ravn_persistence::messages::search` directly. In Phase 2 this
//! module wraps the query with re-ranking + recency-bias and exposes a
//! hybrid retrieval API that combines BM25 with `sqlite-vec`.
