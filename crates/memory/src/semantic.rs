//! Semantic memory: persistent, curated text the user has chosen to keep
//! in the agent's permanent system prompt prefix.
//!
//! Three files live in the memory dir (see [`Self::load`]):
//! * `soul.md` — persona / identity / values
//! * `memory.md` — long-term facts the agent has learned
//! * `user.md` — model of the user (preferences, role, context)
//!
//! Missing files are not an error — semantic memory is optional and
//! starts empty on a fresh install. Token-budget enforcement lives in
//! [`super::limits`] (Phase 1.7).

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Error;

/// Which of the three semantic-memory files we're addressing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Slot {
    Soul,
    Memory,
    User,
}

impl Slot {
    pub fn filename(self) -> &'static str {
        match self {
            Slot::Soul => "soul.md",
            Slot::Memory => "memory.md",
            Slot::User => "user.md",
        }
    }

    pub fn all() -> [Slot; 3] {
        [Slot::Soul, Slot::Memory, Slot::User]
    }
}

/// In-memory view of all three semantic files. `None` means the file
/// doesn't exist or is empty after trimming.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticMemory {
    pub soul: Option<String>,
    pub memory: Option<String>,
    pub user: Option<String>,
}

impl SemanticMemory {
    pub async fn load(dir: &Path) -> Result<Self, Error> {
        Ok(Self {
            soul: read_optional(&dir.join(Slot::Soul.filename())).await?,
            memory: read_optional(&dir.join(Slot::Memory.filename())).await?,
            user: read_optional(&dir.join(Slot::User.filename())).await?,
        })
    }

    pub fn get(&self, slot: Slot) -> Option<&str> {
        match slot {
            Slot::Soul => self.soul.as_deref(),
            Slot::Memory => self.memory.as_deref(),
            Slot::User => self.user.as_deref(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.soul.is_none() && self.memory.is_none() && self.user.is_none()
    }

    /// Total bytes across all three slots — cheap proxy for token budget
    /// before the proper tokenizer-based check runs in [`super::limits`].
    pub fn total_bytes(&self) -> usize {
        Slot::all()
            .iter()
            .filter_map(|s| self.get(*s))
            .map(str::len)
            .sum()
    }
}

/// Persist a slot to disk. Overwrites atomically via a tmp-file + rename.
pub async fn write_slot(dir: &Path, slot: Slot, body: &str) -> Result<(), Error> {
    let target = dir.join(slot.filename());
    let tmp = target.with_extension("md.tmp");
    tokio::fs::create_dir_all(dir).await.map_err(|e| Error::Io {
        path: dir.to_path_buf(),
        source: e,
    })?;
    tokio::fs::write(&tmp, body).await.map_err(|e| Error::Io {
        path: tmp.clone(),
        source: e,
    })?;
    tokio::fs::rename(&tmp, &target)
        .await
        .map_err(|e| Error::Io {
            path: target,
            source: e,
        })
}

/// Append `body` under a new `## {section}` heading. Creates the file
/// if it doesn't exist. Phase 1 `memory_save` tool uses this for
/// timestamped appends.
pub async fn append_section(
    dir: &Path,
    slot: Slot,
    section: &str,
    body: &str,
) -> Result<(), Error> {
    let target = dir.join(slot.filename());
    let existing = read_optional(&target).await?.unwrap_or_default();
    let separator = if existing.is_empty() || existing.ends_with("\n\n") {
        ""
    } else if existing.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    let next = format!("{existing}{separator}## {section}\n\n{body}\n");
    write_slot(dir, slot, &next).await
}

async fn read_optional(path: &Path) -> Result<Option<String>, Error> {
    match tokio::fs::read_to_string(path).await {
        Ok(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(s))
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) if e.kind() == io::ErrorKind::InvalidData => {
            Err(Error::NotUtf8(PathBuf::from(path)))
        }
        Err(e) => Err(Error::Io {
            path: PathBuf::from(path),
            source: e,
        }),
    }
}
