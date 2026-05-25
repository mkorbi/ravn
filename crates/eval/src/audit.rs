//! Phase 6.9: constitutional self-auditing.
//!
//! A user writes a **constitution** (markdown rules — privacy, tone, safety).
//! An auditor agent reviews recent session transcripts against it and records
//! concrete violations into `memory.md`, so the assistant carries its own
//! review findings forward as long-term memory.
//!
//! The review itself is an LLM call (mirrors [`crate::judge`]); the
//! constitution/transcript loading, response parsing, and memory write are pure
//! and tested directly.

use std::path::Path;
use std::sync::Arc;

use ravn_llm::{ContentBlock, LlmProvider, Message, PromptBuilder};
use ravn_memory::{append_section, Slot};
use ravn_persistence::{messages, sessions, Db};
use serde::{Deserialize, Serialize};

use crate::judge::{collect_text, strip_code_fences};
use crate::Error;

const AUDITOR_MODEL: &str = "claude-sonnet-4-6";

const AUDITOR_SYSTEM: &str = "You are a privacy- and quality-focused auditor for a personal AI \
assistant. You receive a constitution (rules the assistant must follow) and transcripts of recent \
sessions. Identify concrete, specific violations of the constitution — not vague concerns. Output \
ONLY a JSON array; each element exactly: {\"session_id\": string, \"principle\": string (the rule \
violated), \"severity\": \"low\"|\"medium\"|\"high\", \"finding\": string under 200 chars}. If \
there are no clear violations, output [].";

/// Per-message content cap, to keep the audit prompt bounded.
const MSG_CAP: usize = 2000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub session_id: String,
    pub principle: String,
    pub severity: String,
    pub finding: String,
}

/// The user-defined constitution (markdown).
pub struct Constitution {
    pub text: String,
}

impl Constitution {
    pub async fn load(path: &Path) -> Result<Self, Error> {
        let text = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
        Ok(Self { text })
    }
}

#[derive(Debug, Clone)]
pub struct SessionTranscript {
    pub session_id: String,
    pub channel: String,
    pub text: String,
}

/// Load transcripts for the most recent `limit` sessions, rendering each
/// message's text (tool calls/results are noted briefly, content capped).
pub async fn load_transcripts(db: &Db, limit: i64) -> Result<Vec<SessionTranscript>, Error> {
    let recent = sessions::recent(db, limit).await?;
    let mut out = Vec::with_capacity(recent.len());
    for s in recent {
        let rows = messages::list_session(db, &s.id).await?;
        let mut text = String::new();
        for m in rows {
            let rendered = render_content(&m.content);
            if rendered.trim().is_empty() {
                continue;
            }
            text.push_str(&m.role);
            text.push_str(": ");
            text.push_str(&truncate(&rendered, MSG_CAP));
            text.push('\n');
        }
        out.push(SessionTranscript {
            session_id: s.id,
            channel: s.channel,
            text,
        });
    }
    Ok(out)
}

/// Render a message's stored `Vec<ContentBlock>` JSON to readable text.
fn render_content(json: &str) -> String {
    let Ok(blocks) = serde_json::from_str::<Vec<ContentBlock>>(json) else {
        return json.to_string(); // fallback: raw
    };
    let mut s = String::new();
    for b in blocks {
        match b {
            ContentBlock::Text { text } => s.push_str(&text),
            ContentBlock::ToolUse { name, .. } => {
                s.push_str(&format!("[tool:{name}]"));
            }
            ContentBlock::ToolResult { content, .. } => {
                s.push_str(&format!("[result:{}]", truncate(&content, 200)));
            }
            _ => {}
        }
    }
    s
}

fn truncate(s: &str, cap: usize) -> String {
    if s.len() <= cap {
        return s.to_string();
    }
    let end = s
        .char_indices()
        .take_while(|(i, _)| *i < cap)
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    format!("{}…", &s[..end])
}

pub struct Auditor {
    provider: Arc<dyn LlmProvider>,
    model: String,
}

