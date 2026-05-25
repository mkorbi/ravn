//! Evaluation harness (Phase 3.11, D18).
//!
//! Loads hand-crafted eval tasks from `tasks/*.toml`, runs each through
//! a real [`ravn_core::Agent`], and grades the final text with
//! `Sonnet 4.6` as the LLM-as-Judge. Outputs an aggregated JSON
//! report (pass-rate, mean cost, mean steps) plus per-task records.
//!
//! Not a unit-test crate — the runner needs a live `ANTHROPIC_API_KEY`
//! and burns real tokens. Invoke via the `ravn-eval` binary.

pub mod audit;
pub mod curator;
pub mod judge;
pub mod reward;
pub mod runner;
pub mod skillrepo;
pub mod synthesis;
pub mod task;

pub use audit::{load_transcripts, parse_findings, write_findings, Auditor, Constitution, Finding};
pub use curator::{mine, render_skill_md, CuratorConfig, SkillCandidate};
pub use judge::{Judge, Judgement};
pub use skillrepo::{promote_committed, PromotionCommit, SkillRepo};
pub use synthesis::{
    decide, promote, verify_and_promote, verify_with_rates, Decision, PassRateMeasurer,
    VerificationReport,
};
pub use reward::{
    score, score_and_record, FileMatches, GitCommitted, Match, RewardOutcome, TestsPass, Verifier,
};
pub use runner::{EvalReport, EvalRunner, TaskOutcome};
pub use task::{EvalTask, ToolSet};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(String),
    #[error("parse {path}: {reason}")]
    Parse { path: String, reason: String },
    #[error("agent: {0}")]
    Agent(String),
    #[error("judge: {0}")]
    Judge(String),
    #[error("synthesis: {0}")]
    Synthesis(String),
    #[error("audit: {0}")]
    Audit(String),
    #[error("persistence: {0}")]
    Persistence(#[from] ravn_persistence::Error),
}
