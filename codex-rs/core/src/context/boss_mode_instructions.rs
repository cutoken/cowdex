use super::ContextualUserFragment;
use codex_protocol::protocol::BossModeSettings;

pub(crate) const BOSS_MAIN_AGENT_RESPONSIBILITIES: &str = "As the boss main agent, you remain accountable for creative direction, product and interaction design, architecture, scope, and preventing overengineering. Own user intent, technical decisions, task decomposition, coordination, and acceptance of the integrated result. Aggressively delegate substantial execution work, including exploration, implementation, validation, and log analysis. Every delegation must state a bounded objective, explicit ownership, what is out of scope, sufficient validation, and a stopping condition. Assess concise worker plans and results against those constraints, and redirect unnecessary abstractions, adjacent cleanup, excessive tests, or duplicate validation before they sprawl. Do not add monitoring, polling, or review gates solely to supervise workers; rely on worker messages and long interruptible waits matched to the expected runtime. Do not take over implementation, repeat investigations or tests, read whole diffs, or perform line-by-line review yourself. Routine Git operations and small coordination operations are fine. Limit your own checks to a few targeted spot-checks justified by a concrete concern or an architectural decision; delegate deeper checking. Delegate the next useful task as soon as its dependencies are ready. This division of responsibility applies only to you as the boss main agent; workers should execute their assigned work.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BossModeInstructions {
    enabled: bool,
    model: String,
    reasoning_effort: String,
    responsibilities: &'static str,
}

impl BossModeInstructions {
    pub(crate) fn new(settings: &BossModeSettings, responsibilities: &'static str) -> Self {
        Self {
            enabled: settings.enabled,
            model: settings.model.chars().take(128).collect(),
            reasoning_effort: settings
                .reasoning_effort
                .to_string()
                .chars()
                .take(32)
                .collect(),
            responsibilities,
        }
    }
}

impl ContextualUserFragment for BossModeInstructions {
    fn role(&self) -> &'static str {
        "developer"
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        ("<boss_mode>", "</boss_mode>")
    }

    fn body(&self) -> String {
        if !self.enabled {
            return "Boss mode is off. Earlier boss-mode instructions requiring delegation, prohibiting substantial local execution, or fixing worker model and reasoning effort no longer apply. Follow the current ordinary multi-agent policy. This change applies to future spawns; existing workers keep their settings.".to_string();
        }
        let model = &self.model;
        let reasoning_effort = &self.reasoning_effort;
        let mut text = format!(
            "Boss mode is on. Future workers are mechanically assigned model {model} with {reasoning_effort} reasoning effort. This selection supersedes earlier worker model and effort instructions. Existing workers keep their settings."
        );
        if !self.responsibilities.is_empty() {
            text.push_str("\n\n");
            text.push_str(self.responsibilities);
        }
        text
    }
}
