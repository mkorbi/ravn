//! Native tools shipped with Phase 1 (1.4).
//!
//! 5 Read tools (no approval): [`file_read`], [`web_fetch`],
//! [`session_search`], [`datetime`], plus any future read-only additions.
//! 2 Write tools (approval): [`file_write`], [`memory_save`].
//! 1 Exec tool (approval): [`shell`].
//!
//! Call [`register_defaults`] to bulk-register all seven into a
//! [`crate::ToolRegistry`].

use std::path::PathBuf;

pub mod datetime;
pub mod file_read;
pub mod file_write;
pub mod memory_save;
pub mod session_search;
pub mod shell;
pub mod web_fetch;

pub use datetime::DateTime;
pub use file_read::FileRead;
pub use file_write::FileWrite;
pub use memory_save::MemorySave;
pub use session_search::SessionSearch;
pub use shell::Shell;
pub use web_fetch::WebFetch;

use crate::ToolRegistry;

/// Register all Phase-1 native tools. `data_dir` is the location of
/// `soul.md` / `memory.md` / `user.md` used by `memory_save`.
pub fn register_defaults(reg: &mut ToolRegistry, data_dir: PathBuf) -> &mut ToolRegistry {
    reg.register(FileRead);
    reg.register(FileWrite);
    reg.register(Shell);
    reg.register(WebFetch::new());
    reg.register(SessionSearch);
    reg.register(MemorySave { data_dir });
    reg.register(DateTime);
    reg
}
