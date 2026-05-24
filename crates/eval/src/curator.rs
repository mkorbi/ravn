//! Phase 6.3: the curator mines recurring tool-call sequences out of logged
//! trajectories and abstracts them into **SKILL.md candidates**.
//!
//! The idea (project.md §"self-improvement"): if the agent keeps solving tasks
//! with the same short tool sequence, that sequence is worth promoting to a
//! named skill. The curator only *proposes* — candidates land in a staging
//! directory for verification (Phase 6.4) and git-tracked promotion (6.5), not
//! into the live skills set.
//!
//! Mining is deliberately simple: count how many distinct (optionally
//! reward-filtered) trajectories contain each contiguous tool n-gram, keep
//! those at/above a support threshold, rank by support then length.

use std::collections::{HashMap, HashSet};

use ravn_persistence::trajectory::TrajectoryStep;

#[derive(Debug, Clone)]
pub struct CuratorConfig {
    /// Only mine traces whose episode reward is at least this. A trace with no
    /// recorded reward counts as `0.0`, so the default (`0.0`) mines everything;
    /// raise it (e.g. `1.0`) to learn only from verified successes (Phase 6.2).
    pub min_reward: f64,
    /// A sequence must appear in at least this many distinct traces.
    pub min_support: usize,
    /// Inclusive bounds on candidate sequence length (in tool calls).
    pub min_len: usize,
    pub max_len: usize,
    /// Cap on emitted candidates (highest support first).
    pub max_candidates: usize,
}

