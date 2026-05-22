//! In-memory A2A task store with lifecycle transitions.
//!
//! MVP: tasks live for the process lifetime in a `Mutex<HashMap>`. (Persisting
//! to SQLite for cross-restart `tasks/get` is a later refinement.)

use std::collections::HashMap;
use std::sync::Mutex;

use crate::types::{Artifact, Message, Task, TaskState, TaskStatus};

#[derive(Default)]
pub struct TaskStore {
    tasks: Mutex<HashMap<String, Task>>,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

impl TaskStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new task in `submitted` state.
    pub fn create(&self, id: &str, context_id: &str) -> Task {
        let task = Task {
            id: id.to_string(),
            context_id: context_id.to_string(),
            status: TaskStatus::new(TaskState::Submitted),
            artifacts: Vec::new(),
            history: Vec::new(),
            kind: "task".to_string(),
        };
        self.tasks
            .lock()
            .unwrap()
            .insert(id.to_string(), task.clone());
        task
    }

    pub fn set_state(&self, id: &str, state: TaskState) {
        if let Some(t) = self.tasks.lock().unwrap().get_mut(id) {
            t.status = TaskStatus::new(state);
        }
    }

    pub fn complete(&self, id: &str, artifact: Artifact, reply: Message) {
        if let Some(t) = self.tasks.lock().unwrap().get_mut(id) {
            t.artifacts.push(artifact);
            t.status = TaskStatus {
                state: TaskState::Completed,
                message: Some(reply),
                timestamp: Some(now()),
            };
        }
    }

    pub fn fail(&self, id: &str, error: &str) {
        if let Some(t) = self.tasks.lock().unwrap().get_mut(id) {
            t.status = TaskStatus {
                state: TaskState::Failed,
                message: Some(Message::agent_text(error)),
                timestamp: Some(now()),
            };
        }
    }

    /// Mark a task canceled; returns the updated task, or `None` if unknown.
    pub fn cancel(&self, id: &str) -> Option<Task> {
        let mut g = self.tasks.lock().unwrap();
        let t = g.get_mut(id)?;
        t.status = TaskStatus::new(TaskState::Canceled);
        Some(t.clone())
    }

    pub fn get(&self, id: &str) -> Option<Task> {
        self.tasks.lock().unwrap().get(id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_submitted_to_completed() {
        let store = TaskStore::new();
        store.create("t1", "c1");
        assert_eq!(store.get("t1").unwrap().status.state, TaskState::Submitted);
        store.set_state("t1", TaskState::Working);
        assert_eq!(store.get("t1").unwrap().status.state, TaskState::Working);
        store.complete("t1", Artifact::text("response", "hi"), Message::agent_text("hi"));
        let t = store.get("t1").unwrap();
        assert_eq!(t.status.state, TaskState::Completed);
        assert_eq!(t.artifacts.len(), 1);
    }

    #[test]
    fn cancel_unknown_is_none() {
        let store = TaskStore::new();
        assert!(store.cancel("nope").is_none());
    }
}
