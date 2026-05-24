//! Phase 6.1: trajectory schema-lock + JSONL export.
//!
//! Each ReAct iteration is logged as a [`TrajectoryStep`] under the
//! [`STEP_EVENT_KIND`] event kind. The same type is the unit of the JSONL
//! export consumed by RL tooling later (Phase 6.6+) — locking the shape now,
//! per project.md's "trajectory logging from day one", so the data is usable
//! without a schema migration when training arrives.
//!
//! `trace_id` lives in the `events` table column (not duplicated in the stored
//! payload); [`export_jsonl`] merges it back in so each emitted line is the
//! self-contained record `{trace_id, step, thought, action, observation,
//! reward?}`.

use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::error::Error;

/// Event `kind` for a logged ReAct step.
pub const STEP_EVENT_KIND: &str = "react.step";

/// One ReAct iteration: the model's reasoning, the tool calls it issued, and
/// the results observed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryStep {
    /// Correlates all steps of one run. Empty in the stored payload (it's the
    /// event's `trace_id` column); populated by [`export_jsonl`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub trace_id: String,
    /// 1-based iteration index within the run.
    pub step: usize,
    /// Reasoning mode for this step (`Fast`, `Deep`, `Reflect`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// The assistant's text output for this step (the "thought").
    pub thought: String,
    /// Tool calls issued this step (the "action"). Empty on a terminal step.
    pub action: Vec<Action>,
    /// Tool results observed this step. Empty on a terminal step.
    pub observation: Vec<Observation>,
    /// Reward signal — `None` until a reward function (Phase 6.2) scores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reward: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    pub tool: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub tool: String,
    pub content: String,
    pub is_error: bool,
}

/// Filter for [`export_jsonl`]. Default exports every logged step.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub session_id: Option<String>,
    pub trace_id: Option<String>,
}

/// Export logged trajectory steps as JSONL (one [`TrajectoryStep`] per line,
/// ordered by event id), with `trace_id` merged in from the event column.
pub async fn export_jsonl(db: &Db, filter: &Filter) -> Result<String, Error> {
    let rows: Vec<(Option<String>, Vec<u8>)> = match (&filter.session_id, &filter.trace_id) {
        (Some(sid), _) => sqlx::query_as(
            "SELECT trace_id, payload FROM events
              WHERE kind = ?1 AND session_id = ?2 ORDER BY id",
        )
        .bind(STEP_EVENT_KIND)
        .bind(sid)
        .fetch_all(&db.pool)
        .await?,
        (None, Some(tid)) => sqlx::query_as(
            "SELECT trace_id, payload FROM events
              WHERE kind = ?1 AND trace_id = ?2 ORDER BY id",
        )
        .bind(STEP_EVENT_KIND)
        .bind(tid)
        .fetch_all(&db.pool)
        .await?,
        (None, None) => sqlx::query_as(
            "SELECT trace_id, payload FROM events WHERE kind = ?1 ORDER BY id",
        )
        .bind(STEP_EVENT_KIND)
        .fetch_all(&db.pool)
        .await?,
    };

    let mut out = String::new();
    for (trace_id, payload) in rows {
        let mut step: TrajectoryStep = serde_json::from_slice(&payload)?;
        step.trace_id = trace_id.unwrap_or_default();
        out.push_str(&serde_json::to_string(&step)?);
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events;

    #[tokio::test]
    async fn export_round_trips_and_merges_trace_id() {
        let db = Db::open_in_memory().await.unwrap();
        crate::sessions::create(&db, "s1", "test", None).await.unwrap();

        let step = TrajectoryStep {
            trace_id: String::new(), // omitted from payload on the wire
            step: 1,
            mode: Some("Fast".into()),
            thought: "let me add".into(),
            action: vec![Action {
                tool: "add".into(),
                input: serde_json::json!({"a": 1, "b": 2}),
            }],
            observation: vec![Observation {
                tool: "add".into(),
                content: "3".into(),
                is_error: false,
            }],
            reward: None,
        };
        events::append_json(&db, Some("trace-1"), Some("s1"), STEP_EVENT_KIND, &step)
            .await
            .unwrap();
        // A non-trajectory event must be ignored by the export.
        events::append_json(&db, Some("trace-1"), Some("s1"), "react.done", &serde_json::json!({}))
            .await
            .unwrap();

        let jsonl = export_jsonl(&db, &Filter::default()).await.unwrap();
        let lines: Vec<&str> = jsonl.lines().collect();
        assert_eq!(lines.len(), 1, "only react.step rows are exported");

        let parsed: TrajectoryStep = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(parsed.trace_id, "trace-1", "trace_id merged from column");
        assert_eq!(parsed.step, 1);
        assert_eq!(parsed.action[0].tool, "add");
        assert_eq!(parsed.observation[0].content, "3");

        // Filter by a non-matching trace yields nothing.
        let empty = export_jsonl(
            &db,
            &Filter {
                trace_id: Some("nope".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(empty.is_empty());
    }
}