impl Default for CuratorConfig {
    fn default() -> Self {
        Self {
            min_reward: 0.0,
            min_support: 2,
            min_len: 2,
            max_len: 4,
            max_candidates: 20,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkillCandidate {
    /// Generated kebab-case skill name, e.g. `auto-web-fetch-then-file-write`.
    pub name: String,
    /// The recurring tool sequence.
    pub sequence: Vec<String>,
    /// Number of distinct trajectories the sequence appeared in.
    pub support: usize,
}

/// Mine [`SkillCandidate`]s from trajectory steps.
pub fn mine(steps: &[TrajectoryStep], cfg: &CuratorConfig) -> Vec<SkillCandidate> {
    let sequences = trace_sequences(steps, cfg.min_reward);

    // For each n-gram, count the number of *distinct* traces containing it.
    let mut support: HashMap<Vec<String>, usize> = HashMap::new();
    for seq in &sequences {
        let mut seen: HashSet<Vec<String>> = HashSet::new();
        for len in cfg.min_len..=cfg.max_len {
            if seq.len() < len {
                break;
            }
            for window in seq.windows(len) {
                seen.insert(window.to_vec());
            }
        }
        for ngram in seen {
            *support.entry(ngram).or_insert(0) += 1;
        }
    }

    let mut candidates: Vec<SkillCandidate> = support
        .into_iter()
        .filter(|(_, n)| *n >= cfg.min_support)
        .map(|(sequence, support)| SkillCandidate {
            name: candidate_name(&sequence),
            sequence,
            support,
        })
        .collect();

    // Highest support first; longer sequences before shorter; name for ties
    // (deterministic output run-to-run).
    candidates.sort_by(|a, b| {
        b.support
            .cmp(&a.support)
            .then(b.sequence.len().cmp(&a.sequence.len()))
            .then(a.name.cmp(&b.name))
    });
    candidates.truncate(cfg.max_candidates);
    candidates
}

/// Per-trace ordered tool-call sequences, keeping only traces whose episode
/// reward (attached to the terminal step by `trajectory::load`) is at least
/// `min_reward`. Order within a trace is preserved; a missing reward is `0.0`.
fn trace_sequences(steps: &[TrajectoryStep], min_reward: f64) -> Vec<Vec<String>> {
    // Insertion-ordered accumulation per trace.
    let mut order: Vec<String> = Vec::new();
    let mut tools: HashMap<String, Vec<String>> = HashMap::new();
    let mut reward: HashMap<String, f64> = HashMap::new();

    for step in steps {
        let trace = step.trace_id.clone();
        if !tools.contains_key(&trace) {
            order.push(trace.clone());
        }
        let entry = tools.entry(trace.clone()).or_default();
        for action in &step.action {
            entry.push(action.tool.clone());
        }
        if let Some(r) = step.reward {
            let slot = reward.entry(trace).or_insert(f64::MIN);
            *slot = slot.max(r);
        }
    }

    order
        .into_iter()
        .filter(|trace| reward.get(trace).copied().unwrap_or(0.0) >= min_reward)
        .filter_map(|trace| tools.remove(&trace))
        .filter(|seq| !seq.is_empty())
        .collect()
}

fn candidate_name(sequence: &[String]) -> String {
    let body = sequence
        .iter()
        .map(|t| t.replace('_', "-"))
        .collect::<Vec<_>>()
        .join("-then-");
    format!("auto-{body}")
}

/// Render a candidate as a SKILL.md document compatible with the skills-crate
/// frontmatter (`name`, `description`, `allowed_tools`).
pub fn render_skill_md(candidate: &SkillCandidate) -> String {
    let arrow = candidate.sequence.join(" → ");

    // Unique tools, order-preserving, for `allowed_tools`.
    let mut seen = HashSet::new();
    let allowed: Vec<&String> = candidate
        .sequence
        .iter()
        .filter(|t| seen.insert((*t).clone()))
        .collect();
    let allowed_yaml: String = allowed.iter().map(|t| format!("  - {t}\n")).collect();

    let steps: String = candidate
        .sequence
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{}. `{t}`\n", i + 1))
        .collect();

    format!(
        "---\n\
         name: {name}\n\
         description: Auto-synthesized candidate — tool sequence {arrow}, observed in {support} trajectories.\n\
         allowed_tools:\n{allowed_yaml}\
         ---\n\n\
         # {name}\n\n\
         Recurring tool sequence mined from {support} trajectories:\n\n\
         {steps}\n\
         > Auto-generated by the curator (Phase 6.3). A candidate only — verify on\n\
         > historical tasks (6.4) before promoting into the live skills set.\n",
        name = candidate.name,
        arrow = arrow,
        support = candidate.support,
        allowed_yaml = allowed_yaml,
        steps = steps,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ravn_persistence::trajectory::Action;

    fn step(trace: &str, tools: &[&str], reward: Option<f64>) -> TrajectoryStep {
        TrajectoryStep {
            trace_id: trace.into(),
            step: 1,
            mode: None,
            thought: String::new(),
            action: tools
                .iter()
                .map(|t| Action {
                    tool: (*t).into(),
                    input: serde_json::Value::Null,
                })
                .collect(),
            observation: vec![],
            reward,
        }
    }

    #[test]
    fn mines_frequent_sequences_above_support() {
        let steps = vec![
            step("a", &["web_fetch", "file_write", "datetime"], Some(1.0)),
            step("b", &["web_fetch", "file_write", "datetime"], Some(1.0)),
            step("c", &["web_fetch", "file_write", "shell"], Some(1.0)),
        ];
        let cfg = CuratorConfig {
            min_support: 2,
            ..Default::default()
        };
        let candidates = mine(&steps, &cfg);

        // [web_fetch, file_write] appears in all 3 traces → top by support.
        let top = &candidates[0];
        assert_eq!(top.sequence, vec!["web_fetch", "file_write"]);
        assert_eq!(top.support, 3);
        assert_eq!(top.name, "auto-web-fetch-then-file-write");

        // A sequence unique to one trace is below the threshold.
        assert!(!candidates
            .iter()
            .any(|c| c.sequence == vec!["file_write".to_string(), "shell".to_string()]));
        // [web_fetch, file_write, datetime] occurs in 2 traces → present.
        assert!(candidates.iter().any(|c| c.sequence.len() == 3 && c.support == 2));
    }

    #[test]
    fn min_reward_filters_unsuccessful_traces() {
        let steps = vec![
            step("a", &["x", "y"], Some(1.0)),
            step("b", &["x", "y"], Some(0.0)), // below threshold
            step("c", &["x", "y"], None),      // no reward → 0.0
        ];
        let cfg = CuratorConfig {
            min_reward: 1.0,
            min_support: 2,
            ..Default::default()
        };
        // Only trace "a" qualifies → support 1 < min_support → nothing.
        assert!(mine(&steps, &cfg).is_empty());

        let cfg0 = CuratorConfig {
            min_reward: 0.0,
            min_support: 2,
            ..Default::default()
        };
        assert_eq!(mine(&steps, &cfg0)[0].support, 3);
    }

    #[test]
    fn renders_parseable_frontmatter() {
        let c = SkillCandidate {
            name: "auto-web-fetch-then-file-write".into(),
            sequence: vec!["web_fetch".into(), "file_write".into()],
            support: 4,
        };
        let md = render_skill_md(&c);
        assert!(md.starts_with("---\n"));
        assert!(md.contains("name: auto-web-fetch-then-file-write"));
        assert!(md.contains("allowed_tools:\n  - web_fetch\n  - file_write\n"));
        assert!(md.contains("observed in 4 trajectories"));
    }

    /// A rendered candidate must be a valid SKILL.md the skills crate can parse.
    #[tokio::test]
    async fn rendered_candidate_parses_as_a_skill() {
        let c = SkillCandidate {
            name: "auto-web-fetch-then-file-write".into(),
            sequence: vec!["web_fetch".into(), "file_write".into(), "web_fetch".into()],
            support: 3,
        };
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join(&c.name);
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        let path = skill_dir.join("SKILL.md");
        tokio::fs::write(&path, render_skill_md(&c)).await.unwrap();

        let skill = ravn_skills::load_skill(&path).await.unwrap();
        assert_eq!(skill.name, "auto-web-fetch-then-file-write");
        // allowed_tools is de-duplicated, order-preserving.
        assert_eq!(skill.allowed_tools, vec!["web_fetch", "file_write"]);
        assert!(!skill.description.is_empty());
    }
}
