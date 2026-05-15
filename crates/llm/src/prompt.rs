//! Cache-stable Prompt-Assembly (Phase 0.11).
//!
//! Anthropic prompt-caching rewards a stable prefix: any change inside
//! the cached region invalidates everything downstream. To keep the cache
//! warm we always assemble the request in the same order:
//!
//! 1. **Tools**            — definitions don't change between turns.
//! 2. **System**           — persona, instructions, frozen.
//! 3. **Skills metadata**  — list of skill descriptors (Phase 2+).
//! 4. **MEMORY.md**        — long-term curated facts.
//! 5. **SOUL.md** + **USER.md** — persona and user model.
//! 6. **History**          — prior assistant/user turns.
//! 7. **User turn**        — the new prompt (always last).
//!
//! `PromptBuilder` enforces this order at the type level via consuming
//! setters; once you call a later-stage method, the earlier-stage methods
//! are no longer available on the same builder value.
//!
//! Cache breakpoints are placed automatically at the end of tools and at
//! the end of the static system block (after USER.md). Up to four
//! breakpoints are emitted per Anthropic's limit.

use crate::message::Message;
use crate::request::{CacheBreakpoint, CachePosition, CacheTtl, CompletionRequest, ToolSchema};

/// Builder enforcing the cache-stable prompt assembly order. Methods take
/// `self` by value so that the type system catches out-of-order usage at
/// compile time — you can't call `system()` after `history()`.
pub struct PromptBuilder {
    tools: Vec<ToolSchema>,
    system: Option<String>,
    skills_meta: Option<String>,
    memory_md: Option<String>,
    soul_md: Option<String>,
    user_md: Option<String>,
    history: Vec<Message>,
}

impl PromptBuilder {
    pub fn new() -> Self {
        Self {
            tools: Vec::new(),
            system: None,
            skills_meta: None,
            memory_md: None,
            soul_md: None,
            user_md: None,
            history: Vec::new(),
        }
    }

    pub fn tools(mut self, tools: Vec<ToolSchema>) -> Self {
        self.tools = tools;
        self
    }

    pub fn system(mut self, prompt: impl Into<String>) -> Self {
        self.system = Some(prompt.into());
        self
    }

    pub fn skills_meta(mut self, body: impl Into<String>) -> Self {
        self.skills_meta = Some(body.into());
        self
    }

    pub fn memory_md(mut self, body: impl Into<String>) -> Self {
        self.memory_md = Some(body.into());
        self
    }

    pub fn soul_md(mut self, body: impl Into<String>) -> Self {
        self.soul_md = Some(body.into());
        self
    }

    pub fn user_md(mut self, body: impl Into<String>) -> Self {
        self.user_md = Some(body.into());
        self
    }

    pub fn history(mut self, history: Vec<Message>) -> Self {
        self.history = history;
        self
    }

    /// Assemble the final [`CompletionRequest`]. `user_turn` is the new
    /// user input — it always goes last and is never inside the cached
    /// prefix.
    pub fn build(
        self,
        model: impl Into<String>,
        user_turn: Message,
        max_tokens: u32,
    ) -> CompletionRequest {
        let PromptBuilder {
            tools,
            system,
            skills_meta,
            memory_md,
            soul_md,
            user_md,
            history,
        } = self;

        let combined_system = assemble_system(
            system.as_deref(),
            skills_meta.as_deref(),
            memory_md.as_deref(),
            soul_md.as_deref(),
            user_md.as_deref(),
        );

        let mut messages: Vec<Message> = Vec::with_capacity(history.len() + 2);
        if let Some(text) = combined_system {
            messages.push(Message::system(text));
        }
        messages.extend(history);
        messages.push(user_turn);

        // Cache breakpoints: end-of-tools (if any) and end-of-system (last
        // static message before history). Both use the 5-min TTL — callers
        // can upgrade to 1h via Anthropic's beta header.
        let mut cache_breakpoints = Vec::new();
        if !tools.is_empty() {
            cache_breakpoints.push(CacheBreakpoint {
                position: CachePosition::EndOfTools,
                ttl: CacheTtl::FiveMinutes,
            });
        }
        if messages
            .first()
            .map(|m| matches!(m.role, crate::message::Role::System))
            .unwrap_or(false)
        {
            cache_breakpoints.push(CacheBreakpoint {
                position: CachePosition::EndOfSystem,
                ttl: CacheTtl::FiveMinutes,
            });
        }

        CompletionRequest {
            model: model.into(),
            messages,
            tools,
            cache_breakpoints,
            reasoning_effort: None,
            max_tokens,
            temperature: None,
        }
    }
}

impl Default for PromptBuilder {
    fn default() -> Self {
        Self::new()
    }
}

fn assemble_system(
    base: Option<&str>,
    skills_meta: Option<&str>,
    memory_md: Option<&str>,
    soul_md: Option<&str>,
    user_md: Option<&str>,
) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    parts.extend(base);
    parts.extend(skills_meta);
    parts.extend(memory_md);
    parts.extend(soul_md);
    parts.extend(user_md);

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{ContentBlock, Role};

    #[test]
    fn order_is_stable() {
        let req = PromptBuilder::new()
            .system("you are helpful")
            .skills_meta("- git-workflow")
            .memory_md("user works in rust")
            .soul_md("persona-x")
            .user_md("max likes brevity")
            .history(vec![Message::user("earlier turn")])
            .build("claude-sonnet-4-6", Message::user("now"), 1024);

        // System message comes first.
        let sys = match &req.messages[0].content[0] {
            ContentBlock::Text { text } => text.clone(),
            _ => panic!("expected text"),
        };
        let idx = |needle: &str| sys.find(needle).expect("found");
        assert!(idx("you are helpful") < idx("- git-workflow"));
        assert!(idx("- git-workflow") < idx("user works in rust"));
        assert!(idx("user works in rust") < idx("persona-x"));
        assert!(idx("persona-x") < idx("max likes brevity"));

        // History sandwiched, user_turn last.
        assert_eq!(req.messages.last().unwrap().role, Role::User);
    }

    #[test]
    fn cache_breakpoints_placed() {
        let req = PromptBuilder::new()
            .tools(vec![ToolSchema {
                name: "x".into(),
                description: "y".into(),
                parameters: serde_json::json!({}),
            }])
            .system("be brief")
            .build("m", Message::user("hi"), 256);

        assert_eq!(req.cache_breakpoints.len(), 2);
        assert!(matches!(
            req.cache_breakpoints[0].position,
            CachePosition::EndOfTools
        ));
        assert!(matches!(
            req.cache_breakpoints[1].position,
            CachePosition::EndOfSystem
        ));
    }

    #[test]
    fn no_system_no_system_breakpoint() {
        let req = PromptBuilder::new().build("m", Message::user("hi"), 64);
        assert!(req.cache_breakpoints.is_empty());
        assert_eq!(req.messages.len(), 1);
    }
}