impl Auditor {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self {
            provider,
            model: AUDITOR_MODEL.to_string(),
        }
    }

    /// Review `transcripts` against `constitution`, returning findings.
    pub async fn audit(
        &self,
        constitution: &Constitution,
        transcripts: &[SessionTranscript],
    ) -> Result<Vec<Finding>, Error> {
        if transcripts.is_empty() {
            return Ok(Vec::new());
        }
        let mut sessions_block = String::new();
        for t in transcripts {
            sessions_block.push_str(&format!(
                "=== SESSION {} (channel {}) ===\n{}\n\n",
                t.session_id, t.channel, t.text
            ));
        }
        let user = format!(
            "CONSTITUTION:\n{}\n\nRECENT SESSIONS:\n{}\n\nOutput the JSON array of findings now.",
            constitution.text, sessions_block
        );
        let req = PromptBuilder::new()
            .system(AUDITOR_SYSTEM)
            .build(&self.model, Message::user(user), 2048);
        let text = collect_text(&self.provider, req)
            .await
            .map_err(|e| Error::Audit(e.to_string()))?;
        parse_findings(&text)
    }
}

/// Parse the auditor's JSON-array response (tolerating ```json fences).
pub fn parse_findings(raw: &str) -> Result<Vec<Finding>, Error> {
    let cleaned = strip_code_fences(raw);
    serde_json::from_str::<Vec<Finding>>(cleaned.trim())
        .map_err(|e| Error::Audit(format!("non-JSON auditor response: {e}\nraw output:\n{raw}")))
}

/// Append findings to `memory.md` under a dated `## Audit findings <date>`
/// heading. No-op when there are no findings.
pub async fn write_findings(memory_dir: &Path, findings: &[Finding]) -> Result<(), Error> {
    if findings.is_empty() {
        return Ok(());
    }
    let date = chrono::Utc::now().format("%Y-%m-%d");
    let mut body = String::new();
    for f in findings {
        let sid = &f.session_id[..f.session_id.len().min(8)];
        body.push_str(&format!(
            "- [{}] {} — {} (session {sid})\n",
            f.severity, f.principle, f.finding
        ));
    }
    append_section(
        memory_dir,
        Slot::Memory,
        &format!("Audit findings {date}"),
        body.trim_end(),
    )
    .await
    .map_err(|e| Error::Audit(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn loads_transcripts_from_recent_sessions() {
        let db = Db::open_in_memory().await.unwrap();
        sessions::create(&db, "s1", "cli", Some("mock")).await.unwrap();
        messages::append(&db, "s1", "user", r#"[{"type":"text","text":"my SSN is 123"}]"#)
            .await
            .unwrap();
        messages::append(
            &db,
            "s1",
            "assistant",
            r#"[{"type":"tool_use","id":"t1","name":"shell","input":{}}]"#,
        )
        .await
        .unwrap();

        let transcripts = load_transcripts(&db, 100).await.unwrap();
        assert_eq!(transcripts.len(), 1);
        assert_eq!(transcripts[0].session_id, "s1");
        assert!(transcripts[0].text.contains("user: my SSN is 123"));
        assert!(transcripts[0].text.contains("[tool:shell]"));
    }

    #[test]
    fn parses_findings_array_and_empty() {
        let raw = r#"```json
        [{"session_id":"s1","principle":"no PII in logs","severity":"high","finding":"logged an SSN"}]
        ```"#;
        let findings = parse_findings(raw).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "high");

        assert!(parse_findings("[]").unwrap().is_empty());
        assert!(parse_findings("not json").is_err());
    }

    #[tokio::test]
    async fn write_findings_appends_dated_section() {
        let dir = tempfile::tempdir().unwrap();
        let findings = vec![Finding {
            session_id: "sess-abcdef12".into(),
            principle: "no PII".into(),
            severity: "high".into(),
            finding: "logged an SSN".into(),
        }];
        write_findings(dir.path(), &findings).await.unwrap();

        let memory = tokio::fs::read_to_string(dir.path().join("memory.md")).await.unwrap();
        assert!(memory.contains("## Audit findings"));
        assert!(memory.contains("[high] no PII — logged an SSN (session sess-abc"));

        // Empty findings is a no-op (no second section added).
        write_findings(dir.path(), &[]).await.unwrap();
        let after = tokio::fs::read_to_string(dir.path().join("memory.md")).await.unwrap();
        assert_eq!(after.matches("## Audit findings").count(), 1);
    }
}
