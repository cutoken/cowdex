//! Session concurrency selection from the slash-command UI.

use super::*;

impl ChatWidget {
    pub(super) fn dispatch_agent_limit_command(&mut self, args: &str) {
        if args.trim().is_empty() {
            let limit = self
                .config
                .multi_agent_v2
                .max_concurrent_threads_per_session;
            self.add_info_message(
                format!("Agent limit: {limit} concurrent agents including the main agent. Use /limit_agents N to change it for this session."),
                /*hint*/ None,
            );
        } else {
            match args.trim().parse::<usize>() {
                Ok(limit) if limit > 0 => self.app_event_tx.send(AppEvent::SetAgentLimit(limit)),
                _ => self.add_error_message(
                    "Use /limit_agents N with a positive integer (including the main agent)."
                        .to_string(),
                ),
            }
        }
    }

    pub(crate) fn set_agent_limit(&mut self, agent_limit: usize) {
        self.config
            .multi_agent_v2
            .max_concurrent_threads_per_session = agent_limit;
        self.config.agent_max_threads = Some(agent_limit.saturating_sub(1));
    }
}
