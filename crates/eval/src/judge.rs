//! LLM-as-Judge (Sonnet 4.6 per D18).
//!
//! Takes a task rubric + the agent's `final_text` + a brief
//! transcript-summary, asks Sonnet for a structured judgement:
//! `{pass: bool, score: 0.0–1.0, reasoning: ""}`. Sonnet is asked to
//! be strict and to fail on missing rubric items rather than be
//! generous.

use std::sync::Arc;

use ravn_llm::{CompletionRequest, ContentBlock, LlmProvider, Message, PromptBuilder};
use serde::{Deserialize, Serialize};

use crate::Error;

const JUDGE_MODEL: &str = "claude-sonnet-4-6";

const JUDGE_SYSTEM: &str = "You are a strict evaluator for an AI agent's responses. \
You receive a task input, a grading rubric, and the agent's final text answer. \
Output a single JSON object with these exact keys: \
{\"pass\": boolean, \"score\": number between 0 and 1, \"reasoning\": string under 200 chars}. \
Be strict: missing any rubric item should fail. Output ONLY the JSON, no prose.";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Judgement {
    pub pass: bool,
    pub score: f64,
    pub reasoning: String,
}

pub struct Judge {
    provider: Arc<dyn LlmProvider>,
}

impl Judge {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }

    pub async fn grade(
        &self,
        input: &str,
        rubric: &str,
        final_text: &str,
    ) -> Result<Judgement, Error> {
        let user = format!(
            "TASK INPUT:\n{input}\n\nRUBRIC:\n{rubric}\n\nAGENT ANSWER:\n{final_text}\n\n\
             Now output the JSON judgement (no other text)."
        );
        let req = PromptBuilder::new()
            .system(JUDGE_SYSTEM)
            .build(JUDGE_MODEL, Message::user(user), 1024);

        let text = collect_text(&self.provider, req).await?;
        // Strip ```json fences if the model includes them despite the prompt.
        let trimmed = strip_code_fences(&text);
        serde_json::from_str::<Judgement>(trimmed.trim()).map_err(|e| {
            Error::Judge(format!("non-JSON response: {e}\nraw output:\n{text}"))
        })
    }
}

pub(crate) async fn collect_text(
    provider: &Arc<dyn LlmProvider>,
    req: CompletionRequest,
) -> Result<String, Error> {
    // Use the non-streaming `complete` path — judge calls are short
    // and we don't need token-by-token UX.
    let resp = provider
        .complete(req)
        .await
        .map_err(|e| Error::Judge(format!("provider: {e}")))?;
    let mut out = String::new();
    for block in resp.content {
        if let ContentBlock::Text { text } = block {
            out.push_str(&text);
        }
    }
    Ok(out)
}

pub(crate) fn strip_code_fences(s: &str) -> &str {
    let trimmed = s.trim();
    if let Some(rest) = trimmed.strip_prefix("```json") {
        return rest.strip_suffix("```").unwrap_or(rest).trim();
    }
    if let Some(rest) = trimmed.strip_prefix("```") {
        return rest.strip_suffix("```").unwrap_or(rest).trim();
    }
    trimmed
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_json_fences() {
        let raw = "```json\n{\"pass\": true, \"score\": 1.0, \"reasoning\": \"x\"}\n```";
        let cleaned = strip_code_fences(raw);
        let j: Judgement = serde_json::from_str(cleaned).unwrap();
        assert!(j.pass);
        assert_eq!(j.score, 1.0);
    }

    #[test]
    fn parses_plain_json() {
        let raw = "{\"pass\": false, \"score\": 0.4, \"reasoning\": \"missing X\"}";
        let cleaned = strip_code_fences(raw);
        let j: Judgement = serde_json::from_str(cleaned).unwrap();
        assert!(!j.pass);
        assert!((j.score - 0.4).abs() < 1e-9);
        assert!(j.reasoning.contains("missing"));
    }
}
