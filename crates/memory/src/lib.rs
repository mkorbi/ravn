//! Memory layers per `project.md` §1.4.
//!
//! | Layer       | Phase | Backing                              |
//! |-------------|-------|--------------------------------------|
//! | Working     | 2     | (placeholder — buffer lives in core) |
//! | Episodic    | 2     | `ravn-persistence` FTS5 + sqlite-vec |
//! | Semantic    | **1** | `soul.md`/`memory.md`/`user.md`      |
//! | Procedural  | 2     | (placeholder — Skills system)        |
//!
//! Phase 1 implements only the Semantic layer ([`semantic`]). The other
//! three modules exist as documented stubs so the taxonomy is visible at
//! the workspace level — they get fleshed out alongside their target
//! features in Phase 2.

pub mod episodic;
pub mod error;
pub mod limits;
pub mod procedural;
pub mod semantic;
pub mod working;

pub use error::Error;
pub use limits::{enforce, estimate_tokens, Limits, Trimmed};
pub use semantic::{append_section, write_slot, SemanticMemory, Slot};

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn td() -> TempDir {
        TempDir::new().expect("tempdir")
    }

    #[tokio::test]
    async fn missing_dir_yields_empty_memory() {
        let dir = td();
        let mem = SemanticMemory::load(dir.path()).await.unwrap();
        assert!(mem.is_empty());
        assert_eq!(mem.total_bytes(), 0);
    }

    #[tokio::test]
    async fn write_then_load_round_trips() {
        let dir = td();
        write_slot(dir.path(), Slot::Soul, "# Soul\nI am ravn.")
            .await
            .unwrap();
        write_slot(dir.path(), Slot::User, "Max prefers brevity.")
            .await
            .unwrap();

        let mem = SemanticMemory::load(dir.path()).await.unwrap();
        assert!(mem.soul.as_deref().unwrap().contains("ravn"));
        assert_eq!(mem.user.as_deref(), Some("Max prefers brevity."));
        assert!(mem.memory.is_none());
    }

    #[tokio::test]
    async fn append_section_concatenates_under_heading() {
        let dir = td();
        append_section(dir.path(), Slot::Memory, "2026-05-15", "User uses Rust 1.95.")
            .await
            .unwrap();
        append_section(dir.path(), Slot::Memory, "2026-05-16", "User prefers German.")
            .await
            .unwrap();

        let mem = SemanticMemory::load(dir.path()).await.unwrap();
        let body = mem.memory.unwrap();
        assert!(body.contains("## 2026-05-15"));
        assert!(body.contains("## 2026-05-16"));
        assert!(body.find("2026-05-15").unwrap() < body.find("2026-05-16").unwrap());
    }

    #[tokio::test]
    async fn empty_or_whitespace_file_treated_as_none() {
        let dir = td();
        write_slot(dir.path(), Slot::User, "   \n\n  ").await.unwrap();
        let mem = SemanticMemory::load(dir.path()).await.unwrap();
        assert!(mem.user.is_none());
    }
}
