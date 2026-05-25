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

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::error::Error;

/// Event `kind` for a logged ReAct step.
pub const STEP_EVENT_KIND: &str = "react.step";

/// Event `kind` for an episode reward (Phase 6.2), keyed by `trace_id`.
pub const REWARD_EVENT_KIND: &str = "react.reward";

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

/// Record an episode reward (Phase 6.2) as a `react.reward` event keyed by
/// `trace_id`. [`export_jsonl`] surfaces it on the trajectory's terminal step.
pub async fn record_reward(
    db: &Db,
    trace_id: &str,
    session_id: Option<&str>,
    reward: f64,
    detail: &str,
) -> Result<i64, Error> {
    crate::events::append_json(
        db,
        Some(trace_id),
        session_id,
        REWARD_EVENT_KIND,
        &serde_json::json!({ "reward": reward, "detail": detail }),
    )
    .await
}

/// Fetch `(trace_id, payload)` rows for one event `kind`, honoring the filter,
/// ordered by event id.
async fn fetch_rows(
    db: &Db,
    kind: &str,
    filter: &Filter,
) -> Result<Vec<(Option<String>, Vec<u8>)>, Error> {
    Ok(match (&filter.session_id, &filter.trace_id) {
        (Some(sid), _) => sqlx::query_as(
            "SELECT trace_id, payload FROM events
              WHERE kind = ?1 AND session_id = ?2 ORDER BY id",
        )
        .bind(kind)
        .bind(sid)
        .fetch_all(&db.pool)
        .await?,
        (None, Some(tid)) => sqlx::query_as(
            "SELECT trace_id, payload FROM events
              WHERE kind = ?1 AND trace_id = ?2 ORDER BY id",
        )
        .bind(kind)
        .bind(tid)
        .fetch_all(&db.pool)
        .await?,
        (None, None) => {
            sqlx::query_as("SELECT trace_id, payload FROM events WHERE kind = ?1 ORDER BY id")
                .bind(kind)
                .fetch_all(&db.pool)
                .await?
        }
    })
}

/// Load logged trajectory steps (ordered by event id), with `trace_id` merged
/// in from the event column and any episode reward attached to each trace's
/// terminal (last) step.
pub async fn load(db: &Db, filter: &Filter) -> Result<Vec<TrajectoryStep>, Error> {
    let mut steps: Vec<TrajectoryStep> = Vec::new();
    for (trace_id, payload) in fetch_rows(db, STEP_EVENT_KIND, filter).await? {
        let mut step: TrajectoryStep = serde_json::from_slice(&payload)?;
        step.trace_id = trace_id.unwrap_or_default();
        steps.push(step);
    }

    // Attach the latest reward per trace to that trace's last step.
    let rewards = load_rewards(db, filter).await?;
    if !rewards.is_empty() {
        let mut last_idx: HashMap<String, usize> = HashMap::new();
        for (i, s) in steps.iter().enumerate() {
            last_idx.insert(s.trace_id.clone(), i);
        }
        for (trace, idx) in last_idx {
            if let Some(r) = rewards.get(&trace) {
                steps[idx].reward = Some(*r);
            }
        }
    }
    Ok(steps)
}

/// Export logged trajectory steps as JSONL (one [`TrajectoryStep`] per line).
pub async fn export_jsonl(db: &Db, filter: &Filter) -> Result<String, Error> {
    let mut out = String::new();
    for step in load(db, filter).await? {
        out.push_str(&serde_json::to_string(&step)?);
        out.push('\n');
    }
    Ok(out)
}

/// Latest reward per `trace_id` (last write wins, since rows are id-ordered).
async fn load_rewards(db: &Db, filter: &Filter) -> Result<HashMap<String, f64>, Error> {
    let mut map = HashMap::new();
    for (trace_id, payload) in fetch_rows(db, REWARD_EVENT_KIND, filter).await? {
        let (Some(tid), Ok(v)) = (trace_id, serde_json::from_slice::<serde_json::Value>(&payload))
        else {
            continue;
        };
        if let Some(r) = v.get("reward").and_then(serde_json::Value::as_f64) {
            map.insert(tid, r);
        }
    }
    Ok(map)
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

    #[tokio::test]
    async fn reward_attaches_to_terminal_step() {
        let db = Db::open_in_memory().await.unwrap();
        crate::sessions::create(&db, "s1", "test", None).await.unwrap();

        let mk = |step: usize, terminal: bool| TrajectoryStep {
            trace_id: String::new(),
            step,
            mode: Some("Fast".into()),
            thought: format!("step {step}"),
            action: if terminal {
                vec![]
            } else {
                vec![Action {
                    tool: "add".into(),
                    input: serde_json::json!({}),
                }]
            },
            observation: vec![],
            reward: None,
        };
        events::append_json(&db, Some("t1"), Some("s1"), STEP_EVENT_KIND, &mk(1, false))
            .await
            .unwrap();
        events::append_json(&db, Some("t1"), Some("s1"), STEP_EVENT_KIND, &mk(2, true))
            .await
            .unwrap();
        record_reward(&db, "t1", Some("s1"), 0.75, "tests_pass=1.00").await.unwrap();

        let jsonl = export_jsonl(&db, &Filter::default()).await.unwrap();
        let steps: Vec<TrajectoryStep> =
            jsonl.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(steps.len(), 2);
        // Reward lands on the terminal (last) step only.
        assert_eq!(steps[0].reward, None);
        assert_eq!(steps[1].reward, Some(0.75));
    }
}
