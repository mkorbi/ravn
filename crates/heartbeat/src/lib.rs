//! Heartbeat scheduler (Phase 4.10).
//!
//! User-defined cron jobs (`~/.ravn/heartbeats.toml`) that fire **unattended**
//! agent runs — e.g. "every morning at 8am, summarise my calendar". Each fire
//! runs in its own session with a tight [`Budget`] and a per-job
//! [`AllowlistApprover`], so an autonomous run can only use the Write/Exec
//! tools the job explicitly opted into. Results are reported back over a
//! channel as [`HeartbeatReport`]s rather than streamed into the live UI.
//!
//! [`Budget`]: ravn_core::Budget

pub mod approver;
pub mod config;
pub mod error;
pub mod scheduler;

pub use approver::AllowlistApprover;
pub use config::{HeartbeatConfig, JobConfig};
pub use error::Error;
pub use scheduler::{HeartbeatReport, HeartbeatStatus, Scheduler};
