use std::collections::HashMap;
use std::sync::Arc;

use ravn_llm::ToolSchema;

use crate::tool::Tool;

/// Holds the agent's registered tools. Lookup is O(1) by name.
#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: HashMap<&'static str, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register<T: Tool + 'static>(&mut self, tool: T) -> &mut Self {
        self.tools.insert(tool.name(), Arc::new(tool));
        self
    }

    pub fn register_arc(&mut self, tool: Arc<dyn Tool>) -> &mut Self {
        self.tools.insert(tool.name(), tool);
        self
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.tools.keys().copied()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Render the registry as `ToolSchema` values ready for inclusion in a
    /// [`ravn_llm::CompletionRequest`].
    pub fn as_schemas(&self) -> Vec<ToolSchema> {
        let mut out: Vec<ToolSchema> = self
            .tools
            .values()
            .map(|t| ToolSchema {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.schema(),
            })
            .collect();
        // Stable order across runs keeps the prompt cache-friendly.
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }
}
